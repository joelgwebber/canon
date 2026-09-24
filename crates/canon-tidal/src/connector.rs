//! Tidal's ways in (docs/connections.md): the PKCE browser login, which streams, and the
//! device-code login, which only browses. Each method has its own session and its own credential
//! file, so signing in one way never replaces the other's tokens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use canon_core::{
    Capabilities, Capability, Catalog, ConnectionInfo, Connector, Error, FlowKind, Health,
    LoginFlow, LoginStatus, Method, Quality, Result, Service, Source,
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

/// Tidal's connector: one session per login method.
pub struct TidalConnector {
    pkce: Arc<TidalSession>,
    device: Arc<TidalSession>,
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
        })
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
            .find(|(session, method)| session.is_authenticated() && grants(method).has(need))
            .map(|(session, _)| *session)
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
            let (health, account) = if session.is_authenticated() {
                match session.account().await {
                    Ok(account) => (Health::Ok, Some(account)),
                    Err(e) => (Health::Failing { why: e.to_string() }, None),
                }
            } else {
                (Health::NeedsLogin, None)
            };
            connections.push(ConnectionInfo {
                id: method.id,
                service: Service::Tidal,
                label: method.label,
                grants: method.grants,
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
                Ok(LoginStatus::Authorized)
            }
            (PKCE, None) => Err(Error::Auth(
                "finish the browser login with the URL you landed on".into(),
            )),
            _ => session.poll_login().await,
        }
    }

    async fn disconnect(&self, method: &str) -> Result<()> {
        self.session(method)?.forget().await
    }

    fn grants(&self, capability: Capability) -> bool {
        self.session_for(capability).is_some()
    }

    fn source(&self, need: Capability) -> Option<Arc<dyn Source>> {
        self.session_for(need)
            .map(|session| Arc::new(TidalSource::new(Arc::clone(session))) as Arc<dyn Source>)
    }

    fn catalog(&self) -> Option<Arc<dyn Catalog>> {
        self.session_for(Capability::Catalog)
            .map(|session| Arc::new(TidalSource::new(Arc::clone(session))) as Arc<dyn Catalog>)
    }

    fn hint(&self, capability: Capability) -> String {
        match capability {
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
}
