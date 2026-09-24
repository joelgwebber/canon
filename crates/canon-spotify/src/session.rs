//! A Spotify login: the tokens, their refresh, and authenticated calls to the Web API.
//!
//! * **Tokens and expiry** live behind one [`tokio::sync::Mutex`]. [`SpotifySession::bearer`]
//!   refreshes under that lock, so concurrent callers queue and the later ones find a fresh token:
//!   one refresh, not a herd of them.
//! * **Rotation.** A refresh reply may or may not carry a new refresh token; the old one is kept
//!   when it doesn't.
//! * **Persistence.** Every token change goes through the [`TokenStore`] before it is used.
//! * **429.** Spotify's rate limit answers `429` with `Retry-After`; calls wait it out a bounded
//!   number of times, then give up with [`Error::Transient`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canon_core::{Account, Error, Result, Service};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::{self, DEFAULT_REDIRECT_URI, PendingLogin, TokenResponse};
use crate::http::SpotifyHttp;
use crate::store::{PersistedTokens, TokenStore};

pub const API_BASE: &str = "https://api.spotify.com/v1";

/// Refresh this long before expiry, so a call never races the boundary into a 401.
const REFRESH_SKEW: Duration = Duration::from_secs(60);
/// How many 429s one call waits out before giving up.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
/// The longest single `Retry-After` worth waiting for inside a call. Spotify can ask for hours
/// after sustained abuse; a catalog call shouldn't hang that long.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// When a 429 carries no `Retry-After`.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: SystemTime,
    scope: Option<String>,
}

#[derive(Default)]
struct Inner {
    tokens: Option<Tokens>,
    pending: Option<PendingLogin>,
    /// The signed-in user's id, once `/me` has said it.
    user_id: Option<String>,
}

/// A Spotify Web API login for one developer app (its client id). Share it with an `Arc`.
pub struct SpotifySession {
    http: Arc<dyn SpotifyHttp>,
    store: TokenStore,
    client_id: String,
    redirect_uri: String,
    inner: tokio::sync::Mutex<Inner>,
    authenticated: AtomicBool,
}

#[derive(Debug, Deserialize)]
struct Me {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

impl SpotifySession {
    /// A session for `client_id`, resuming the tokens in `store` if there are any. Tokens issued
    /// to a different client id are ignored (they can't be refreshed by this one).
    ///
    /// # Errors
    /// A token file that doesn't parse: log in again rather than run silently signed out.
    pub async fn restore(
        http: Arc<dyn SpotifyHttp>,
        store: TokenStore,
        client_id: impl Into<String>,
    ) -> Result<Self> {
        let client_id = client_id.into();
        let mut inner = Inner::default();
        match store.load().await? {
            Some(saved) if saved.client_id == client_id => {
                inner.tokens = Some(Tokens {
                    access_token: saved.access_token,
                    refresh_token: saved.refresh_token,
                    expires_at: UNIX_EPOCH + Duration::from_secs(saved.expires_at_unix),
                    scope: saved.scope,
                });
            }
            Some(_) => tracing::warn!(
                path = %store.path().display(),
                "spotify tokens were issued to another client id; log in again"
            ),
            None => {}
        }
        let authenticated = AtomicBool::new(inner.tokens.is_some());
        Ok(Self {
            http,
            store,
            client_id,
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            inner: tokio::sync::Mutex::new(inner),
            authenticated,
        })
    }

    /// Use another redirect URI than [`DEFAULT_REDIRECT_URI`]. It must be registered on the app,
    /// character for character.
    #[must_use]
    pub fn with_redirect_uri(mut self, redirect_uri: impl Into<String>) -> Self {
        self.redirect_uri = redirect_uri.into();
        self
    }

    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Whether there are tokens to call with. It doesn't prove they still work: [`account`]
    /// does.
    ///
    /// [`account`]: Self::account
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    /// Start a login: the URL to open in a browser. The login is parked beside the token file, so
    /// [`complete_login`](Self::complete_login) may run in another process.
    pub async fn login_url(&self) -> Result<String> {
        let pending = PendingLogin::generate(&self.redirect_uri)?;
        self.store.save_pending(&pending).await?;
        let url = pending.authorize_url(&self.client_id);
        self.inner.lock().await.pending = Some(pending);
        Ok(url)
    }

    /// Finish a login with what the browser was redirected to: the whole URL, or just its code.
    pub async fn complete_login(&self, redirect_or_code: &str) -> Result<()> {
        let parked = self.inner.lock().await.pending.clone();
        let pending = match parked {
            Some(pending) => pending,
            None => self
                .store
                .load_pending()
                .await?
                .ok_or_else(|| Error::Auth("no spotify login in flight; start one".into()))?,
        };
        let code = auth::code_from(redirect_or_code, &pending.state)?;
        let reply = auth::exchange_code(&*self.http, &self.client_id, &code, &pending).await?;
        let refresh_token = reply
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Auth("spotify issued no refresh token".into()))?;
        let mut inner = self.inner.lock().await;
        let tokens = Tokens {
            access_token: reply.access_token,
            refresh_token,
            expires_at: SystemTime::now() + Duration::from_secs(reply.expires_in),
            scope: reply.scope,
        };
        self.persist(&tokens).await?;
        inner.tokens = Some(tokens);
        inner.pending = None;
        // A new login may be a different account.
        inner.user_id = None;
        self.authenticated.store(true, Ordering::Release);
        drop(inner);
        self.store.clear_pending().await;
        Ok(())
    }

    /// Who is signed in (`GET /me`). Also the proof that the tokens work.
    pub async fn account(&self) -> Result<Account> {
        let me: Me = self.get(&api_url("/me", &[])).await?;
        self.inner.lock().await.user_id = Some(me.id.clone());
        Ok(Account {
            service: Service::Spotify,
            user_id: me.id,
            username: me.display_name.filter(|name| !name.is_empty()),
            attributes: std::collections::BTreeMap::new(),
        })
    }

    /// The signed-in user's id, from the cache or `/me`.
    pub(crate) async fn user_id(&self) -> Result<String> {
        if let Some(id) = self.inner.lock().await.user_id.clone() {
            return Ok(id);
        }
        Ok(self.account().await?.user_id)
    }

    async fn persist(&self, tokens: &Tokens) -> Result<()> {
        self.store
            .save(&PersistedTokens {
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at_unix: tokens
                    .expires_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                scope: tokens.scope.clone(),
                client_id: self.client_id.clone(),
            })
            .await
    }

    /// A bearer token good for at least [`REFRESH_SKEW`]. When `rejected` is the token held, it is
    /// refreshed regardless of its expiry: Spotify said no to it.
    async fn bearer(&self, rejected: Option<&str>) -> Result<String> {
        let mut inner = self.inner.lock().await;
        let Some(tokens) = inner.tokens.as_ref() else {
            return Err(Error::Auth("not logged in to spotify".into()));
        };
        let stale = tokens.expires_at <= SystemTime::now() + REFRESH_SKEW
            || rejected == Some(tokens.access_token.as_str());
        if !stale {
            return Ok(tokens.access_token.clone());
        }
        let reply = auth::refresh(&*self.http, &self.client_id, &tokens.refresh_token).await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(Error::Auth(why)) => {
                // Revoked, or six months past consent: nothing works until a new login.
                inner.tokens = None;
                self.authenticated.store(false, Ordering::Release);
                return Err(Error::Auth(why));
            }
            Err(other) => return Err(other),
        };
        let refreshed = folded(tokens, reply);
        self.persist(&refreshed).await?;
        let access = refreshed.access_token.clone();
        inner.tokens = Some(refreshed);
        Ok(access)
    }

    /// An authenticated GET of a full Web API URL, decoded. Retries once on a 401 with a refreshed
    /// token, and waits out 429s up to [`MAX_RATE_LIMIT_RETRIES`] times.
    pub(crate) async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        if !url.starts_with(API_BASE) {
            // `next` links come from replies; never send the token anywhere else.
            return Err(Error::Source(format!("spotify: refusing to call {url}")));
        }
        let mut bearer = self.bearer(None).await?;
        let mut reauthorized = false;
        let mut rate_limited = 0;
        loop {
            let authorization = format!("Bearer {bearer}");
            let reply = self
                .http
                .get(url, &[("Authorization", &authorization)])
                .await?;
            match reply.status {
                _ if reply.is_success() => return reply.json(),
                401 if !reauthorized => {
                    reauthorized = true;
                    bearer = self.bearer(Some(&bearer)).await?;
                }
                401 => {
                    return Err(Error::Auth(format!(
                        "spotify rejected a fresh token for {url}"
                    )));
                }
                429 => {
                    let wait = reply.retry_after.unwrap_or(DEFAULT_RETRY_AFTER);
                    if rate_limited >= MAX_RATE_LIMIT_RETRIES || wait > MAX_RETRY_AFTER {
                        return Err(Error::Transient(format!(
                            "spotify rate limit on {url}: retry after {}s",
                            wait.as_secs()
                        )));
                    }
                    rate_limited += 1;
                    tracing::debug!(url, ?wait, attempt = rate_limited, "spotify 429");
                    tokio::time::sleep(wait).await;
                }
                // Development-mode apps get 403 for what they may not see (other people's
                // playlists' items), and for users not on the app's allowlist.
                403 => {
                    return Err(Error::Unsupported(format!(
                        "spotify forbids {url} to this app: {}",
                        reply.text()
                    )));
                }
                404 => return Err(Error::NotFound(format!("spotify {url}"))),
                status if status >= 500 => {
                    return Err(Error::Transient(format!("spotify {url}: HTTP {status}")));
                }
                status => {
                    return Err(Error::Source(format!(
                        "spotify {url}: HTTP {status} {}",
                        reply.text()
                    )));
                }
            }
        }
    }
}

/// A refresh folded into the tokens held: a rotated refresh token replaces the old one, and its
/// absence keeps it.
fn folded(held: &Tokens, reply: TokenResponse) -> Tokens {
    Tokens {
        access_token: reply.access_token,
        refresh_token: reply
            .refresh_token
            .unwrap_or_else(|| held.refresh_token.clone()),
        expires_at: SystemTime::now() + Duration::from_secs(reply.expires_in),
        scope: reply.scope.or_else(|| held.scope.clone()),
    }
}

/// A Web API URL for `path` with `query` encoded.
pub(crate) fn api_url(path: &str, query: &[(&str, &str)]) -> String {
    let mut url = format!("{API_BASE}{path}");
    for (i, (key, value)) in query.iter().enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        url.push_str(key);
        url.push('=');
        url.push_str(&auth::encode_component(value));
    }
    url
}

#[cfg(test)]
pub(crate) mod testing {
    //! A scripted [`SpotifyHttp`]: replies by exact URL, and a log of what was asked.

    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::http::HttpResponse;

    #[derive(Default)]
    pub struct Scripted {
        /// Replies per URL, in order; the last one repeats.
        replies: Mutex<HashMap<String, VecDeque<HttpResponse>>>,
        pub requests: Mutex<Vec<String>>,
        pub forms: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl Scripted {
        pub fn on(&self, url: &str, reply: HttpResponse) -> &Self {
            self.replies
                .lock()
                .unwrap()
                .entry(url.to_string())
                .or_default()
                .push_back(reply);
            self
        }

        pub fn json(&self, url: &str, body: &str) -> &Self {
            self.on(url, HttpResponse::new(200, body))
        }

        pub fn requested(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn reply(&self, url: &str) -> Result<HttpResponse> {
            self.requests.lock().unwrap().push(url.to_string());
            let mut replies = self.replies.lock().unwrap();
            let queue = replies
                .get_mut(url)
                .ok_or_else(|| Error::Source(format!("unscripted request: {url}")))?;
            Ok(if queue.len() > 1 {
                queue.pop_front().unwrap()
            } else {
                queue.front().cloned().unwrap()
            })
        }
    }

    #[async_trait]
    impl SpotifyHttp for Scripted {
        async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
            let bearer = headers
                .iter()
                .find(|(name, _)| *name == "Authorization")
                .map(|(_, value)| *value)
                .unwrap_or("none");
            self.requests
                .lock()
                .unwrap()
                .push(format!("auth: {bearer}"));
            self.reply(url)
        }

        async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
            self.forms.lock().unwrap().push(
                form.iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            );
            self.reply(url)
        }
    }

    /// A session already holding a fresh token, over `http`.
    pub async fn signed_in(http: Arc<Scripted>) -> SpotifySession {
        let store = TokenStore::new(crate::store::scratch("session"));
        store
            .save(&PersistedTokens {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at_unix: unix_now() + 3_600,
                scope: None,
                client_id: "client".into(),
            })
            .await
            .unwrap();
        SpotifySession::restore(http, store, "client")
            .await
            .unwrap()
    }

    pub fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Scripted, signed_in, unix_now};
    use super::*;
    use crate::auth::TOKEN_URL;
    use crate::http::HttpResponse;

    fn form_value(form: &[(String, String)], key: &str) -> Option<String> {
        form.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[tokio::test]
    async fn a_login_exchanges_the_code_and_persists_the_tokens() {
        let http = Arc::new(Scripted::default());
        http.json(
            TOKEN_URL,
            r#"{"access_token":"at-1","token_type":"Bearer","scope":"user-library-read",
                "expires_in":3600,"refresh_token":"rt-1"}"#,
        );
        let store = TokenStore::new(crate::store::scratch("login"));
        let session = SpotifySession::restore(http.clone(), store.clone(), "client")
            .await
            .unwrap();
        assert!(!session.is_authenticated());

        let url = session.login_url().await.unwrap();
        let state = url.split("state=").nth(1).unwrap().to_string();
        // A second process picks the login up from the parked file.
        let later = SpotifySession::restore(http.clone(), store.clone(), "client")
            .await
            .unwrap();
        later
            .complete_login(&format!(
                "http://127.0.0.1:8898/spotify/callback?code=the-code&state={state}"
            ))
            .await
            .unwrap();
        assert!(later.is_authenticated());

        let form = http.forms.lock().unwrap()[0].clone();
        assert_eq!(
            form_value(&form, "grant_type").as_deref(),
            Some("authorization_code")
        );
        assert_eq!(form_value(&form, "code").as_deref(), Some("the-code"));
        assert_eq!(form_value(&form, "client_id").as_deref(), Some("client"));
        assert_eq!(
            form_value(&form, "redirect_uri").as_deref(),
            Some(DEFAULT_REDIRECT_URI)
        );
        let verifier = form_value(&form, "code_verifier").unwrap();
        assert!(url.contains(&format!(
            "code_challenge={}",
            auth::challenge_for(&verifier)
        )));

        let saved = store.load().await.unwrap().unwrap();
        assert_eq!(
            (saved.access_token.as_str(), saved.refresh_token.as_str()),
            ("at-1", "rt-1")
        );
        assert_eq!(saved.client_id, "client");
        assert!(saved.expires_at_unix >= unix_now() + 3_500);
        assert_eq!(store.load_pending().await.unwrap(), None, "pending cleared");
    }

    #[tokio::test]
    async fn completing_without_a_login_in_flight_is_an_auth_error() {
        let http = Arc::new(Scripted::default());
        let store = TokenStore::new(crate::store::scratch("no-login"));
        let session = SpotifySession::restore(http, store, "client")
            .await
            .unwrap();
        assert!(matches!(
            session.complete_login("code").await,
            Err(Error::Auth(_))
        ));
    }

    async fn expired_session(http: Arc<Scripted>, store: &TokenStore) -> SpotifySession {
        store
            .save(&PersistedTokens {
                access_token: "old".into(),
                refresh_token: "rt-1".into(),
                expires_at_unix: unix_now() - 10,
                scope: Some("user-library-read".into()),
                client_id: "client".into(),
            })
            .await
            .unwrap();
        SpotifySession::restore(http, store.clone(), "client")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_refresh_without_a_new_refresh_token_keeps_the_old_one() {
        let http = Arc::new(Scripted::default());
        http.json(
            TOKEN_URL,
            r#"{"access_token":"at-2","token_type":"Bearer","expires_in":3600}"#,
        );
        http.json(
            &api_url("/me", &[]),
            r#"{"id":"joel","display_name":"Joel"}"#,
        );
        let store = TokenStore::new(crate::store::scratch("refresh-keep"));
        let session = expired_session(http.clone(), &store).await;

        let account = session.account().await.unwrap();
        assert_eq!(account.user_id, "joel");
        assert_eq!(account.username.as_deref(), Some("Joel"));
        assert_eq!(account.service, Service::Spotify);

        let form = http.forms.lock().unwrap()[0].clone();
        assert_eq!(
            form_value(&form, "grant_type").as_deref(),
            Some("refresh_token")
        );
        assert_eq!(form_value(&form, "refresh_token").as_deref(), Some("rt-1"));
        assert_eq!(form_value(&form, "client_id").as_deref(), Some("client"));
        let saved = store.load().await.unwrap().unwrap();
        assert_eq!(saved.access_token, "at-2");
        assert_eq!(saved.refresh_token, "rt-1", "kept");
        assert_eq!(saved.scope.as_deref(), Some("user-library-read"), "kept");
        assert!(http.requested().contains(&"auth: Bearer at-2".to_string()));
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_is_persisted() {
        let http = Arc::new(Scripted::default());
        http.json(
            TOKEN_URL,
            r#"{"access_token":"at-2","expires_in":3600,"refresh_token":"rt-2"}"#,
        );
        http.json(&api_url("/me", &[]), r#"{"id":"joel"}"#);
        let store = TokenStore::new(crate::store::scratch("refresh-rotate"));
        let session = expired_session(http.clone(), &store).await;
        session.account().await.unwrap();
        assert_eq!(store.load().await.unwrap().unwrap().refresh_token, "rt-2");
    }

    #[tokio::test]
    async fn concurrent_calls_share_one_refresh() {
        let http = Arc::new(Scripted::default());
        http.json(TOKEN_URL, r#"{"access_token":"at-2","expires_in":3600}"#);
        http.json(&api_url("/me", &[]), r#"{"id":"joel"}"#);
        let store = TokenStore::new(crate::store::scratch("single-flight"));
        let session = Arc::new(expired_session(http.clone(), &store).await);
        let calls = (0..4).map(|_| {
            let session = Arc::clone(&session);
            tokio::spawn(async move { session.account().await })
        });
        for call in calls {
            call.await.unwrap().unwrap();
        }
        assert_eq!(http.forms.lock().unwrap().len(), 1, "one refresh");
    }

    #[tokio::test]
    async fn a_rejected_refresh_signs_out() {
        let http = Arc::new(Scripted::default());
        http.on(
            TOKEN_URL,
            HttpResponse::new(
                400,
                r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#,
            ),
        );
        let store = TokenStore::new(crate::store::scratch("revoked"));
        let session = expired_session(http, &store).await;
        assert!(session.is_authenticated());
        let error = session.account().await.unwrap_err();
        assert!(matches!(error, Error::Auth(_)), "{error}");
        assert!(error.to_string().contains("invalid_grant"), "{error}");
        assert!(!session.is_authenticated());
    }

    #[tokio::test]
    async fn a_401_refreshes_once_and_retries() {
        let http = Arc::new(Scripted::default());
        let me = api_url("/me", &[]);
        http.on(&me, HttpResponse::new(401, r#"{"error":{"status":401}}"#))
            .json(&me, r#"{"id":"joel"}"#);
        http.json(TOKEN_URL, r#"{"access_token":"at-2","expires_in":3600}"#);
        let session = signed_in(http.clone()).await;
        assert_eq!(session.account().await.unwrap().user_id, "joel");
        assert_eq!(
            http.requested()
                .into_iter()
                .filter(|r| r.starts_with("auth:"))
                .collect::<Vec<_>>(),
            ["auth: Bearer at", "auth: Bearer at-2"]
        );
    }

    #[tokio::test]
    async fn tokens_for_another_client_id_are_ignored() {
        let http = Arc::new(Scripted::default());
        let store = TokenStore::new(crate::store::scratch("other-client"));
        store
            .save(&PersistedTokens {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at_unix: unix_now() + 3_600,
                scope: None,
                client_id: "someone-else".into(),
            })
            .await
            .unwrap();
        let session = SpotifySession::restore(http, store, "client")
            .await
            .unwrap();
        assert!(!session.is_authenticated());
    }

    #[tokio::test(start_paused = true)]
    async fn a_429_waits_out_retry_after_then_succeeds() {
        let http = Arc::new(Scripted::default());
        let me = api_url("/me", &[]);
        let limited = HttpResponse {
            status: 429,
            retry_after: Some(Duration::from_secs(7)),
            body: Vec::new(),
        };
        http.on(&me, limited.clone())
            .on(&me, limited)
            .json(&me, r#"{"id":"joel"}"#);
        let session = signed_in(http).await;
        let started = tokio::time::Instant::now();
        assert_eq!(session.account().await.unwrap().user_id, "joel");
        assert_eq!(started.elapsed(), Duration::from_secs(14));
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_429s_give_up_as_transient() {
        let http = Arc::new(Scripted::default());
        let me = api_url("/me", &[]);
        http.on(
            &me,
            HttpResponse {
                status: 429,
                retry_after: Some(Duration::from_secs(2)),
                body: Vec::new(),
            },
        );
        let session = signed_in(http.clone()).await;
        assert!(matches!(session.account().await, Err(Error::Transient(_))));
        let gets = http.requested().iter().filter(|r| **r == me).count();
        assert_eq!(gets, 1 + MAX_RATE_LIMIT_RETRIES as usize);
    }

    #[tokio::test(start_paused = true)]
    async fn a_retry_after_too_long_to_wait_fails_at_once() {
        let http = Arc::new(Scripted::default());
        let me = api_url("/me", &[]);
        http.on(
            &me,
            HttpResponse {
                status: 429,
                retry_after: Some(Duration::from_secs(3_600)),
                body: Vec::new(),
            },
        );
        let session = signed_in(http).await;
        let started = tokio::time::Instant::now();
        let error = session.account().await.unwrap_err();
        assert!(error.to_string().contains("3600"), "{error}");
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test]
    async fn next_links_off_the_api_host_are_refused() {
        let http = Arc::new(Scripted::default());
        let session = signed_in(http.clone()).await;
        let result: Result<serde_json::Value> = session.get("https://evil.example/v1/me").await;
        assert!(result.is_err());
        assert!(http.requested().is_empty());
    }

    #[test]
    fn api_urls_encode_their_query() {
        assert_eq!(
            api_url("/search", &[("q", "isrc:GBAYE0601498"), ("type", "track")]),
            "https://api.spotify.com/v1/search?q=isrc%3AGBAYE0601498&type=track"
        );
    }
}
