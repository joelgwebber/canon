//! Spotify's Authorization Code flow with PKCE: the stateless steps. [`crate::SpotifySession`]
//! drives them and owns the tokens.
//!
//! PKCE needs no client secret, which is what makes a user-registered app usable from a daemon:
//! the only thing canon is given is a client id. Per Spotify's PKCE tutorial
//! (developer.spotify.com/documentation/web-api/tutorials/code-pkce-flow):
//!
//! 1. The browser opens `accounts.spotify.com/authorize` with `code_challenge_method=S256` and the
//!    SHA-256 of a random verifier of 43 to 128 characters.
//! 2. Spotify redirects to the registered redirect URI with `code` and `state`.
//! 3. `POST accounts.spotify.com/api/token` with `grant_type=authorization_code`, the code, the
//!    same redirect URI, the client id and the verifier.
//! 4. Refresh is the same endpoint with `grant_type=refresh_token` and the client id. "A refresh
//!    token might not be included in each response. When a refresh token is not returned, continue
//!    using the existing token" (tutorials/refreshing-tokens).

use canon_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::http::SpotifyHttp;

pub const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Where Spotify sends the browser after consent. Spotify refuses `localhost` and requires an
/// explicit loopback literal, over plain HTTP (developer.spotify.com/documentation/web-api/
/// concepts/redirect_uri). Nothing needs to listen there: the browser shows a failed page whose
/// address the user pastes back. Register exactly this in the app's settings.
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8898/spotify/callback";

/// What canon asks for: reading and editing the saved library and playlists, followed artists
/// (for favorites), and the profile (for the account id that decides which playlists are ours).
pub const SCOPES: &[&str] = &[
    "user-library-read",
    "user-library-modify",
    "user-follow-read",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
    "user-read-private",
];

/// A login in flight: the verifier the code exchange must prove, the `state` the redirect must
/// echo, and the redirect URI the exchange must repeat. Serializable so the flow can span two
/// processes (print the URL now, paste the redirect later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLogin {
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

impl PendingLogin {
    /// Fresh material: a 64-character verifier (48 random bytes, base64url) and a random state.
    pub fn generate(redirect_uri: &str) -> Result<Self> {
        Ok(Self {
            verifier: random_token(48)?,
            state: random_token(16)?,
            redirect_uri: redirect_uri.to_string(),
        })
    }

    /// The S256 challenge: base64url(SHA-256(verifier)), unpadded.
    #[must_use]
    pub fn challenge(&self) -> String {
        challenge_for(&self.verifier)
    }

    /// The URL the user opens to consent.
    #[must_use]
    pub fn authorize_url(&self, client_id: &str) -> String {
        let scope = SCOPES.join(" ");
        format!(
            "{AUTHORIZE_URL}?response_type=code&client_id={}&scope={}&redirect_uri={}\
             &code_challenge_method=S256&code_challenge={}&state={}",
            encode_component(client_id),
            encode_component(&scope),
            encode_component(&self.redirect_uri),
            self.challenge(),
            encode_component(&self.state),
        )
    }
}

fn random_token(bytes: usize) -> Result<String> {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut buffer = vec![0u8; bytes];
    getrandom::getrandom(&mut buffer).map_err(|e| Error::Auth(format!("pkce: rng: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// RFC 7636 S256.
#[must_use]
pub fn challenge_for(verifier: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The authorization code in what the user pasted: the whole redirect URL, or just the code.
///
/// When a URL carries a `state`, it must be the one this login sent: anything else is a redirect
/// from some other attempt, and exchanging it would fail confusingly or, worse, succeed as someone
/// else's consent. A consent the user declined arrives as `error=access_denied`.
pub fn code_from(pasted: &str, expected_state: &str) -> Result<String> {
    let pasted = pasted.trim();
    let Some((_, query)) = pasted.split_once('?') else {
        if pasted.is_empty() || pasted.contains(['/', ':', ' ', '&', '=']) {
            return Err(Error::Auth(
                "paste the whole redirect URL, or the code from it".into(),
            ));
        }
        return Ok(pasted.to_string());
    };
    let query = query.split('#').next().unwrap_or(query);
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| decode_component(value))
        })
    };
    if let Some(error) = param("error") {
        return Err(Error::Auth(format!("spotify login refused: {error}")));
    }
    if let Some(state) = param("state")
        && state != expected_state
    {
        return Err(Error::Auth(
            "the redirect is from a different login attempt (state mismatch); start again".into(),
        ));
    }
    param("code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| Error::Auth("the redirect URL has no `code`".into()))
}

/// The token endpoint's reply to both grants.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Always on the first grant; on refresh only when Spotify rotates it.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds.
    pub expires_in: u64,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Step 3: trade the code for tokens.
pub async fn exchange_code(
    http: &dyn SpotifyHttp,
    client_id: &str,
    code: &str,
    pending: &PendingLogin,
) -> Result<TokenResponse> {
    let reply = http
        .post_form(
            TOKEN_URL,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &pending.redirect_uri),
                ("client_id", client_id),
                ("code_verifier", &pending.verifier),
            ],
        )
        .await?;
    token_reply("code exchange", &reply)
}

/// Step 4: a fresh access token.
pub async fn refresh(
    http: &dyn SpotifyHttp,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse> {
    let reply = http
        .post_form(
            TOKEN_URL,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
            ],
        )
        .await?;
    token_reply("refresh", &reply)
}

fn token_reply(step: &str, reply: &crate::http::HttpResponse) -> Result<TokenResponse> {
    if reply.is_success() {
        return reply.json();
    }
    if reply.status == 429 || reply.status >= 500 {
        return Err(Error::Transient(format!(
            "spotify {step}: HTTP {}",
            reply.status
        )));
    }
    // 400 `invalid_grant` is a revoked or expired (six months after consent) refresh token, or a
    // stale code: either way the user has to log in again.
    let detail = reply
        .json::<TokenError>()
        .map(|e| match e.error_description {
            Some(description) => format!("{}: {description}", e.error),
            None => e.error,
        })
        .unwrap_or_else(|_| reply.text());
    Err(Error::Auth(format!(
        "spotify {step} failed: HTTP {} {detail}",
        reply.status
    )))
}

/// Percent-encode a URL component: everything but the unreserved characters.
#[must_use]
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Undo percent-encoding (and `+` for space), leaving malformed escapes as they are.
fn decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 appendix B's worked example.
    #[test]
    fn the_challenge_is_rfc_7636_s256() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_verifier_is_long_enough_and_url_safe() {
        let pending = PendingLogin::generate(DEFAULT_REDIRECT_URI).unwrap();
        assert!((43..=128).contains(&pending.verifier.len()));
        assert!(
            pending
                .verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(
            pending.verifier,
            PendingLogin::generate(DEFAULT_REDIRECT_URI)
                .unwrap()
                .verifier
        );
    }

    #[test]
    fn the_authorize_url_carries_the_client_redirect_scopes_and_challenge() {
        let pending = PendingLogin {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: "st4te".into(),
            redirect_uri: DEFAULT_REDIRECT_URI.into(),
        };
        let url = pending.authorize_url("abc123");
        assert!(url.starts_with("https://accounts.spotify.com/authorize?"));
        for part in [
            "response_type=code",
            "client_id=abc123",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fspotify%2Fcallback",
            "code_challenge_method=S256",
            "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "state=st4te",
            "scope=user-library-read%20user-library-modify%20user-follow-read%20\
             playlist-read-private%20playlist-read-collaborative%20playlist-modify-private%20\
             playlist-modify-public%20user-read-private",
        ] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
    }

    #[test]
    fn the_code_comes_from_a_pasted_redirect_or_is_the_paste() {
        let url = "http://127.0.0.1:8898/spotify/callback?code=AQD%2Dx_y&state=st4te";
        assert_eq!(code_from(url, "st4te").unwrap(), "AQD-x_y");
        assert_eq!(code_from("  AQDxyz \n", "st4te").unwrap(), "AQDxyz");
        assert!(matches!(
            code_from(
                "http://127.0.0.1:8898/spotify/callback?code=AQD&state=other",
                "st4te"
            ),
            Err(Error::Auth(_))
        ));
        let refused = code_from(
            "http://127.0.0.1:8898/spotify/callback?error=access_denied&state=st4te",
            "st4te",
        )
        .unwrap_err();
        assert!(refused.to_string().contains("access_denied"), "{refused}");
        assert!(
            code_from(
                "http://127.0.0.1:8898/spotify/callback?state=st4te",
                "st4te"
            )
            .is_err()
        );
        assert!(code_from("http://127.0.0.1:8898/spotify/callback", "st4te").is_err());
    }

    #[test]
    fn components_round_trip() {
        assert_eq!(decode_component("a%20b+c%2F%zz%"), "a b c/%zz%");
        assert_eq!(
            decode_component(&encode_component("isrc:US 1/2")),
            "isrc:US 1/2"
        );
    }
}
