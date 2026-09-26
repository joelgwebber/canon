//! `spotify.librespot`: Spotify's audio as a canon connection (docs/connections.md, yak
//! canon-58c3).
//!
//! It grants one thing, streaming, at Ogg Vorbis 320. Browsing and the library stay with the Web
//! API connection (`spotify.web`); `Sources` takes each capability from whichever of a service's
//! connectors grants it.
//!
//! Sign-in is a browser login finished by pasting back the URL landed on, like `tidal.pkce`.
//! librespot's own OAuth helper waits for the redirect on a blocking listener, which would stall
//! the daemon's runtime and needs the browser on the same machine, so canon builds the PKCE
//! authorize URL and exchanges the code itself, then hands librespot the access token. librespot
//! caches reusable credentials, so later starts need no browser.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use canon_core::{
    Account, Capabilities, Capability, Catalog, ConnectionInfo, Connector, Error, FlowKind, Health,
    LoginFlow, LoginStatus, Method, Quality, ResolvedStream, Result, Service, Source, SourceRef,
    SourceTrack,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{CLIENT_ID, REDIRECT_URI, SpotifyAudio};

/// The one login method.
pub const LIBRESPOT: &str = "spotify.librespot";

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Audio only, at Spotify's best Ogg Vorbis (no lossless is offered to this client).
const GRANTS: Capabilities = Capabilities {
    catalog: false,
    library_read: false,
    library_write: false,
    recommendations: false,
    stream: Some(Quality::High),
};

/// A browser login in flight: what the code exchange must prove.
struct Pending {
    verifier: String,
    state: String,
}

/// Spotify audio through librespot.
pub struct LibrespotConnector {
    dir: PathBuf,
    audio: RwLock<Option<Arc<SpotifyAudio>>>,
    /// Why the cached login couldn't be resumed at startup, if it couldn't.
    restore_failed: Mutex<Option<String>>,
    pending: Mutex<Option<Pending>>,
    /// Why Spotify last refused this login audio, until it plays again or signs in anew.
    refused: Arc<Mutex<Option<String>>>,
    http: reqwest::Client,
}

impl LibrespotConnector {
    /// Resume the login cached in `dir`, if there is one. Never fails: a login that can't be
    /// resumed shows as failing in `services`, and can be signed in again.
    pub async fn restore(dir: &Path) -> Self {
        let (audio, restore_failed) = match SpotifyAudio::restore(dir).await {
            Ok(audio) => (audio.map(Arc::new), None),
            Err(e) => {
                tracing::warn!("spotify.librespot: couldn't resume the cached login: {e}");
                (None, Some(e.to_string()))
            }
        };
        if let Some(audio) = &audio {
            tracing::info!("spotify audio restored for {}", audio.username());
        }
        Self {
            dir: dir.to_path_buf(),
            audio: RwLock::new(audio),
            restore_failed: Mutex::new(restore_failed),
            pending: Mutex::new(None),
            refused: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
        }
    }

    fn audio(&self) -> Option<Arc<SpotifyAudio>> {
        self.audio
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn refusal(&self) -> Option<String> {
        self.refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn check_method(method: &str) -> Result<()> {
        if method == LIBRESPOT {
            Ok(())
        } else {
            Err(Error::NotFound(format!(
                "no librespot login method {method}"
            )))
        }
    }

    /// Exchange the authorization code from `redirect` for an access token.
    async fn exchange(&self, redirect: &str) -> Result<String> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| Error::Auth(format!("no {LIBRESPOT} login in flight: connect first")))?;
        let (code, state) = code_and_state(redirect)?;
        if state.as_deref() != Some(pending.state.as_str()) {
            return Err(Error::Auth(
                "that URL is from a different login attempt: connect again".into(),
            ));
        }
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT_URI),
                ("client_id", CLIENT_ID),
                ("code_verifier", pending.verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|e| Error::Transient(format!("spotify token exchange: {e}")))?;
        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| Error::Transient(format!("spotify token exchange: {e}")))?;
        if !status.is_success() {
            return Err(Error::Auth(format!(
                "spotify refused the login: HTTP {status} {}",
                String::from_utf8_lossy(&body)
            )));
        }
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
        }
        let token: Token = serde_json::from_slice(&body)
            .map_err(|e| Error::Auth(format!("spotify token reply: {e}")))?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl Connector for LibrespotConnector {
    fn service(&self) -> Service {
        Service::Spotify
    }

    fn methods(&self) -> Vec<Method> {
        vec![Method {
            id: LIBRESPOT.into(),
            service: Service::Spotify,
            label: "Spotify audio (librespot)".into(),
            flow: FlowKind::Browser,
            grants: GRANTS,
            note: "Spotify Premium only. Unofficial (librespot signs in as Spotify's own client), \
                   and Spotify may refuse it. Log in in a browser, then paste back the URL of the \
                   page you land on (it won't load)."
                .into(),
        }]
    }

    async fn connections(&self) -> Vec<ConnectionInfo> {
        let (health, account) = match (self.audio(), self.refusal()) {
            (Some(audio), refused) => {
                let account = Some(Account {
                    service: Service::Spotify,
                    user_id: audio.username(),
                    username: Some(audio.username()),
                    attributes: std::collections::BTreeMap::new(),
                });
                match refused {
                    Some(why) => (Health::Degraded { why }, account),
                    None => (Health::Ok, account),
                }
            }
            (None, _) => match self
                .restore_failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
            {
                Some(why) => (Health::Failing { why }, None),
                None => (Health::NeedsLogin, None),
            },
        };
        vec![ConnectionInfo {
            id: LIBRESPOT.into(),
            service: Service::Spotify,
            label: "Spotify audio (librespot)".into(),
            grants: GRANTS,
            verified: None,
            health,
            account,
        }]
    }

    async fn begin(&self, method: &str) -> Result<LoginFlow> {
        Self::check_method(method)?;
        let verifier = random_token();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_token();
        let url = format!(
            "{AUTHORIZE_URL}?response_type=code&client_id={CLIENT_ID}&redirect_uri={}\
             &code_challenge_method=S256&code_challenge={challenge}&scope=streaming&state={state}",
            encode(REDIRECT_URI)
        );
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Pending { verifier, state });
        Ok(LoginFlow::Browser { url })
    }

    async fn complete(&self, method: &str, input: Option<String>) -> Result<LoginStatus> {
        Self::check_method(method)?;
        let redirect = input.ok_or_else(|| {
            Error::Auth("finish the browser login with the URL you landed on".into())
        })?;
        let token = self.exchange(&redirect).await?;
        let audio = SpotifyAudio::with_access_token(&self.dir, token).await?;
        tracing::info!("spotify audio signed in as {}", audio.username());
        *self.audio.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(audio));
        *self.refused.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .restore_failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        Ok(LoginStatus::Authorized)
    }

    async fn disconnect(&self, method: &str) -> Result<()> {
        Self::check_method(method)?;
        *self.audio.write().unwrap_or_else(PoisonError::into_inner) = None;
        *self.refused.lock().unwrap_or_else(PoisonError::into_inner) = None;
        match tokio::fs::remove_file(self.dir.join("credentials.json")).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn grants(&self, capability: Capability) -> bool {
        capability == Capability::Stream && self.audio().is_some() && self.refusal().is_none()
    }

    fn source(&self, need: Capability) -> Option<Arc<dyn Source>> {
        if !self.grants(need) {
            return None;
        }
        let audio = self.audio()?;
        Some(Arc::new(Observed {
            audio,
            refused: Arc::clone(&self.refused),
        }))
    }

    fn catalog(&self) -> Option<Arc<dyn Catalog>> {
        None
    }

    fn hint(&self, capability: Capability) -> String {
        match (capability, self.refusal()) {
            (Capability::Stream, Some(why)) => format!(
                "Spotify refused audio for the librespot login ({LIBRESPOT}): {why}. Sign in \
                 again with `connect {LIBRESPOT}`"
            ),
            (Capability::Stream, None) => {
                format!("sign in with the librespot login ({LIBRESPOT}; Spotify Premium)")
            }
            _ => "the librespot login only plays audio; browse with spotify.web".into(),
        }
    }
}

/// The librespot source, reporting what playback shows: a refused audio key takes streaming
/// away from the login until it plays again or signs in anew.
struct Observed {
    audio: Arc<SpotifyAudio>,
    refused: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl Source for Observed {
    fn service(&self) -> Service {
        Service::Spotify
    }

    async fn open(
        &self,
        source: &SourceRef,
        quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream> {
        let opened = self.audio.open(source, quality, start).await;
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        match &opened {
            Ok(_) => *refused = None,
            Err(Error::Auth(why)) => {
                tracing::warn!("spotify refused audio: {why}");
                *refused = Some(why.clone());
            }
            Err(_) => {}
        }
        opened
    }

    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        self.audio.describe(source).await
    }
}

/// 32 random bytes, URL-safe: a PKCE verifier, or a state.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("the OS has randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Percent-encoding for a URL component.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The `code` and `state` query parameters of the URL a login landed on.
fn code_and_state(redirect: &str) -> Result<(String, Option<String>)> {
    let query = redirect
        .split_once('?')
        .map_or(redirect, |(_, query)| query)
        .split('#')
        .next()
        .unwrap_or_default();
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", value)) => code = Some(value.to_string()),
            Some(("state", value)) => state = Some(value.to_string()),
            Some(("error", value)) => {
                return Err(Error::Auth(format!("spotify declined the login: {value}")));
            }
            _ => {}
        }
    }
    let code = code.ok_or_else(|| Error::Auth("that URL has no ?code= in it".into()))?;
    Ok((code, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("canon-librespot-{}", random_token()))
    }

    #[test]
    fn the_landing_url_gives_up_its_code_and_state() {
        let (code, state) =
            code_and_state("http://127.0.0.1:5588/login?code=AQB-x_1&state=s1").unwrap();
        assert_eq!((code.as_str(), state.as_deref()), ("AQB-x_1", Some("s1")));
        assert!(code_and_state("http://127.0.0.1:5588/login?error=access_denied").is_err());
        assert!(code_and_state("http://127.0.0.1:5588/login").is_err());
    }

    /// Signed out, it grants nothing and says how to sign in; a login URL carries PKCE, the
    /// streaming scope and the redirect librespot's client allows.
    #[tokio::test]
    async fn signed_out_it_offers_a_browser_login() {
        let connector = LibrespotConnector::restore(&scratch()).await;
        assert!(!connector.grants(Capability::Stream));
        assert!(connector.source(Capability::Stream).is_none());
        assert!(connector.hint(Capability::Stream).contains(LIBRESPOT));
        let info = connector.connections().await;
        assert_eq!(info[0].health, Health::NeedsLogin);

        let LoginFlow::Browser { url } = connector.begin(LIBRESPOT).await.unwrap() else {
            panic!("a browser login");
        };
        assert!(url.starts_with(AUTHORIZE_URL), "{url}");
        assert!(url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(url.contains("scope=streaming"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5588%2Flogin"));

        let wrong_state = connector
            .complete(
                LIBRESPOT,
                Some("http://127.0.0.1:5588/login?code=c&state=nope".into()),
            )
            .await
            .unwrap_err();
        assert!(
            wrong_state.to_string().contains("different login"),
            "{wrong_state}"
        );
        assert!(connector.begin("spotify.web").await.is_err());
    }
}
