//! Spotify's way in (docs/connections.md, yak canon-6272): the Web API login `spotify.web`, which
//! browses, reads and writes the library, and has no audio. Spotify tracks play through a
//! streaming connection on another service, matched by ISRC.
//!
//! The client id comes from the user's own developer app, set as `spotify.client_id` in
//! settings. The connector reads it from the [`SettingsStore`] whenever it signs in or reports its
//! connections, so setting it (or changing it) takes effect without a restart. Sessions are keyed
//! by it: tokens issued to another app are ignored, since only their own app can refresh them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{
    Capabilities, Capability, Catalog, ConnectionInfo, Connector, Error, FlowKind, Health,
    LoginFlow, LoginStatus, Method, Quality, ResolvedStream, Result, Service, SettingsStore,
    Source, SourceRef, SourceTrack,
};

use crate::{DEFAULT_REDIRECT_URI, SpotifyHttp, SpotifySession, TokenStore};

/// The Web API login, with the user's own developer app.
pub const WEB: &str = "spotify.web";

/// What a Web API login grants. Spotify took recommendations from development-mode apps in
/// November 2024, and the Web API has never carried audio.
const GRANTS: Capabilities = Capabilities {
    catalog: true,
    library_read: true,
    library_write: true,
    recommendations: false,
    stream: None,
};

/// Spotify's connector: one Web API session, for the client id currently in settings.
pub struct SpotifyConnector {
    http: Arc<dyn SpotifyHttp>,
    credentials: PathBuf,
    settings: Arc<dyn SettingsStore>,
    /// The session for the client id last read from settings; `None` when there was none.
    session: RwLock<Option<Arc<SpotifySession>>>,
    /// Held while the session is swapped for another client id's, so two callers don't both
    /// restore it.
    swapping: tokio::sync::Mutex<()>,
}

impl SpotifyConnector {
    /// Restore the saved login from `dir` (`spotify.web.json`), for the client id in `settings`.
    ///
    /// # Errors
    /// The credential file is corrupt.
    pub async fn restore(
        http: Arc<dyn SpotifyHttp>,
        dir: &Path,
        settings: Arc<dyn SettingsStore>,
    ) -> Result<Self> {
        let connector = Self {
            http,
            credentials: credentials(dir, WEB),
            settings,
            session: RwLock::new(None),
            swapping: tokio::sync::Mutex::new(()),
        };
        connector.current().await?;
        Ok(connector)
    }

    /// The client id in settings, if one is set.
    fn client_id(&self) -> Option<String> {
        self.settings
            .get()
            .spotify
            .client_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    }

    /// The session as last restored, without looking at settings again.
    fn held(&self) -> Option<Arc<SpotifySession>> {
        self.session
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The held session if it is signed in.
    fn signed_in(&self) -> Option<Arc<SpotifySession>> {
        self.held().filter(|session| session.is_authenticated())
    }

    /// The session for the client id settings hold now, restored afresh if it has changed.
    async fn current(&self) -> Result<Option<Arc<SpotifySession>>> {
        let _swapping = self.swapping.lock().await;
        let wanted = self.client_id();
        let held = self.held();
        if held.as_ref().map(|session| session.client_id()) == wanted.as_deref() {
            return Ok(held);
        }
        let session = match wanted {
            Some(client_id) => Some(Arc::new(
                SpotifySession::restore(
                    Arc::clone(&self.http),
                    TokenStore::new(&self.credentials),
                    client_id,
                )
                .await?,
            )),
            None => None,
        };
        *self
            .session
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = session.clone();
        Ok(session)
    }

    /// The session for `method`, or why there can't be one.
    async fn session(&self, method: &str) -> Result<Arc<SpotifySession>> {
        if method != WEB {
            return Err(Error::NotFound(format!("no Spotify login method {method}")));
        }
        self.current()
            .await?
            .ok_or_else(|| Error::Auth(NO_CLIENT_ID.into()))
    }
}

/// What to do before `spotify.web` can sign in.
const NO_CLIENT_ID: &str = "no Spotify client id: register an app at \
     developer.spotify.com (its owner needs Premium), add \
     http://127.0.0.1:8898/spotify/callback as its redirect URI, then set spotify.client_id in \
     settings.json (or with set_settings) to the app's client id";

/// Where `method`'s credentials live.
fn credentials(dir: &Path, method: &str) -> PathBuf {
    dir.join(format!("{method}.json"))
}

#[async_trait]
impl Connector for SpotifyConnector {
    fn service(&self) -> Service {
        Service::Spotify
    }

    fn methods(&self) -> Vec<Method> {
        let mut note = format!(
            "Log in in a browser, then paste back the URL of the page you land on ({DEFAULT_REDIRECT_URI}, \
             which won't load). Library and browsing only: Spotify's audio isn't available this \
             way, so its tracks play through a streaming service matched by ISRC."
        );
        if self.client_id().is_none() {
            note = format!("Not set up: {NO_CLIENT_ID}. {note}");
        }
        vec![Method {
            id: WEB.into(),
            service: Service::Spotify,
            label: "Spotify (browser login)".into(),
            flow: FlowKind::Browser,
            grants: GRANTS,
            note,
        }]
    }

    async fn connections(&self) -> Vec<ConnectionInfo> {
        let (health, account) = match self.current().await {
            Ok(Some(session)) if session.is_authenticated() => match session.account().await {
                Ok(account) => (Health::Ok, Some(account)),
                Err(e) => (Health::Failing { why: e.to_string() }, None),
            },
            Ok(_) => (Health::NeedsLogin, None),
            Err(e) => (Health::Failing { why: e.to_string() }, None),
        };
        vec![ConnectionInfo {
            id: WEB.into(),
            service: Service::Spotify,
            label: "Spotify (browser login)".into(),
            grants: GRANTS,
            // Nothing to probe: this login never streams, and account() above checks the rest.
            verified: None,
            health,
            account,
        }]
    }

    async fn begin(&self, method: &str) -> Result<LoginFlow> {
        let session = self.session(method).await?;
        Ok(LoginFlow::Browser {
            url: session.login_url().await?,
        })
    }

    async fn complete(&self, method: &str, input: Option<String>) -> Result<LoginStatus> {
        let session = self.session(method).await?;
        let Some(redirect) = input else {
            return Err(Error::Auth(
                "finish the browser login with the URL you landed on".into(),
            ));
        };
        session.complete_login(&redirect).await?;
        Ok(LoginStatus::Authorized)
    }

    async fn disconnect(&self, method: &str) -> Result<()> {
        if method != WEB {
            return Err(Error::NotFound(format!("no Spotify login method {method}")));
        }
        match self.current().await? {
            Some(session) => session.forget().await,
            // No client id, so no session: the file may still hold another app's tokens.
            None => TokenStore::new(&self.credentials).clear().await,
        }
    }

    fn grants(&self, capability: Capability) -> bool {
        GRANTS.has(capability) && self.signed_in().is_some()
    }

    fn source(&self, need: Capability) -> Option<Arc<dyn Source>> {
        if !self.grants(need) {
            return None;
        }
        self.signed_in()
            .map(|session| Arc::new(SpotifySource { session }) as Arc<dyn Source>)
    }

    fn catalog(&self) -> Option<Arc<dyn Catalog>> {
        if !GRANTS.catalog {
            return None;
        }
        self.signed_in().map(|session| session as Arc<dyn Catalog>)
    }

    fn hint(&self, capability: Capability) -> String {
        match capability {
            Capability::Stream => "Spotify's Web API has no audio; Spotify tracks play through a \
                                   streaming service by ISRC match (Spotify's own audio would \
                                   need librespot, which canon doesn't have yet)"
                .into(),
            Capability::Recommendations => {
                "Spotify no longer gives development-mode apps recommendations".into()
            }
            _ if self.client_id().is_none() => NO_CLIENT_ID.into(),
            _ => format!("sign in to Spotify ({WEB})"),
        }
    }
}

/// Spotify as a [`Source`]: it describes tracks, and can't open them.
struct SpotifySource {
    session: Arc<SpotifySession>,
}

#[async_trait]
impl Source for SpotifySource {
    fn service(&self) -> Service {
        Service::Spotify
    }

    async fn open(
        &self,
        _source: &SourceRef,
        _quality: Quality,
        _start: Duration,
    ) -> Result<ResolvedStream> {
        Err(Error::NotEntitled {
            service: Service::Spotify,
            capability: Capability::Stream,
            hint: "Spotify's Web API has no audio; play it through a streaming service by ISRC \
                   match"
                .into(),
        })
    }

    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        match source {
            SourceRef::Spotify { id } => self.session.track(id).await,
            other => Err(Error::Unsupported(format!(
                "Spotify can't describe a {} binding",
                other.service()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use canon_core::{Settings, SpotifySettings};

    use super::*;
    use crate::PersistedTokens;
    use crate::session::testing::{Scripted, unix_now};

    #[derive(Default)]
    struct MemorySettings(Mutex<Settings>);

    impl MemorySettings {
        fn with_client_id(id: Option<&str>) -> Arc<Self> {
            let settings = Settings {
                spotify: SpotifySettings {
                    client_id: id.map(str::to_string),
                },
                ..Settings::default()
            };
            Arc::new(Self(Mutex::new(settings)))
        }
    }

    #[async_trait]
    impl SettingsStore for MemorySettings {
        fn get(&self) -> Settings {
            self.0.lock().unwrap().clone()
        }

        async fn set(&self, settings: Settings) -> Result<()> {
            *self.0.lock().unwrap() = settings;
            Ok(())
        }
    }

    fn scratch() -> PathBuf {
        crate::store::scratch("connector")
            .parent()
            .unwrap()
            .to_path_buf()
    }

    async fn save_tokens(dir: &Path, client_id: &str) {
        TokenStore::new(credentials(dir, WEB))
            .save(&PersistedTokens {
                access_token: "at".into(),
                refresh_token: "rt".into(),
                expires_at_unix: unix_now() + 3_600,
                scope: None,
                client_id: client_id.into(),
            })
            .await
            .unwrap();
    }

    async fn connector(
        http: Arc<Scripted>,
        dir: &Path,
        settings: Arc<MemorySettings>,
    ) -> SpotifyConnector {
        SpotifyConnector::restore(http, dir, settings)
            .await
            .unwrap()
    }

    #[test]
    fn the_web_login_browses_and_keeps_a_library_but_has_no_audio() {
        assert!(GRANTS.has(Capability::Catalog));
        assert!(GRANTS.has(Capability::LibraryRead));
        assert!(GRANTS.has(Capability::LibraryWrite));
        assert!(!GRANTS.has(Capability::Recommendations));
        assert!(!GRANTS.has(Capability::Stream));
    }

    /// With no client id the method is still offered, saying what to set, and starting it fails
    /// with the steps rather than a Spotify error page.
    #[tokio::test]
    async fn without_a_client_id_the_method_says_what_to_set() {
        let dir = scratch();
        let connector = connector(
            Arc::new(Scripted::default()),
            &dir,
            MemorySettings::with_client_id(None),
        )
        .await;
        let methods = connector.methods();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].id, WEB);
        assert_eq!(methods[0].flow, FlowKind::Browser);
        assert_eq!(methods[0].grants, GRANTS);
        assert!(methods[0].note.contains("spotify.client_id"));

        let err = connector.begin(WEB).await.unwrap_err().to_string();
        assert!(err.contains("spotify.client_id"), "{err}");
        assert!(err.contains(DEFAULT_REDIRECT_URI), "{err}");
        assert!(err.contains("Premium"), "{err}");
        assert!(connector.begin("spotify.other").await.is_err());

        let connections = connector.connections().await;
        assert_eq!(connections[0].health, Health::NeedsLogin);
        assert!(!connector.grants(Capability::Catalog));
        assert!(connector.catalog().is_none());
    }

    /// Setting the client id needs no restart: the next sign-in reads it.
    #[tokio::test]
    async fn a_client_id_set_later_is_picked_up() {
        let dir = scratch();
        let settings = MemorySettings::with_client_id(None);
        let connector = connector(Arc::new(Scripted::default()), &dir, settings.clone()).await;
        assert!(connector.begin(WEB).await.is_err());

        let mut changed = settings.get();
        changed.spotify.client_id = Some("my-app".into());
        settings.set(changed).await.unwrap();
        let LoginFlow::Browser { url } = connector.begin(WEB).await.unwrap() else {
            panic!("spotify.web is a browser login");
        };
        assert!(url.contains("client_id=my-app"), "{url}");
        assert!(!connector.methods()[0].note.contains("Not set up"));
        assert!(
            dir.join("spotify.web.pending.json").exists(),
            "the login in flight is parked beside the method's credentials"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The method's own file holds its tokens; signing out removes it, and nothing else.
    #[tokio::test]
    async fn the_login_lives_in_its_own_credential_file() {
        let dir = scratch();
        save_tokens(&dir, "my-app").await;
        std::fs::write(dir.join("tidal.pkce.json"), "{}").unwrap();
        let connector = connector(
            Arc::new(Scripted::default()),
            &dir,
            MemorySettings::with_client_id(Some("my-app")),
        )
        .await;
        assert!(connector.grants(Capability::Catalog));
        assert!(connector.grants(Capability::LibraryRead));
        assert!(connector.catalog().is_some());
        assert!(connector.source(Capability::Catalog).is_some());
        assert!(connector.source(Capability::Stream).is_none());

        connector.disconnect(WEB).await.unwrap();
        assert!(!credentials(&dir, WEB).exists());
        assert!(dir.join("tidal.pkce.json").exists());
        assert!(!connector.grants(Capability::Catalog));
        connector.disconnect(WEB).await.unwrap(); // already signed out: harmless
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Tokens from another app can't be refreshed by this one, so they don't count.
    #[tokio::test]
    async fn tokens_from_another_client_id_are_not_a_login() {
        let dir = scratch();
        save_tokens(&dir, "old-app").await;
        let connector = connector(
            Arc::new(Scripted::default()),
            &dir,
            MemorySettings::with_client_id(Some("new-app")),
        )
        .await;
        assert!(!connector.grants(Capability::Catalog));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn the_source_describes_tracks_and_refuses_to_open_them() {
        let dir = scratch();
        save_tokens(&dir, "my-app").await;
        let http = Arc::new(Scripted::default());
        http.json(
            &crate::session::api_url("/tracks/t1", &[]),
            r#"{"id": "t1", "name": "Army of Me", "type": "track", "is_local": false,
                "artists": [{"id": "a1", "name": "Björk"}], "duration_ms": 234200,
                "external_ids": {"isrc": "GBAYE9500001"}}"#,
        );
        let connector = connector(http, &dir, MemorySettings::with_client_id(Some("my-app"))).await;
        let source = connector.source(Capability::Catalog).unwrap();
        let binding = SourceRef::Spotify { id: "t1".into() };
        let track = source.describe(&binding).await.unwrap();
        assert_eq!(track.isrc.as_deref(), Some("GBAYE9500001"));
        let refused = source
            .open(&binding, Quality::Lossless, Duration::ZERO)
            .await;
        assert!(matches!(
            refused,
            Err(Error::NotEntitled {
                capability: Capability::Stream,
                ..
            })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hints_explain_what_spotify_cannot_do() {
        let connector = SpotifyConnector {
            http: Arc::new(Scripted::default()),
            credentials: PathBuf::from("unused"),
            settings: MemorySettings::with_client_id(None),
            session: RwLock::new(None),
            swapping: tokio::sync::Mutex::new(()),
        };
        let stream = connector.hint(Capability::Stream);
        assert!(stream.contains("ISRC"), "{stream}");
        assert!(stream.contains("librespot"), "{stream}");
        assert!(
            connector
                .hint(Capability::Catalog)
                .contains("spotify.client_id")
        );
    }
}
