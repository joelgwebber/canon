//! The HTTP seam: the little of HTTP the Spotify flows need, behind a trait so tests can script
//! replies without a network.
//!
//! Spotify, unlike Tidal, doesn't fingerprint TLS clients, so the one real implementation is plain
//! [`reqwest`] over rustls.

use std::time::Duration;

use async_trait::async_trait;
use canon_core::{Error, Result};
use serde::de::DeserializeOwned;

/// A response: status, the one header canon reads, and the whole body.
#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    /// `Retry-After` on a 429, in seconds.
    pub retry_after: Option<Duration>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// A response with this status and body, and no `Retry-After`.
    #[must_use]
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            retry_after: None,
            body: body.into(),
        }
    }

    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, lossily: for error messages.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body decoded as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| Error::Source(format!("spotify: unexpected reply: {e}")))
    }
}

/// The HTTP surface the Spotify flows use: authenticated GETs and form POSTs to the token endpoint.
#[async_trait]
pub trait SpotifyHttp: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse>;

    /// An `application/x-www-form-urlencoded` POST.
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse>;
}

/// [`SpotifyHttp`] over [`reqwest`] with rustls.
#[derive(Clone)]
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    /// A client with a request timeout, so a stalled connection can't hang a catalog call.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("canon/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Source(format!("spotify http: build client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl SpotifyHttp for ReqwestHttp {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        let mut request = self.client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| Error::Transient(format!("spotify GET {url}: {e}")))?;
        into_response(response).await
    }

    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
        let response = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|e| Error::Transient(format!("spotify POST {url}: {e}")))?;
        into_response(response).await
    }
}

async fn into_response(response: reqwest::Response) -> Result<HttpResponse> {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response
        .bytes()
        .await
        .map_err(|e| Error::Transient(format!("spotify: read body: {e}")))?
        .to_vec();
    Ok(HttpResponse {
        status,
        retry_after,
        body,
    })
}
