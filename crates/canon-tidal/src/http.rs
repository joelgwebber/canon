//! The swappable HTTP seam for Tidal (yak canon-a880).
//!
//! Tidal's anti-abuse layer blocks default TLS/HTTP2 stacks by JA3/JA4 fingerprint.
//! tideway (the Python predecessor) worked around this with `curl-cffi` impersonating
//! Chrome-on-Android. This module is the Rust equivalent seam: a **thin trait**
//! ([`TidalHttp`]) covering only the HTTP surface the Tidal flows need, plus a concrete
//! implementation ([`WreqHttp`]) backed by the pure-Rust browser-impersonation client
//! [`wreq`] + its [`wreq_util`] emulation presets.
//!
//! The point of the trait is that the impersonation backend is **swappable**: if the
//! pure-Rust path ever regresses against Tidal, a `curl-impersonate` C-FFI implementation
//! can be dropped in behind the same trait without touching auth/stream/realtime code.
//!
//! Everything above this seam speaks in plain `&str` URLs, `(name, value)` header pairs,
//! and [`HttpResponse`] (status + raw body, with a JSON helper). Nothing above the seam
//! sees a `wreq` type, which is exactly what keeps the backend replaceable.

use async_trait::async_trait;
use canon_core::{Error, Result};
use serde::de::DeserializeOwned;
use wreq::Client;
use wreq_util::{Emulation, EmulationOS, EmulationOption};

/// A minimal HTTP response: the status code plus the raw body bytes.
///
/// Deliberately backend-agnostic — it holds owned bytes rather than a streaming body so
/// the seam stays object-safe and the concrete client type never leaks upward. The JSON
/// helper ([`HttpResponse::json`]) is the "JSON helper" half of the seam's contract.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 401, 403).
    pub status: u16,
    /// Raw response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// `true` for 2xx status codes.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Decode the body as UTF-8 text.
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| Error::Source(format!("tidal http: non-utf8 body: {e}")))
    }

    /// Decode the body as JSON into `T`. The seam's JSON helper.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| Error::Source(format!("tidal http: json decode: {e}")))
    }
}

/// The thin async HTTP surface the Tidal flows require.
///
/// Kept intentionally small: a `GET` and a form `POST` (device-code OAuth and most Tidal
/// endpoints are either GETs or `application/x-www-form-urlencoded` POSTs). JSON decoding
/// lives on [`HttpResponse`] so this trait stays object-safe (`Box<dyn TidalHttp>`).
#[async_trait]
pub trait TidalHttp: Send + Sync {
    /// Issue a GET with the given extra headers.
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse>;

    /// Issue a `application/x-www-form-urlencoded` POST with the given form fields and
    /// extra headers.
    async fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse>;
}

/// The pure-Rust browser-impersonation backend for [`TidalHttp`].
///
/// Wraps a [`wreq::Client`] configured with a [`wreq_util`] emulation preset so the TLS
/// ClientHello (JA3/JA4) and HTTP/2 settings match a real browser rather than a default
/// rustls/hyper stack. Construct with [`WreqHttp::chrome_android`] to mirror tideway's
/// Chrome-on-Android profile, or [`WreqHttp::with_emulation`] for any other preset.
#[derive(Clone)]
pub struct WreqHttp {
    client: Client,
    /// Kept for diagnostics (`/api/realtime/status`-style reporting): which emulation
    /// profile this client presents.
    profile: &'static str,
}

impl WreqHttp {
    /// Build a client impersonating **Chrome-on-Android** — the profile tideway proved
    /// works against Tidal's anti-abuse layer.
    pub fn chrome_android() -> Result<Self> {
        // `Emulation::Chrome136` picks the browser ClientHello + HTTP/2 profile;
        // `EmulationOS::Android` sets the mobile platform (UA-CH `sec-ch-ua-platform`,
        // mobile flag, etc.). Together they reproduce a Chrome-on-Android fingerprint.
        let option = EmulationOption::builder()
            .emulation(Emulation::Chrome136)
            .emulation_os(EmulationOS::Android)
            .build();
        Self::build(option, "chrome136-android")
    }

    /// Build a client with an arbitrary emulation factory (e.g. `Emulation::OkHttp5`,
    /// `Emulation::Firefox139`, or a hand-rolled [`EmulationOption`]). This is the seam
    /// point for swapping fingerprints without touching callers.
    pub fn with_emulation(
        factory: impl wreq::EmulationProviderFactory,
        profile: &'static str,
    ) -> Result<Self> {
        Self::build(factory, profile)
    }

    fn build(factory: impl wreq::EmulationProviderFactory, profile: &'static str) -> Result<Self> {
        let client = Client::builder()
            .emulation(factory)
            .build()
            .map_err(|e| Error::Source(format!("tidal http: build wreq client: {e}")))?;
        Ok(Self { client, profile })
    }

    /// Which emulation profile this client presents (for diagnostics).
    pub fn profile(&self) -> &'static str {
        self.profile
    }
}

#[async_trait]
impl TidalHttp for WreqHttp {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Transient(format!("tidal http GET {url}: {e}")))?;
        into_response(resp).await
    }

    async fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse> {
        let mut req = self.client.post(url).form(form);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Transient(format!("tidal http POST {url}: {e}")))?;
        into_response(resp).await
    }
}

/// Collapse a `wreq::Response` into the backend-agnostic [`HttpResponse`]. Network/read
/// failures map to [`Error::Transient`] — the retryable class the player already knows.
async fn into_response(resp: wreq::Response) -> Result<HttpResponse> {
    let status = resp.status().as_u16();
    let body = resp
        .bytes()
        .await
        .map_err(|e| Error::Transient(format!("tidal http: read body: {e}")))?
        .to_vec();
    Ok(HttpResponse { status, body })
}
