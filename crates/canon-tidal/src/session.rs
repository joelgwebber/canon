//! The live Tidal session: device-code login, token lifecycle, and the first
//! authenticated call (yak canon-8bab).
//!
//! This is where the compile-only skeleton in [`crate::auth`] becomes a real,
//! stateful login. [`TidalSession`] implements [`canon_core::ServiceSession`], so the
//! daemon holds it as `Arc<dyn ServiceSession>` and the control API drives it without
//! ever naming Tidal.
//!
//! What it owns, and why:
//! * **The token pair + absolute expiry**, behind a [`tokio::sync::Mutex`]. Every
//!   refresh folds through [`Tokens::apply`], which captures a *rotated* refresh token —
//!   the single choke point against silent logout.
//! * **Single-flight refresh.** [`TidalSession::account`] refreshes under the same lock
//!   it reads the token from, so concurrent callers serialize: the first refreshes, the
//!   rest wake to a fresh token and skip the network. No thundering herd of refreshes.
//! * **Persistence.** Every adopted/refreshed token is written through the
//!   [`TokenStore`] so a restart resumes the session instead of forcing re-login.
//!
//! The authenticated call is `GET /v1/sessions` — the same call tidalapi uses to
//! validate a session. It returns the numeric user id and country code, which doubles
//! as the end-to-end proof that the held access token actually works against Tidal.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use canon_core::{Account, DeviceCode, Error, LoginStatus, Result, Service, ServiceSession};
use serde::Deserialize;

use crate::auth::{self, API_BASE, DeviceAuthorization, PollOutcome, Tokens};
use crate::http::TidalHttp;
use crate::store::{PersistedTokens, TokenStore};

/// Refresh this long before the access token actually expires, so a call never races
/// the boundary and gets a 401.
const REFRESH_SKEW: Duration = Duration::from_secs(60);

/// The OAuth scope tideway requests for the device-code client.
const DEFAULT_SCOPE: &str = "r_usr w_usr w_sub";

/// A live Tidal connection. Cheap to share (`Arc`); all mutable state is internal.
pub struct TidalSession {
    http: Arc<dyn TidalHttp>,
    store: TokenStore,
    scope: String,
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The held token pair, if authenticated.
    tokens: Option<Tokens>,
    /// Absolute access-token expiry.
    expires_at: Option<SystemTime>,
    /// The in-flight device authorization between `begin_login` and success.
    pending: Option<DeviceAuthorization>,
}

impl TidalSession {
    /// Build a session and restore any persisted tokens from `store`. A corrupt store
    /// surfaces as an error (re-login required) rather than a silent unauthenticated
    /// start.
    pub async fn restore(http: Arc<dyn TidalHttp>, store: TokenStore) -> Result<Self> {
        let mut inner = Inner::default();
        if let Some(persisted) = store.load().await? {
            inner.expires_at = Some(UNIX_EPOCH + Duration::from_secs(persisted.expires_at_unix));
            inner.tokens = Some(Tokens {
                access_token: persisted.access_token,
                refresh_token: persisted.refresh_token,
                expires_in: 0,
                user_id: persisted.user_id,
            });
        }
        Ok(Self {
            http,
            store,
            scope: DEFAULT_SCOPE.to_string(),
            inner: tokio::sync::Mutex::new(inner),
        })
    }

    /// Adopt a freshly issued token pair: record its absolute expiry and persist it.
    /// Assumes the caller holds `inner`.
    async fn adopt(&self, inner: &mut Inner, tokens: Tokens, expires_in: u64) -> Result<()> {
        let expires_at = SystemTime::now() + Duration::from_secs(expires_in);
        inner.expires_at = Some(expires_at);
        inner.tokens = Some(tokens);
        self.persist(inner).await
    }

    /// Write the currently held tokens to the store. Assumes the caller holds `inner`.
    async fn persist(&self, inner: &Inner) -> Result<()> {
        let (Some(tokens), Some(expires_at)) = (inner.tokens.as_ref(), inner.expires_at) else {
            return Ok(());
        };
        let expires_at_unix = expires_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.store
            .save(&PersistedTokens {
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at_unix,
                user_id: tokens.user_id,
            })
            .await
    }

    /// Ensure the access token is valid for at least [`REFRESH_SKEW`], refreshing it in
    /// place if not. Runs under `inner`, which is what makes refresh single-flight:
    /// concurrent callers queue on the lock and the later ones find a fresh token.
    async fn ensure_fresh(&self, inner: &mut Inner) -> Result<()> {
        let stale = match (inner.tokens.as_ref(), inner.expires_at) {
            (None, _) => return Err(Error::Auth("not authenticated".into())),
            (Some(_), Some(expires_at)) => expires_at <= SystemTime::now() + REFRESH_SKEW,
            // No known expiry (e.g. just restored from disk): treat as stale and prove
            // the token by refreshing before the first call.
            (Some(_), None) => true,
        };
        if !stale {
            return Ok(());
        }

        let refresh = inner
            .tokens
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .expect("tokens present per match above");
        let resp = auth::refresh_token(&*self.http, &refresh).await?;
        let expires_in = resp.expires_in;
        if let Some(tokens) = inner.tokens.as_mut() {
            tokens.apply(&resp); // captures a rotated refresh token
        }
        inner.expires_at = Some(SystemTime::now() + Duration::from_secs(expires_in));
        self.persist(inner).await
    }
}

#[async_trait]
impl ServiceSession for TidalSession {
    fn service(&self) -> Service {
        Service::Tidal
    }

    fn is_authenticated(&self) -> bool {
        // A blocking try_lock is fine: this is a cheap, non-async predicate and the
        // lock is only ever held across short critical sections.
        self.inner
            .try_lock()
            .map(|inner| inner.tokens.is_some())
            .unwrap_or(true) // locked => a login/refresh is in flight => treat as active
    }

    async fn begin_login(&self) -> Result<DeviceCode> {
        let authorization = auth::start_device_authorization(&*self.http, &self.scope).await?;
        let code = DeviceCode {
            user_code: authorization.user_code.clone(),
            verification_uri: authorization.verification_uri.clone(),
            verification_uri_complete: authorization.verification_uri_complete.clone(),
            expires_in: authorization.expires_in,
            interval: authorization.interval,
        };
        self.inner.lock().await.pending = Some(authorization);
        Ok(code)
    }

    async fn poll_login(&self) -> Result<LoginStatus> {
        let device_code = {
            let inner = self.inner.lock().await;
            inner
                .pending
                .as_ref()
                .map(|p| p.device_code.clone())
                .ok_or_else(|| Error::Auth("no device login in flight".into()))?
        };

        match auth::poll_device_token(&*self.http, &device_code, &self.scope).await? {
            PollOutcome::Pending => Ok(LoginStatus::Pending),
            PollOutcome::SlowDown => Ok(LoginStatus::SlowDown),
            PollOutcome::Authorized(resp) => {
                let tokens = Tokens::from_initial(&resp)?;
                let mut inner = self.inner.lock().await;
                self.adopt(&mut inner, tokens, resp.expires_in).await?;
                inner.pending = None;
                Ok(LoginStatus::Authorized)
            }
        }
    }

    async fn account(&self) -> Result<Account> {
        let mut inner = self.inner.lock().await;
        self.ensure_fresh(&mut inner).await?;
        let access = inner
            .tokens
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| Error::Auth("not authenticated".into()))?;
        // Drop the lock before the (independent) authed GET so it doesn't serialize the
        // whole session for the round trip; freshness is already guaranteed above.
        drop(inner);

        let url = format!("{API_BASE}/v1/sessions");
        let bearer = format!("Bearer {access}");
        let resp = self.http.get(&url, &[("Authorization", &bearer)]).await?;

        if resp.status == 401 {
            return Err(Error::Auth("access token rejected by Tidal (401)".into()));
        }
        if !resp.is_success() {
            return Err(Error::Source(format!(
                "GET /v1/sessions failed: HTTP {} {}",
                resp.status,
                resp.text().unwrap_or_default()
            )));
        }

        let info: SessionInfo = resp.json()?;
        let mut attributes = std::collections::BTreeMap::new();
        attributes.insert("country_code".to_string(), info.country_code);
        attributes.insert("session_id".to_string(), info.session_id);
        Ok(Account {
            service: Service::Tidal,
            user_id: info.user_id.to_string(),
            username: None,
            attributes,
        })
    }
}

/// The `GET /v1/sessions` response — Tidal's session/identity probe.
#[derive(Debug, Deserialize)]
struct SessionInfo {
    #[serde(rename = "userId")]
    user_id: i64,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "countryCode")]
    country_code: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpResponse;
    use std::sync::Mutex;

    /// A scripted [`TidalHttp`] that replays canned responses in order, so the session
    /// logic is tested without touching the network.
    struct MockHttp {
        gets: Mutex<Vec<HttpResponse>>,
        posts: Mutex<Vec<HttpResponse>>,
    }

    impl MockHttp {
        fn new() -> Self {
            Self {
                gets: Mutex::new(Vec::new()),
                posts: Mutex::new(Vec::new()),
            }
        }
        fn push_get(&self, status: u16, body: &str) {
            self.gets.lock().unwrap().push(HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            });
        }
        fn push_post(&self, status: u16, body: &str) {
            self.posts.lock().unwrap().push(HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            });
        }
    }

    #[async_trait]
    impl TidalHttp for MockHttp {
        async fn get(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<HttpResponse> {
            Ok(self.gets.lock().unwrap().remove(0))
        }
        async fn post_form(
            &self,
            _url: &str,
            _form: &[(&str, &str)],
            _headers: &[(&str, &str)],
        ) -> Result<HttpResponse> {
            Ok(self.posts.lock().unwrap().remove(0))
        }
    }

    fn store_at(name: &str) -> (TokenStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "canon-session-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (TokenStore::new(dir.join("tidal.json")), dir)
    }

    #[tokio::test]
    async fn login_then_account_persists_and_authenticates() {
        let http = Arc::new(MockHttp::new());
        // begin_login POST → device authorization
        http.push_post(
            200,
            r#"{"deviceCode":"dev","userCode":"AB-CD","verificationUri":"link.tidal.com","verificationUriComplete":"link.tidal.com/AB-CD","expiresIn":300,"interval":2}"#,
        );
        // poll_login POST → still pending, then authorized
        http.push_post(400, r#"{"error":"authorization_pending"}"#);
        http.push_post(
            200,
            r#"{"access_token":"at","refresh_token":"rt","expires_in":86400,"user_id":42}"#,
        );
        // account GET → /v1/sessions
        http.push_get(
            200,
            r#"{"userId":42,"sessionId":"sess-1","countryCode":"US"}"#,
        );

        let (store, dir) = store_at("happy");
        let session = TidalSession::restore(http.clone(), store.clone())
            .await
            .unwrap();
        assert!(!session.is_authenticated());

        let code = session.begin_login().await.unwrap();
        assert_eq!(code.user_code, "AB-CD");
        assert_eq!(session.poll_login().await.unwrap(), LoginStatus::Pending);
        assert_eq!(session.poll_login().await.unwrap(), LoginStatus::Authorized);
        assert!(session.is_authenticated());

        let account = session.account().await.unwrap();
        assert_eq!(account.service, Service::Tidal);
        assert_eq!(account.user_id, "42");
        assert_eq!(account.attributes.get("country_code").unwrap(), "US");

        // A fresh session restored from the same store is authenticated without login.
        let restored = TidalSession::restore(http.clone(), store).await.unwrap();
        assert!(restored.is_authenticated());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn restore_refreshes_a_token_with_unknown_expiry() {
        // A restored token has no known expiry, so the first authed call must refresh
        // first (proving the token) and capture any rotated refresh token.
        let http = Arc::new(MockHttp::new());
        http.push_post(
            200,
            r#"{"access_token":"at2","refresh_token":"rt2","expires_in":86400,"user_id":42}"#,
        );
        http.push_get(
            200,
            r#"{"userId":42,"sessionId":"sess-2","countryCode":"GB"}"#,
        );

        let (store, dir) = store_at("refresh");
        store
            .save(&PersistedTokens {
                access_token: "at1".into(),
                refresh_token: "rt1".into(),
                expires_at_unix: 1,
                user_id: Some(42),
            })
            .await
            .unwrap();

        let session = TidalSession::restore(http, store.clone()).await.unwrap();
        let account = session.account().await.unwrap();
        assert_eq!(account.attributes.get("country_code").unwrap(), "GB");

        // The rotated refresh token was captured and persisted.
        let persisted = store.load().await.unwrap().unwrap();
        assert_eq!(persisted.refresh_token, "rt2");
        assert_eq!(persisted.access_token, "at2");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
