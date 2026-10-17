//! The audio endpoint a player fetches a track from (yak canon-040e).
//!
//! `GET /audio/<entity id>?position_ms=<ms>` answers with the track as `audio/flac` at its own
//! bit depth and sample rate, starting at (or just before) the position asked for. A seek is
//! another request from the new position: no byte ranges, as for a renderer.
//!
//! This crate owns the HTTP side: the route, the headers, the status codes, and where a request
//! is authorized ([`authorize`]). Producing the audio does not live here — decoding and FLAC
//! encoding belong to crates this one must not depend on — so the server supplies an
//! [`AudioSource`], which the daemon implements.
//!
//! ## Where the audio starts
//!
//! A source may start before the position asked for (Tidal starts at the segment that holds it),
//! and the stream is not trimmed to make up the difference. [`START_MS_HEADER`] says where its
//! first sample really sits in track time, and a player reads position against that: the same
//! rule as for a renderer's stream (`canon-core`'s `position.rs`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use canon_core::{EntityId, Error};
use serde::Deserialize;

use crate::auth::Access;
use crate::server::AppState;

/// The response header that says where the stream's first sample sits in the track, in
/// milliseconds. It can be earlier than the `position_ms` asked for.
pub const START_MS_HEADER: &str = "x-canon-start-ms";

/// A future that borrows its source, boxed so [`AudioSource`] stays object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where a track's audio comes from: the server's side of the endpoint.
pub trait AudioSource: Send + Sync {
    /// Open library track `track` for streaming from about `position`. The stream is ready, with
    /// its start known, when this returns; a failure after that ends the body with an error
    /// rather than a clean end, so a player can tell a track that ended from one that broke.
    fn open(
        &self,
        track: EntityId,
        position: Duration,
    ) -> BoxFuture<'_, canon_core::Result<AudioStream>>;
}

/// An opened track: FLAC, and where in the track it starts.
pub struct AudioStream {
    /// Where the first sample sits in track time, in milliseconds.
    pub start_ms: u64,
    /// The FLAC stream: its header, then frames to the end of the track.
    pub body: Body,
}

/// What a request for audio carries besides the track.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AudioQuery {
    /// Where in the track to start, in milliseconds. The start of the track when absent.
    position_ms: Option<u64>,
    /// The short-lived token an audio URL carries (canon-65d2).
    token: Option<String>,
}

/// Whether a request for `track`, carrying `token` in its query, may have the audio: the one
/// place audio requests are authorized (canon-65d2).
///
/// A token must be one this server signed, for this track, unexpired. Without one, only an
/// [`Access::Open`] server (bound to loopback) serves the audio, as it serves everything else.
fn authorize(state: &AppState, track: EntityId, token: Option<&str>) -> Result<(), StatusCode> {
    match (token, &state.signer) {
        (Some(token), Some(signer)) => match signer.verify(token) {
            Ok(claim) if claim.track == track => Ok(()),
            Ok(_) | Err(_) => Err(StatusCode::FORBIDDEN),
        },
        (Some(_), None) => Err(StatusCode::FORBIDDEN),
        (None, _) if state.access() == Access::Open => Ok(()),
        (None, _) => Err(StatusCode::UNAUTHORIZED),
    }
}

/// `GET /audio/{id}`.
pub(crate) async fn handler(
    State(state): State<Arc<AppState>>,
    Path(track): Path<EntityId>,
    Query(query): Query<AudioQuery>,
) -> Response {
    if let Err(status) = authorize(&state, track, query.token.as_deref()) {
        return status.into_response();
    }
    let Some(audio) = state.audio.as_ref() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "this server has no audio to stream",
        )
            .into_response();
    };
    let position = Duration::from_millis(query.position_ms.unwrap_or(0));
    match audio.open(track, position).await {
        Ok(stream) => Response::builder()
            .header(header::CONTENT_TYPE, "audio/flac")
            .header(header::CACHE_CONTROL, "no-store")
            .header(
                HeaderName::from_static(START_MS_HEADER),
                HeaderValue::from(stream.start_ms),
            )
            .body(stream.body)
            .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()),
        Err(e) => {
            tracing::warn!(%track, "audio request failed: {e}");
            (status_of(&e), e.to_string()).into_response()
        }
    }
}

/// The status a failure to open a track answers with.
fn status_of(error: &Error) -> StatusCode {
    match error {
        Error::NotFound(_) => StatusCode::NOT_FOUND,
        Error::NotEntitled { .. } | Error::Auth(_) => StatusCode::FORBIDDEN,
        Error::Transient(_) => StatusCode::SERVICE_UNAVAILABLE,
        Error::Unsupported(_) => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::BAD_GATEWAY,
    }
}
