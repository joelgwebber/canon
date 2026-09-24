//! Tidal's ways in (docs/connections.md): the PKCE browser login, which streams, and the
//! device-code login, which only browses. Each method has its own session and its own credential
//! file, so signing in one way never replaces the other's tokens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use std::sync::Mutex;

use canon_core::{
    Capabilities, Capability, Catalog, Codec, ConnectionInfo, Connector, Error, FlowKind, Health,
    LoginFlow, LoginStatus, Method, Quality, Result, Service, Source, StreamInfo,
};

use crate::{PersistedTokens, TidalHttp, TidalSession, TidalSource, TokenStore};

/// The PKCE (Android client) login: the only one Tidal lets stream.
pub const PKCE: &str = "tidal.pkce";
/// The device-code (TV client) login: browses, and is refused playback.
pub const DEVICE: &str = "tidal.device";

/// What Tidal grants either login besides audio.
const BROWSING: Capabilities = Capabilities {
    catalog: true,
    library_read: true,
    library_write: true,
    recommendations: true,
    stream: None,
};

/// A track to probe streaming with: long in the catalog, and not region-locked in practice. If it
/// ever isn't there, the probe is inconclusive, never a verdict.
const PROBE_TRACK: &str = "33348478";

/// What use has shown about one login, beyond what its method declares.
#[derive(Default)]
pub(crate) struct Observed(Mutex<Seen>);

#[derive(Default)]
struct Seen {
    /// Why Tidal last refused this login playback, until it streams again or signs in anew.
    refused: Option<String>,
    /// The best quality it has been seen to stream.
    streamed: Option<Quality>,
}

impl Observed {
    /// Tidal refused this login playback: it no longer counts as able to stream.
    pub(crate) fn refused(&self, why: &str) {
        tracing::warn!("tidal refused playback: {why}");
        let mut seen = self.lock();
        seen.refused = Some(why.to_string());
        seen.streamed = None;
    }

    /// A stream opened: this login streams, at least at this quality.
    pub(crate) fn streamed(&self, info: &StreamInfo) {
        let quality = quality_of(info);
        let mut seen = self.lock();
        seen.refused = None;
        seen.streamed = seen.streamed.max(Some(quality));
    }

    /// Forget what was seen: a fresh login starts from what its method declares.
    fn reset(&self) {
        *self.lock() = Seen::default();
    }

    fn refusal(&self) -> Option<String> {
        self.lock().refused.clone()
    }

    fn streamed_at(&self) -> Option<Quality> {
        self.lock().streamed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The quality a stream really is, from its physical description.
fn quality_of(info: &StreamInfo) -> Quality {
    match info.codec {
        Codec::Flac | Codec::Alac | Codec::Pcm if info.bit_depth.unwrap_or(16) > 16 => {
            Quality::HiRes
        }
        Codec::Flac | Codec::Alac | Codec::Pcm => Quality::Lossless,
        _ => Quality::High,
    }
}

/// Tidal's connector: one session per login method.
pub struct TidalConnector {
    pkce: Arc<TidalSession>,
    device: Arc<TidalSession>,
    /// What playback and probes have shown about the PKCE login (the only one that streams).
    pkce_seen: Arc<Observed>,
}

impl TidalConnector {
    /// Restore both logins' saved credentials from `dir` (`tidal.pkce.json`,
    /// `tidal.device.json`). A `tidal.json` from before logins were kept apart is moved to the
    /// file for the method that made it.
    ///
    /// # Errors
    /// A credential file is corrupt, or the old one couldn't be moved.
    pub async fn restore(http: Arc<dyn TidalHttp>, dir: &Path) -> Result<Self> {
        migrate_legacy(dir).await?;
        let session = |method: &str| {
            TidalSession::restore(Arc::clone(&http), TokenStore::new(credentials(dir, method)))
        };
        Ok(Self {
            pkce: Arc::new(session(PKCE).await?),
            device: Arc::new(session(DEVICE).await?),
            pkce_seen: Arc::new(Observed::default()),
        })
    }

    /// Check with Tidal what the streaming login really does: resolve a known track's playback.
    /// A refusal takes streaming away until it signs in again; a network failure or a missing
    /// probe track proves nothing and changes nothing.
    pub async fn probe(&self) {
        if !self.pkce.is_authenticated() {
            return;
        }
        match self.pkce.resolve_stream(PROBE_TRACK, Quality::HiRes).await {
            Ok(resolved) => {
                self.pkce_seen.streamed(&resolved.info);
                tracing::info!(
                    "tidal streaming confirmed ({:?})",
                    self.pkce_seen.streamed_at()
                );
            }
            Err(Error::Auth(why)) => self.pkce_seen.refused(&why),
            Err(e) => tracing::info!("tidal streaming probe inconclusive: {e}"),
        }
    }

    /// The session behind `method`.
    ///
    /// # Errors
    /// Tidal has no such method.
    pub fn session(&self, method: &str) -> Result<&Arc<TidalSession>> {
        match method {
            PKCE => Ok(&self.pkce),
            DEVICE => Ok(&self.device),
            other => Err(Error::NotFound(format!("no Tidal login method {other}"))),
        }
    }

    /// The signed-in session best placed for `need`: streaming needs the PKCE login; anything
    /// else prefers it and makes do with the device login.
    #[must_use]
    pub fn session_for(&self, need: Capability) -> Option<&Arc<TidalSession>> {
        let candidates: &[(&Arc<TidalSession>, &str)] =
            &[(&self.pkce, PKCE), (&self.device, DEVICE)];
        candidates
            .iter()
            .find(|(session, method)| {
                session.is_authenticated()
                    && grants(method).has(need)
                    && !(need == Capability::Stream && self.refused(method))
            })
            .map(|(session, _)| *session)
    }

    /// Whether Tidal has refused `method` playback since it signed in.
    fn refused(&self, method: &str) -> bool {
        method == PKCE && self.pkce_seen.refusal().is_some()
    }
}

/// Where `method`'s credentials live.
fn credentials(dir: &Path, method: &str) -> PathBuf {
    dir.join(format!("{method}.json"))
}

/// What a login made with `method` grants.
fn grants(method: &str) -> Capabilities {
    match method {
        PKCE => Capabilities {
            stream: Some(Quality::HiRes),
            ..BROWSING
        },
        _ => BROWSING,
    }
}

/// Before logins were kept apart, one `tidal.json` held whichever was made last, flagged
/// `is_pkce`. Move it to that method's file, unless that file is newer.
///
/// "Newer" matters: an older canon still running writes `tidal.json` on every token refresh, and
/// Tidal rotates refresh tokens, so after the first move the newest tokens can be back in
/// `tidal.json` while the method's file holds rotated-out ones.
async fn migrate_legacy(dir: &Path) -> Result<()> {
    let legacy = dir.join("tidal.json");
    let Some(tokens): Option<PersistedTokens> = TokenStore::new(&legacy).load().await? else {
        return Ok(());
    };
    let target = credentials(dir, if tokens.is_pkce { PKCE } else { DEVICE });
    if let Ok(existing) = tokio::fs::metadata(&target).await {
        let legacy_modified = tokio::fs::metadata(&legacy).await?.modified()?;
        if existing.modified()? >= legacy_modified {
            return Ok(());
        }
    }
    tokio::fs::rename(&legacy, &target).await?;
    tracing::info!("moved {} to {}", legacy.display(), target.display());
    Ok(())
}

#[async_trait]
impl Connector for TidalConnector {
    fn service(&self) -> Service {
        Service::Tidal
    }

    fn methods(&self) -> Vec<Method> {
        vec![
            Method {
                id: PKCE.into(),
                service: Service::Tidal,
                label: "Tidal (browser login)".into(),
                flow: FlowKind::Browser,
                grants: grants(PKCE),
                note: "Log in in a browser, then paste back the URL of the page you land on \
                       (a blank or \"Oops\" page). The only Tidal login that can stream."
                    .into(),
            },
            Method {
                id: DEVICE.into(),
                service: Service::Tidal,
                label: "Tidal (code on another device)".into(),
                flow: FlowKind::DeviceCode,
                grants: grants(DEVICE),
                note: "Browsing and your library only: Tidal refuses this login playback.".into(),
            },
        ]
    }

    async fn connections(&self) -> Vec<ConnectionInfo> {
        let mut connections = Vec::new();
        for method in self.methods() {
            let Ok(session) = self.session(&method.id) else {
                continue;
            };
            let refusal = (method.id == PKCE)
                .then(|| self.pkce_seen.refusal())
                .flatten();
            let (health, account) = if session.is_authenticated() {
                match (session.account().await, refusal) {
                    (Ok(account), Some(why)) => (Health::Degraded { why }, Some(account)),
                    (Ok(account), None) => (Health::Ok, Some(account)),
                    (Err(e), _) => (Health::Failing { why: e.to_string() }, None),
                }
            } else {
                (Health::NeedsLogin, None)
            };
            let verified = if method.id == PKCE && session.is_authenticated() {
                match (self.pkce_seen.refusal(), self.pkce_seen.streamed_at()) {
                    (Some(_), _) => Some(Capabilities {
                        stream: None,
                        ..method.grants
                    }),
                    (None, Some(quality)) => Some(Capabilities {
                        stream: Some(quality),
                        ..method.grants
                    }),
                    (None, None) => None,
                }
            } else {
                None
            };
            connections.push(ConnectionInfo {
                id: method.id,
                service: Service::Tidal,
                label: method.label,
                grants: method.grants,
                verified,
                health,
                account,
            });
        }
        connections
    }

    async fn begin(&self, method: &str) -> Result<LoginFlow> {
        let session = self.session(method)?;
        match method {
            PKCE => Ok(LoginFlow::Browser {
                url: session.pkce_login_url().await?,
            }),
            _ => Ok(LoginFlow::DeviceCode {
                code: session.begin_login().await?,
            }),
        }
    }

    async fn complete(&self, method: &str, input: Option<String>) -> Result<LoginStatus> {
        let session = self.session(method)?;
        match (method, input) {
            (PKCE, Some(redirect)) => {
                session.complete_pkce_login(&redirect).await?;
                self.pkce_seen.reset();
                self.probe().await;
                Ok(LoginStatus::Authorized)
            }
            (PKCE, None) => Err(Error::Auth(
                "finish the browser login with the URL you landed on".into(),
            )),
            _ => session.poll_login().await,
        }
    }

    async fn disconnect(&self, method: &str) -> Result<()> {
        if method == PKCE {
            self.pkce_seen.reset();
        }
        self.session(method)?.forget().await
    }

    fn grants(&self, capability: Capability) -> bool {
        self.session_for(capability).is_some()
    }

    fn source(&self, need: Capability) -> Option<Arc<dyn Source>> {
        let session = Arc::clone(self.session_for(need)?);
        // Only the streaming login's playback says anything about its entitlement.
        let source = if Arc::ptr_eq(&session, &self.pkce) {
            TidalSource::observed(session, Arc::clone(&self.pkce_seen))
        } else {
            TidalSource::new(session)
        };
        Some(Arc::new(source))
    }

    fn catalog(&self) -> Option<Arc<dyn Catalog>> {
        self.session_for(Capability::Catalog)
            .map(|session| Arc::new(TidalSource::new(Arc::clone(session))) as Arc<dyn Catalog>)
    }

    fn hint(&self, capability: Capability) -> String {
        match capability {
            Capability::Stream if self.pkce.is_authenticated() => match self.pkce_seen.refusal() {
                Some(why) => format!(
                    "Tidal refused playback for the browser login ({PKCE}): {why}. Sign in again \
                     with `connect {PKCE}`"
                ),
                None => format!("sign in with the browser login ({PKCE})"),
            },
            Capability::Stream if self.device.is_authenticated() => format!(
                "the code login ({DEVICE}) can't stream; sign in with the browser login ({PKCE})"
            ),
            Capability::Stream => format!("sign in with the browser login ({PKCE})"),
            _ => format!("sign in to Tidal ({PKCE}, or {DEVICE} for browsing only)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("canon-tidal-connector-{name}-{nanos}"))
    }

    fn tokens(is_pkce: bool) -> PersistedTokens {
        PersistedTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_unix: u64::MAX / 2,
            user_id: Some(42),
            is_pkce,
        }
    }

    #[test]
    fn only_the_browser_login_streams() {
        assert!(grants(PKCE).has(Capability::Stream));
        assert!(!grants(DEVICE).has(Capability::Stream));
        assert!(grants(DEVICE).has(Capability::Catalog));
    }

    /// The old single token file goes to the method that made it, and only once.
    #[tokio::test]
    async fn a_legacy_token_file_moves_to_its_method() {
        for (is_pkce, method) in [(true, PKCE), (false, DEVICE)] {
            let dir = scratch(method);
            TokenStore::new(dir.join("tidal.json"))
                .save(&tokens(is_pkce))
                .await
                .unwrap();
            migrate_legacy(&dir).await.unwrap();
            assert!(!dir.join("tidal.json").exists());
            let moved = TokenStore::new(credentials(&dir, method))
                .load()
                .await
                .unwrap();
            assert_eq!(moved.map(|t| t.is_pkce), Some(is_pkce));
            migrate_legacy(&dir).await.unwrap(); // nothing left to move
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// An older canon still running refreshes into `tidal.json` after the move; the newer file
    /// wins, and an older one is left alone.
    #[tokio::test]
    async fn a_newer_legacy_file_replaces_the_moved_one() {
        let dir = scratch("newer");
        let target = credentials(&dir, PKCE);
        let mut old = tokens(true);
        old.refresh_token = "rotated-out".into();
        TokenStore::new(&target).save(&old).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        TokenStore::new(dir.join("tidal.json"))
            .save(&tokens(true))
            .await
            .unwrap();
        migrate_legacy(&dir).await.unwrap();
        let kept = TokenStore::new(&target).load().await.unwrap().unwrap();
        assert_eq!(kept.refresh_token, "rt", "the newer tokens won");

        std::thread::sleep(std::time::Duration::from_millis(20));
        TokenStore::new(dir.join("tidal.json"))
            .save(&old)
            .await
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        TokenStore::new(&target).save(&tokens(true)).await.unwrap();
        migrate_legacy(&dir).await.unwrap();
        let kept = TokenStore::new(&target).load().await.unwrap().unwrap();
        assert_eq!(
            kept.refresh_token, "rt",
            "an older legacy file doesn't clobber"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two logins, two files: a device-code login no longer replaces the streaming one.
    #[tokio::test]
    async fn each_login_keeps_its_own_credentials() {
        let dir = scratch("separate");
        TokenStore::new(credentials(&dir, PKCE))
            .save(&tokens(true))
            .await
            .unwrap();
        let http: Arc<dyn TidalHttp> = Arc::new(crate::WreqHttp::chrome_android().unwrap());
        let connector = TidalConnector::restore(Arc::clone(&http), &dir)
            .await
            .unwrap();
        assert!(connector.grants(Capability::Stream));
        assert!(connector.source(Capability::Stream).is_some());

        connector.disconnect(DEVICE).await.unwrap(); // not signed in: harmless
        assert!(
            connector.grants(Capability::Stream),
            "the browser login is untouched"
        );
        connector.disconnect(PKCE).await.unwrap();
        assert!(!connector.grants(Capability::Stream));
        assert!(!credentials(&dir, PKCE).exists());
        assert!(connector.hint(Capability::Stream).contains(PKCE));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Answers by URL: the account is fine, playback is refused the way Tidal refuses a login it
    /// won't let stream.
    struct RefusingHttp;

    #[async_trait]
    impl TidalHttp for RefusingHttp {
        async fn get(&self, url: &str, _headers: &[(&str, &str)]) -> Result<crate::HttpResponse> {
            let (status, body) = if url.contains("/v1/sessions") {
                (
                    200,
                    r#"{"userId": 42, "sessionId": "s", "countryCode": "US"}"#,
                )
            } else if url.contains("playbackinfopostpaywall") {
                (
                    401,
                    r#"{"status": 401, "subStatus": 4005, "userMessage": "Asset is not ready for playback"}"#,
                )
            } else {
                (404, "{}")
            };
            Ok(crate::HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            })
        }

        async fn post_form(
            &self,
            _url: &str,
            _form: &[(&str, &str)],
            _headers: &[(&str, &str)],
        ) -> Result<crate::HttpResponse> {
            Err(Error::Unsupported("no posts here".into()))
        }
    }

    /// A login Tidal refuses playback stops being offered for streaming, and says why, instead of
    /// failing every track the same way.
    #[tokio::test]
    async fn a_refused_login_is_routed_around_and_says_why() {
        let dir = scratch("refused");
        TokenStore::new(credentials(&dir, PKCE))
            .save(&tokens(true))
            .await
            .unwrap();
        let connector = TidalConnector::restore(Arc::new(RefusingHttp), &dir)
            .await
            .unwrap();
        assert!(connector.grants(Capability::Stream), "on paper, it streams");

        // Real playback is refused: the source reports it.
        let source = connector.source(Capability::Stream).unwrap();
        let opened = source
            .open(
                &canon_core::SourceRef::Tidal { id: "1".into() },
                Quality::Lossless,
                std::time::Duration::ZERO,
            )
            .await;
        assert!(matches!(opened, Err(Error::Auth(_))));
        assert!(!connector.grants(Capability::Stream));
        assert!(connector.source(Capability::Stream).is_none());
        assert!(connector.grants(Capability::Catalog), "it still browses");
        assert!(
            connector
                .hint(Capability::Stream)
                .contains("refused playback"),
            "{}",
            connector.hint(Capability::Stream)
        );

        let info = connector.connections().await;
        let pkce = info.iter().find(|c| c.id == PKCE).unwrap();
        assert!(
            matches!(pkce.health, Health::Degraded { .. }),
            "{:?}",
            pkce.health
        );
        assert_eq!(pkce.verified.map(|v| v.stream), Some(None));

        // Signing out and in again starts from what the method declares.
        connector.disconnect(PKCE).await.unwrap();
        TokenStore::new(credentials(&dir, PKCE))
            .save(&tokens(true))
            .await
            .unwrap();
        let connector = TidalConnector::restore(Arc::new(RefusingHttp), &dir)
            .await
            .unwrap();
        connector.probe().await; // refused again, by the probe this time
        assert!(!connector.grants(Capability::Stream));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
