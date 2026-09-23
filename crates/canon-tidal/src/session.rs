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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use canon_core::{
    Account, DeviceCode, Error, LoginStatus, Quality, ResolvedStream, Result, Service,
    ServiceSession,
};
use serde::Deserialize;

use crate::auth::{
    self, API_BASE, CLIENT_ID_PKCE, CLIENT_SECRET_PKCE, DEVICE_CLIENT_ID, DEVICE_CLIENT_SECRET,
    DeviceAuthorization, PkceChallenge, PollOutcome, Tokens,
};
use crate::http::TidalHttp;
use crate::segment::{Chunk, SegmentFetch, SegmentReader};
use crate::store::{PersistedTokens, TokenStore};
use crate::stream::{self, ResolvedTidalStream};

/// How many times a single segment may be re-resolved on expiry before giving up — a
/// ceiling so a genuinely dead URL can't loop forever.
const MAX_RERESOLVE: u32 = 3;
/// Segment read-ahead depth (bounded channel capacity): how many segments may be
/// fetched before the decoder consumes them.
const READ_AHEAD_SEGMENTS: usize = 3;

/// Refresh this long before the access token actually expires, so a call never races
/// the boundary and gets a 401.
const REFRESH_SKEW: Duration = Duration::from_secs(60);

/// The OAuth scope tideway requests for the device-code client.
const DEFAULT_SCOPE: &str = "r_usr w_usr w_sub";

/// A live Tidal connection. Cheap to share (`Arc`); all mutable state is internal.
pub struct TidalSession {
    http: Arc<dyn TidalHttp>,
    store: TokenStore,
    /// Where an in-flight PKCE challenge is parked so the flow can span two invocations.
    pending_path: PathBuf,
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
    /// The account country code, cached after the first `account()` call. Tidal
    /// requires it on stream-resolution requests.
    country_code: Option<String>,
    /// The Tidal session id, cached after the first `account()` call. Playback endpoints
    /// require it as a query param (its absence is half the 4005 gate).
    session_id: Option<String>,
    /// Whether the held tokens were minted by the PKCE (streaming) client. Selects the
    /// client credentials used on refresh.
    is_pkce: bool,
    /// The in-flight PKCE challenge material between `pkce_login_url` and completion.
    pending_pkce: Option<PkceChallenge>,
}

impl TidalSession {
    /// Build a session and restore any persisted tokens from `store`. A corrupt store
    /// surfaces as an error (re-login required) rather than a silent unauthenticated
    /// start.
    pub async fn restore(http: Arc<dyn TidalHttp>, store: TokenStore) -> Result<Self> {
        let mut inner = Inner::default();
        if let Some(persisted) = store.load().await? {
            inner.expires_at = Some(UNIX_EPOCH + Duration::from_secs(persisted.expires_at_unix));
            inner.is_pkce = persisted.is_pkce;
            inner.tokens = Some(Tokens {
                access_token: persisted.access_token,
                refresh_token: persisted.refresh_token,
                expires_in: 0,
                user_id: persisted.user_id,
            });
        }
        let pending_path = store.path().with_file_name("tidal_pkce_pending.json");
        Ok(Self {
            http,
            store,
            pending_path,
            scope: DEFAULT_SCOPE.to_string(),
            inner: tokio::sync::Mutex::new(inner),
        })
    }

    /// Adopt a freshly issued token pair: record its absolute expiry, which client
    /// minted it, and persist it. Assumes the caller holds `inner`.
    async fn adopt(
        &self,
        inner: &mut Inner,
        tokens: Tokens,
        expires_in: u64,
        is_pkce: bool,
    ) -> Result<()> {
        let expires_at = SystemTime::now() + Duration::from_secs(expires_in);
        inner.expires_at = Some(expires_at);
        inner.is_pkce = is_pkce;
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
                is_pkce: inner.is_pkce,
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
        // Device-code and PKCE sessions refresh with different clients.
        let (client_id, client_secret) = if inner.is_pkce {
            (CLIENT_ID_PKCE, CLIENT_SECRET_PKCE)
        } else {
            (DEVICE_CLIENT_ID, DEVICE_CLIENT_SECRET)
        };
        let resp = auth::refresh(&*self.http, &refresh, client_id, client_secret).await?;
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
                self.adopt(&mut inner, tokens, resp.expires_in, false)
                    .await?;
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
        // Cache the country code + session id: stream resolution needs both on every
        // playback request.
        {
            let mut inner = self.inner.lock().await;
            inner.country_code = Some(info.country_code.clone());
            inner.session_id = Some(info.session_id.clone());
        }
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

impl TidalSession {
    /// Begin PKCE login: generate challenge material, stash it, and return the browser
    /// login URL. The user opens it, logs in, and pastes the redirect URL into
    /// [`complete_pkce_login`](Self::complete_pkce_login).
    pub async fn pkce_login_url(&self) -> Result<String> {
        let challenge = PkceChallenge::generate()?;
        let url = auth::pkce_login_url(&challenge.challenge, &challenge.unique_key);
        // Persist the challenge so a second `canon` invocation can complete the flow.
        if let Some(parent) = self.pending_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec(&challenge)
            .map_err(|e| Error::Auth(format!("pkce challenge serialize: {e}")))?;
        tokio::fs::write(&self.pending_path, bytes).await?;
        self.inner.lock().await.pending_pkce = Some(challenge);
        Ok(url)
    }

    /// Complete PKCE login from the pasted redirect URL: extract the `code`, exchange it
    /// (proving the flow with the stashed verifier), and adopt the streaming-capable
    /// tokens — replacing any device-code tokens.
    pub async fn complete_pkce_login(&self, redirect_url: &str) -> Result<()> {
        let challenge = match self.inner.lock().await.pending_pkce.clone() {
            Some(challenge) => challenge,
            // A fresh process (the two-step flow): load the parked challenge from disk.
            None => {
                let bytes = tokio::fs::read(&self.pending_path)
                    .await
                    .map_err(|_| Error::Auth("no PKCE login in flight".into()))?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| Error::Auth(format!("pkce challenge decode: {e}")))?
            }
        };
        let code = auth::extract_pkce_code(redirect_url)?;
        let resp = auth::exchange_pkce_code(
            &*self.http,
            &code,
            &challenge.verifier,
            &challenge.unique_key,
        )
        .await?;
        let tokens = Tokens::from_initial(&resp)?;
        let mut inner = self.inner.lock().await;
        self.adopt(&mut inner, tokens, resp.expires_in, true)
            .await?;
        inner.pending_pkce = None;
        // A new identity: drop the cached session context so it's re-fetched.
        inner.session_id = None;
        inner.country_code = None;
        drop(inner);
        let _ = tokio::fs::remove_file(&self.pending_path).await;
        Ok(())
    }

    /// A valid bearer token, refreshing first if it's near expiry.
    async fn bearer(&self) -> Result<String> {
        let mut inner = self.inner.lock().await;
        self.ensure_fresh(&mut inner).await?;
        inner
            .tokens
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| Error::Auth("not authenticated".into()))
    }

    /// The (country code, session id) pair playback requests need, fetching them via
    /// `account()` (which calls `/v1/sessions`) and caching if not already known.
    async fn session_context(&self) -> Result<(String, String)> {
        {
            let inner = self.inner.lock().await;
            if let (Some(country), Some(session)) = (&inner.country_code, &inner.session_id) {
                return Ok((country.clone(), session.clone()));
            }
        }
        self.account().await?; // populates both
        let inner = self.inner.lock().await;
        match (&inner.country_code, &inner.session_id) {
            (Some(country), Some(session)) => Ok((country.clone(), session.clone())),
            _ => Err(Error::Source(
                "tidal did not report a session context".into(),
            )),
        }
    }

    /// Resolve a Tidal track id to its playable stream (URL list + physical description)
    /// without fetching the audio bytes.
    pub async fn resolve_stream(
        &self,
        track_id: &str,
        quality: Quality,
    ) -> Result<ResolvedTidalStream> {
        let bearer = self.bearer().await?;
        let (country, session_id) = self.session_context().await?;
        stream::resolve(
            &*self.http,
            &bearer,
            track_id,
            quality,
            &country,
            &session_id,
        )
        .await
    }

    /// Open a streaming playable input starting at (the segment covering) `position`.
    /// The stream's `start_ms` is that segment's start, which the engine reports so the
    /// clock is positioned correctly. Segments are fetched lazily with read-ahead and
    /// transparent expiry re-resolution (canon-e99d).
    pub async fn open_stream_at(
        self: Arc<Self>,
        track_id: &str,
        quality: Quality,
        position: Duration,
    ) -> Result<ResolvedStream> {
        let resolved = self.resolve_stream(track_id, quality).await?;
        let info = resolved.info.clone();
        let (start_index, start_secs) = resolved.segment_for_secs(position.as_secs_f64());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let start_ms = (start_secs * 1000.0) as u64;

        let (tx, rx) = tokio::sync::mpsc::channel::<Chunk>(READ_AHEAD_SEGMENTS);
        let session = Arc::clone(&self);
        let track_id = track_id.to_string();
        tokio::spawn(run_producer(
            session,
            track_id,
            quality,
            resolved,
            start_index,
            tx,
        ));

        Ok(ResolvedStream {
            input: Box::new(SegmentReader::new(rx)),
            info,
            start_ms,
        })
    }

    /// Fetch one segment URL, classifying an expired (403) URL for re-resolution.
    async fn fetch_segment(&self, url: &str) -> SegmentFetch {
        match self.http.get(url, &[]).await {
            Ok(resp) if resp.status == 403 => SegmentFetch::Expired,
            Ok(resp) if resp.is_success() => SegmentFetch::Data(resp.body),
            Ok(resp) => SegmentFetch::Failed(Error::Source(format!(
                "segment fetch failed: HTTP {}",
                resp.status
            ))),
            Err(e) => SegmentFetch::Failed(e),
        }
    }
}

/// The segment producer: fetch the init segment (if any), then media segments from
/// `start_index`, feeding the reader's channel with read-ahead backpressure. An expired
/// (403) URL re-resolves the manifest and resumes at the same index (the tide-1100 fix),
/// bounded so a dead URL can't loop forever.
async fn run_producer(
    session: Arc<TidalSession>,
    track_id: String,
    quality: Quality,
    mut resolved: ResolvedTidalStream,
    start_index: usize,
    tx: tokio::sync::mpsc::Sender<Chunk>,
) {
    // Init segment first (decoder config); re-fetch its fresh URL if it expires.
    if resolved.init_url.is_some() {
        let mut tries = 0u32;
        while let Some(init) = resolved.init_url.clone() {
            match session.fetch_segment(&init).await {
                SegmentFetch::Data(bytes) => {
                    if tx.send(Chunk::Data(bytes)).await.is_err() {
                        return;
                    }
                    break;
                }
                SegmentFetch::Expired => {
                    tries += 1;
                    if tries > MAX_RERESOLVE {
                        let _ = tx
                            .send(Chunk::Err("init segment kept expiring".into()))
                            .await;
                        return;
                    }
                    match session.resolve_stream(&track_id, quality).await {
                        Ok(fresh) => resolved = fresh,
                        Err(e) => {
                            let _ = tx.send(Chunk::Err(e.to_string())).await;
                            return;
                        }
                    }
                }
                SegmentFetch::Failed(e) => {
                    let _ = tx.send(Chunk::Err(e.to_string())).await;
                    return;
                }
            }
        }
    }

    // Then media segments from start_index onward. (init loop above `break`s on success)
    let mut idx = start_index;
    let mut expiries = 0u32;
    while idx < resolved.media_urls.len() {
        match session.fetch_segment(&resolved.media_urls[idx]).await {
            SegmentFetch::Data(bytes) => {
                expiries = 0;
                if tx.send(Chunk::Data(bytes)).await.is_err() {
                    return; // reader dropped (stop / track change / seek)
                }
                idx += 1;
            }
            SegmentFetch::Expired => {
                expiries += 1;
                if expiries > MAX_RERESOLVE {
                    let _ = tx
                        .send(Chunk::Err("segment URL kept expiring".into()))
                        .await;
                    return;
                }
                // Re-resolve and resume at the SAME index (the tide-1100 fix).
                match session.resolve_stream(&track_id, quality).await {
                    Ok(fresh) => resolved = fresh,
                    Err(e) => {
                        let _ = tx.send(Chunk::Err(e.to_string())).await;
                        return;
                    }
                }
            }
            SegmentFetch::Failed(e) => {
                let _ = tx.send(Chunk::Err(e.to_string())).await;
                return;
            }
        }
    }
    // Loop end drops `tx`, closing the channel = EOF to the reader.
}

impl TidalSession {
    /// An authenticated `GET` of an API path, with the account's country code and `query` added,
    /// decoded as `T`. Catalog calls all go through here.
    pub(crate) async fn api_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let bearer = self.bearer().await?;
        let (country, _session_id) = self.session_context().await?;
        let mut url = format!("{API_BASE}{path}?countryCode={country}");
        for (key, value) in query {
            url.push('&');
            url.push_str(key);
            url.push('=');
            url.push_str(&auth::encode_component(value));
        }
        let authorization = format!("Bearer {bearer}");
        let resp = self
            .http
            .get(&url, &[("Authorization", &authorization)])
            .await?;
        match resp.status {
            404 => Err(Error::NotFound(format!("tidal {path}"))),
            _ if !resp.is_success() => {
                Err(Error::Source(format!("tidal {path}: HTTP {}", resp.status)))
            }
            _ => resp.json(),
        }
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
                is_pkce: false,
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
