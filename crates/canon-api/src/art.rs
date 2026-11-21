//! The album-art endpoint (yak canon-5a2c): `GET /art/<entity id>?size=<px>`.
//!
//! A client names an entity it already has (a track's or an album's canon id), never a URL: the
//! daemon looks the artwork up itself, so it is not an open fetch proxy, and clients cache by
//! the path they asked for and never reach a CDN. The answer is a JPEG bounded by one of
//! [`SIZES`] on its long side.
//!
//! As for [`crate::audio`], this crate owns the HTTP side (the route, headers, status codes and
//! who is let in) and not the work: fetching and resizing need an HTTP client and an image
//! codec this crate must not link, so the server supplies an [`ArtSource`] the daemon implements.
//!
//! Who is let in is decided as a WebSocket connection is (`server::admit`): an
//! [`crate::Access::Open`] server serves anyone, a paired one wants a paired player's bearer token.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use canon_core::EntityId;
use serde::Deserialize;

use crate::audio::{BoxFuture, status_of};
use crate::server::{AppState, admit};

/// The sizes art is served at, in pixels on the long side. A request snaps up to the first that
/// holds it, so a cache holds at most this many files per picture.
pub const SIZES: [u32; 4] = [64, 160, 320, 640];

/// The size served when a request names none.
pub const DEFAULT_SIZE: u32 = 320;

/// How long a client may keep an answer: a picture at a size never changes (a changed artwork URL
/// is a different picture to the daemon's own cache).
const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// The size a request for `asked` pixels is served at: the smallest of [`SIZES`] that holds it,
/// the largest when none does, [`DEFAULT_SIZE`] when it names none.
#[must_use]
pub fn bucket(asked: Option<u32>) -> u32 {
    let Some(asked) = asked else {
        return DEFAULT_SIZE;
    };
    SIZES
        .into_iter()
        .find(|size| *size >= asked)
        .unwrap_or(SIZES[SIZES.len() - 1])
}

/// Where album art comes from: the server's side of the endpoint.
pub trait ArtSource: Send + Sync {
    /// The picture for library entity `entity` (a track or an album) as a JPEG bounded by
    /// `size` pixels on its long side, `size` being one of [`SIZES`].
    ///
    /// # Errors
    /// [`canon_core::Error::NotFound`] when the entity is unknown or has no art (404); anything
    /// else, a failed fetch included, answers 502 or its own status as for audio.
    fn art(&self, entity: EntityId, size: u32) -> BoxFuture<'_, canon_core::Result<Vec<u8>>>;
}

/// What a request for art carries besides the entity.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ArtQuery {
    /// The size wanted, in pixels; snapped up to one of [`SIZES`].
    size: Option<u32>,
}

/// `GET /art/{entity}`.
pub(crate) async fn handler(
    State(state): State<Arc<AppState>>,
    Path(entity): Path<EntityId>,
    Query(query): Query<ArtQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err((status, why)) = admit(&state, &headers, None) {
        return (status, why).into_response();
    }
    let Some(art) = state.art.as_ref() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "this server has no album art to serve",
        )
            .into_response();
    };
    match art.art(entity, bucket(query.size)).await {
        Ok(jpeg) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, CACHE_CONTROL),
            ],
            jpeg,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(%entity, "art request failed: {e}");
            (status_of(&e), e.to_string()).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_snaps_up_to_the_first_bucket_that_holds_it() {
        assert_eq!(bucket(None), 320, "absent is the default");
        assert_eq!(bucket(Some(0)), 64);
        assert_eq!(bucket(Some(64)), 64, "a bucket is its own");
        assert_eq!(bucket(Some(65)), 160);
        assert_eq!(bucket(Some(160)), 160);
        assert_eq!(bucket(Some(161)), 320);
        assert_eq!(bucket(Some(500)), 640);
        assert_eq!(bucket(Some(640)), 640);
        assert_eq!(bucket(Some(4000)), 640, "none holds it: the largest");
    }
}
