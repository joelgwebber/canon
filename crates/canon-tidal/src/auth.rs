//! Tidal device-code OAuth flow (yak canon-8bab).
//!
//! These are the stateless flow steps over the [`TidalHttp`] seam; [`crate::session`]
//! drives them into a live, stateful login and owns the token pair. The wire types here
//! are confirmed against the real endpoints (notably the camelCase
//! [`DeviceAuthorization`]), and the **rotating refresh token** is captured on every
//! refresh via [`Tokens::apply`] — the single choke point against silent logout.
//!
//! ## The flow
//!
//! 1. `POST auth.tidal.com/v1/oauth2/device_authorization` with the device-code client id
//!    → [`DeviceAuthorization`] (a `device_code`, a short `user_code`, a
//!    `verification_uri` the user visits, and a poll `interval`).
//! 2. Poll `POST auth.tidal.com/v1/oauth2/token` with
//!    `grant_type=urn:ietf:params:oauth:grant-type:device_code` until the user approves.
//!    While pending, Tidal returns `400` with `error = "authorization_pending"`; on
//!    success it returns a [`TokenResponse`].
//! 3. Later, refresh with `grant_type=refresh_token`. **Tidal may rotate the refresh
//!    token** — [`Tokens::apply`] captures the new one when present and otherwise keeps
//!    the previous value. Dropping a rotated token is the classic "silently logged out
//!    after a while" bug, so it is modelled explicitly here.
//!
//! Every network step goes through [`TidalHttp`], so the impersonation backend stays
//! swappable and this flow never names a concrete HTTP client.

use serde::{Deserialize, Serialize};

use canon_core::{Error, Result};

use crate::http::TidalHttp;

/// The device-code client id tideway uses (public identifier, not a secret).
pub const DEVICE_CLIENT_ID: &str = "zU4XHVVkc2tDPo4t";

/// Auth host: device authorization + token endpoints live here.
pub const AUTH_BASE: &str = "https://auth.tidal.com/v1/oauth2";
/// API host: authenticated resource calls (sessions, playback info) live here.
pub const API_BASE: &str = "https://api.tidal.com";

/// OAuth grant string for the device-code polling step.
pub const GRANT_DEVICE_CODE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// OAuth grant string for refreshing an access token.
pub const GRANT_REFRESH_TOKEN: &str = "refresh_token";

// ---------------------------------------------------------------------------
// Step 1 — device authorization
// ---------------------------------------------------------------------------

/// Response from `POST /device_authorization`.
///
/// Tidal returns this object in **camelCase** (`deviceCode`, `userCode`, …), unlike its
/// snake_case token endpoint — confirmed against the live endpoint. The `rename_all`
/// keeps the Rust fields idiomatic while matching the wire exactly.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthorization {
    /// Opaque code the client polls the token endpoint with.
    pub device_code: String,
    /// Short human-typed code shown to the user.
    pub user_code: String,
    /// Where the user enters `user_code` (e.g. `link.tidal.com`).
    pub verification_uri: String,
    /// Pre-filled verification URL, when Tidal supplies one.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Seconds until `device_code` expires.
    pub expires_in: u64,
    /// Minimum seconds between token polls.
    pub interval: u64,
}

// ---------------------------------------------------------------------------
// Step 2/3 — token endpoint
// ---------------------------------------------------------------------------

/// Successful response from `POST /token` (both device-code and refresh grants).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Present on the initial grant and whenever Tidal rotates it on refresh. Absent
    /// refresh responses mean "keep the existing refresh token".
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
    /// Tidal returns the numeric user id alongside tokens.
    #[serde(default)]
    pub user_id: Option<i64>,
}

/// Error body from `POST /token`. While the user has not yet approved, `error` is
/// `"authorization_pending"`; `"slow_down"` asks the client to widen its poll interval.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenError {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Outcome of a single token poll while the device authorization is outstanding.
#[derive(Debug)]
pub enum PollOutcome {
    /// The user approved; tokens issued.
    Authorized(TokenResponse),
    /// Still waiting on the user — poll again after `interval`.
    Pending,
    /// Tidal asked us to slow down; increase the interval and poll again.
    SlowDown,
}

/// The token pair the rest of canon-tidal carries. `apply` is where rotation is honoured.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user_id: Option<i64>,
}

impl Tokens {
    /// Fold a fresh [`TokenResponse`] into the current tokens, **capturing a rotated
    /// refresh token** when the response carries one and otherwise preserving the
    /// existing one. This is the single choke point that prevents silent logout.
    pub fn apply(&mut self, resp: &TokenResponse) {
        self.access_token = resp.access_token.clone();
        if let Some(rotated) = resp.refresh_token.as_ref() {
            self.refresh_token = rotated.clone();
        }
        self.expires_in = resp.expires_in;
        if resp.user_id.is_some() {
            self.user_id = resp.user_id;
        }
    }

    /// Build the initial token pair from the device-code grant. Errors if Tidal somehow
    /// returned no refresh token on the initial grant (would make the session
    /// unrefreshable — an auth-class failure, not transient).
    pub fn from_initial(resp: &TokenResponse) -> Result<Self> {
        let refresh_token = resp
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Auth("initial token grant had no refresh_token".into()))?;
        Ok(Self {
            access_token: resp.access_token.clone(),
            refresh_token,
            expires_in: resp.expires_in,
            user_id: resp.user_id,
        })
    }
}

// ---------------------------------------------------------------------------
// Flow steps (over the TidalHttp seam)
// ---------------------------------------------------------------------------

/// Step 1: begin device authorization. Returns the code the user enters and the polling
/// parameters.
pub async fn start_device_authorization(
    http: &dyn TidalHttp,
    scope: &str,
) -> Result<DeviceAuthorization> {
    let url = format!("{AUTH_BASE}/device_authorization");
    let resp = http
        .post_form(
            &url,
            &[("client_id", DEVICE_CLIENT_ID), ("scope", scope)],
            &[],
        )
        .await?;
    if !resp.is_success() {
        return Err(Error::Auth(format!(
            "device_authorization failed: HTTP {} {}",
            resp.status,
            resp.text().unwrap_or_default()
        )));
    }
    resp.json()
}

/// Step 2: poll the token endpoint once. Interprets Tidal's `authorization_pending` /
/// `slow_down` sentinels so the caller can drive its own poll loop.
pub async fn poll_device_token(http: &dyn TidalHttp, device_code: &str) -> Result<PollOutcome> {
    let url = format!("{AUTH_BASE}/token");
    let resp = http
        .post_form(
            &url,
            &[
                ("client_id", DEVICE_CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", GRANT_DEVICE_CODE),
            ],
            &[],
        )
        .await?;

    if resp.is_success() {
        return Ok(PollOutcome::Authorized(resp.json()?));
    }

    // Non-2xx: distinguish the expected "keep polling" sentinels from real failures.
    let err: TokenError = resp.json().unwrap_or(TokenError {
        error: "unknown".into(),
        error_description: None,
    });
    match err.error.as_str() {
        "authorization_pending" => Ok(PollOutcome::Pending),
        "slow_down" => Ok(PollOutcome::SlowDown),
        other => Err(Error::Auth(format!(
            "device token poll failed: {other} ({})",
            err.error_description.unwrap_or_default()
        ))),
    }
}

/// Step 3: refresh an access token. On success, callers fold the result through
/// [`Tokens::apply`] so any rotated refresh token is captured.
pub async fn refresh_token(http: &dyn TidalHttp, refresh_token: &str) -> Result<TokenResponse> {
    let url = format!("{AUTH_BASE}/token");
    let resp = http
        .post_form(
            &url,
            &[
                ("client_id", DEVICE_CLIENT_ID),
                ("refresh_token", refresh_token),
                ("grant_type", GRANT_REFRESH_TOKEN),
            ],
            &[],
        )
        .await?;
    if !resp.is_success() {
        // A rejected refresh token is permanent: the caller must re-run the device flow.
        return Err(Error::Auth(format!(
            "refresh failed: HTTP {} {}",
            resp.status,
            resp.text().unwrap_or_default()
        )));
    }
    resp.json()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Proves the serde shapes match Tidal's wire format (no network). The body is the
    // real camelCase response captured from the live device_authorization endpoint.
    #[test]
    fn device_authorization_deserializes() {
        let body = r#"{
            "deviceCode": "0ebf6d36-ddf6-4d5c-9173-3d3dab13a675",
            "expiresIn": 300,
            "interval": 2,
            "userCode": "AFLYP",
            "verificationUri": "link.tidal.com",
            "verificationUriComplete": "link.tidal.com/AFLYP"
        }"#;
        let da: DeviceAuthorization = serde_json::from_str(body).unwrap();
        assert_eq!(da.device_code, "0ebf6d36-ddf6-4d5c-9173-3d3dab13a675");
        assert_eq!(da.user_code, "AFLYP");
        assert_eq!(da.interval, 2);
        assert_eq!(
            da.verification_uri_complete.as_deref(),
            Some("link.tidal.com/AFLYP")
        );
    }

    #[test]
    fn token_response_optional_refresh() {
        // Refresh responses may omit refresh_token -> keep the old one.
        let body = r#"{"access_token":"at-2","expires_in":86400}"#;
        let tr: TokenResponse = serde_json::from_str(body).unwrap();
        assert!(tr.refresh_token.is_none());

        let mut tokens = Tokens {
            access_token: "at-1".into(),
            refresh_token: "rt-1".into(),
            expires_in: 3600,
            user_id: Some(42),
        };
        tokens.apply(&tr);
        assert_eq!(tokens.access_token, "at-2");
        assert_eq!(tokens.refresh_token, "rt-1"); // preserved
        assert_eq!(tokens.user_id, Some(42)); // preserved
    }

    #[test]
    fn rotating_refresh_token_is_captured() {
        let body =
            r#"{"access_token":"at-3","refresh_token":"rt-2","expires_in":86400,"user_id":42}"#;
        let tr: TokenResponse = serde_json::from_str(body).unwrap();
        let mut tokens = Tokens::from_initial(&TokenResponse {
            access_token: "at-1".into(),
            refresh_token: Some("rt-1".into()),
            token_type: Some("Bearer".into()),
            expires_in: 3600,
            user_id: Some(42),
        })
        .unwrap();
        tokens.apply(&tr);
        assert_eq!(tokens.refresh_token, "rt-2"); // rotated token captured
    }

    #[test]
    fn initial_grant_requires_refresh_token() {
        let no_refresh = TokenResponse {
            access_token: "at".into(),
            refresh_token: None,
            token_type: None,
            expires_in: 3600,
            user_id: None,
        };
        assert!(Tokens::from_initial(&no_refresh).is_err());
    }
}
