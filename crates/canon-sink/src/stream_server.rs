//! LAN FLAC streaming server (yak canon-21f7).
//!
//! This is the HTTP endpoint a network renderer (Chromecast first, DLNA later) pulls its
//! audio from: an unbounded, chunked `audio/flac` body served off the chosen LAN interface.
//!
//! ## The tideway lesson this encodes
//!
//! tideway streamed a *headerless live ring*: it cached nothing, and a consumer that joined
//! mid-stream — the common case being a Cast device that drops its HTTP connection and
//! reconnects a second later — landed in the middle of the FLAC frame sequence with no
//! `fLaC` magic, no STREAMINFO, no metadata. The decoder had nothing to initialise from and
//! played **silence** while the ring kept advancing. The fix is structural, not a retry: the
//! stream is split into two things that are cached and combined *differently*.
//!
//! * The **header** (FLAC magic + STREAMINFO + metadata blocks — everything a decoder needs
//!   before the first audio frame) is cached once and replayed to *every* subscriber, first,
//!   always — including on a reconnect. See [`StreamBroadcaster::set_header`].
//! * The **live edge** is a bounded broadcast ring. A subscriber joins at the *current* edge
//!   and gets only what is pushed from then on — never the whole history. See
//!   [`StreamBroadcaster::subscribe`].
//!
//! ## Backpressure is resync-by-disconnect, never a silent drop
//!
//! A renderer that falls behind the ring cannot be "caught up" by skipping chunks: a gap in
//! the middle of the FLAC frame stream is corrupt, and feeding it produces a glitch or a
//! decoder wedge — the very failure mode we are fixing. So when a consumer lags past the ring
//! capacity we **end its stream**. The renderer's HTTP connection closes, it reconnects, and
//! it replays the header and rejoins cleanly at the new edge. Reconnect-with-header-replay is
//! the *correct* resync; a mid-FLAC oldest-drop is not. See [`StreamBroadcaster::subscribe`].
//!
//! ## Two decoupled layers
//!
//! Layer 1 ([`StreamBroadcaster`]) is protocol-agnostic and holds all the hard join /
//! backpressure logic, so it is unit-testable without a socket or a renderer. Layer 2
//! ([`router`] / [`serve`] / [`spawn`]) is a thin axum shim that maps one subscriber stream
//! onto one HTTP response. PCM→FLAC *encoding* is out of scope here (it is wired in the Cast
//! yak); this module deals only in already-encoded header + frame [`Bytes`].

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use bytes::Bytes;
use canon_core::{Error, Result};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt};

/// The path the live FLAC stream is served on. Exported so the Cast/DLNA control layer can
/// build the media URL it hands the renderer without hardcoding the string twice.
pub const STREAM_PATH: &str = "/stream.flac";

/// Number of live chunks buffered before a stalled reader is force-resynced (see the module
/// docs on backpressure). Each chunk is one or more complete FLAC frames, so this is a small
/// multiple of the frame rate — enough to ride out a brief reader stall, small enough that a
/// truly wedged reader is disconnected promptly instead of accumulating latency.
const DEFAULT_CAPACITY: usize = 64;

/// Layer 1: the protocol-agnostic broadcast core.
///
/// Clone it freely — every clone shares the one header cache and the one live ring, so the
/// control layer can hold a producer handle while the axum state holds another. Producing is
/// [`set_header`](Self::set_header) (once) plus [`push`](Self::push) (per frame-aligned
/// chunk); consuming is [`subscribe`](Self::subscribe).
#[derive(Clone)]
pub struct StreamBroadcaster {
    /// The live edge. New receivers join at the current tail, so a late joiner never replays
    /// history — only the header (below) is replayed, and only by `subscribe`.
    tx: broadcast::Sender<Bytes>,
    /// The decoder-init blob, cached separately from live data and replayed to every
    /// subscriber first. `None` until `set_header` is called.
    header: Arc<Mutex<Option<Bytes>>>,
}

impl StreamBroadcaster {
    /// Create a broadcaster whose live ring buffers `capacity` chunks. `capacity` is clamped
    /// to at least 1 because `tokio::sync::broadcast` requires a non-zero capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Self {
            tx,
            header: Arc::new(Mutex::new(None)),
        }
    }

    /// Cache the header blob (FLAC magic + STREAMINFO + metadata). Everything a decoder needs
    /// before the first audio frame goes here, and every subscriber — new or reconnecting —
    /// receives it before any live chunk. Setting it again replaces the cached copy (e.g. a
    /// new track with a different STREAMINFO); in-flight subscribers keep the header they
    /// already received, which is correct for the frames they are mid-stream on.
    pub fn set_header(&self, header: Bytes) {
        *self.header.lock().expect("header mutex poisoned") = Some(header);
    }

    /// Push one frame-aligned payload onto the live edge. The caller guarantees each chunk is
    /// one or more *complete* FLAC frames (this layer never splits or inspects them). If no
    /// consumers are currently subscribed the chunk is simply dropped — there is no reader to
    /// fall behind and nothing to cache, since the live edge is not history.
    pub fn push(&self, chunk: Bytes) {
        // `send` errors only when there are zero receivers; that is not a failure here.
        let _ = self.tx.send(chunk);
    }

    /// Subscribe to the stream. The returned async stream **always emits the cached header
    /// first** (if one has been set), then live chunks from the *current* edge onward.
    ///
    /// This is the tideway fix: a consumer joining mid-stream — including a renderer that
    /// dropped and reopened its HTTP connection — gets header + live edge, never a headerless
    /// ring and never the whole backlog.
    ///
    /// On lag (a consumer falling past the ring capacity) the stream **ends** rather than
    /// skipping chunks: [`LiveEdge`] stops at the first
    /// [`RecvError::Lagged`](tokio::sync::broadcast::error::RecvError::Lagged), which drops the
    /// HTTP body and prompts the renderer to reconnect and replay the header. A closed channel
    /// (all producers gone) ends the stream normally.
    pub fn subscribe(&self) -> impl Stream<Item = Bytes> + Send + 'static {
        // Snapshot the header, then join the live edge. Header is chained *before* live, so it
        // is emitted first regardless of what arrives on the ring in between.
        let header = self.header.lock().expect("header mutex poisoned").clone();
        tokio_stream::iter(header).chain(LiveEdge::new(self.tx.subscribe()))
    }
}

/// The live half of a subscriber stream: a [`Stream`] over a broadcast [`Receiver`] that ends
/// on lag or close (see [`StreamBroadcaster::subscribe`] for why ending is the correct resync).
///
/// `tokio_stream`'s ready-made `BroadcastStream` wrapper is gated behind a crate feature this
/// build doesn't enable, so we drive the receiver directly. The `recv` future is boxed and
/// *owns* the receiver, handing it back on completion — that keeps the stream `Send` and
/// movable without a self-referential borrow and without pulling in `tokio-util`'s
/// `ReusableBoxFuture`.
///
/// [`Receiver`]: tokio::sync::broadcast::Receiver
struct LiveEdge {
    /// Boxed `recv` future that owns the receiver and returns it alongside the result, so the
    /// receiver survives across polls without a self-referential borrow.
    fut: Pin<Box<dyn Future<Output = (broadcast::Receiver<Bytes>, RecvResult)> + Send>>,
    /// Set once the channel lagged or closed; a terminated stream never polls again.
    done: bool,
}

type RecvResult = std::result::Result<Bytes, RecvError>;

/// Await one chunk while owning the receiver, then hand the receiver back for the next poll.
async fn recv_owned(
    mut rx: broadcast::Receiver<Bytes>,
) -> (broadcast::Receiver<Bytes>, RecvResult) {
    let res = rx.recv().await;
    (rx, res)
}

impl LiveEdge {
    fn new(rx: broadcast::Receiver<Bytes>) -> Self {
        Self {
            fut: Box::pin(recv_owned(rx)),
            done: false,
        }
    }
}

impl Stream for LiveEdge {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Bytes>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.fut.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready((rx, Ok(chunk))) => {
                self.fut = Box::pin(recv_owned(rx));
                Poll::Ready(Some(chunk))
            }
            // Lagged: resync-by-disconnect. Closed: producers gone. Both end the stream (the
            // receiver is dropped here); a lagged reader reconnects and replays the header.
            Poll::Ready((_, Err(RecvError::Lagged(_) | RecvError::Closed))) => {
                self.done = true;
                Poll::Ready(None)
            }
        }
    }
}

impl Default for StreamBroadcaster {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

/// Layer 2: build the axum [`Router`] that serves one broadcaster's live stream.
///
/// The renderer pulls [`STREAM_PATH`] and gets a chunked `audio/flac` body. The router is
/// state-complete (`Router<()>`), so it can be handed straight to [`axum::serve`] or exercised
/// with `tower`'s `oneshot` in tests.
pub fn router(broadcaster: StreamBroadcaster) -> Router {
    Router::new()
        .route(STREAM_PATH, get(stream_handler))
        .with_state(broadcaster)
}

/// Serve the live FLAC stream on `addr` until the server stops. `addr` must be a specific LAN
/// interface IP (never `0.0.0.0`): the renderer reaches us on exactly the interface discovery
/// chose, and we do not expose the stream on every interface. This call runs until the server
/// exits and is the blocking entry point; use [`spawn`] to run it in the background.
///
/// The response is HTTP/1.1: on a plain (non-TLS) [`TcpListener`], hyper serves HTTP/1.1 by
/// default and nothing here negotiates HTTP/2 (no ALPN, no `http2` opt-in), which is what
/// chunked-transfer renderers expect.
pub async fn serve(addr: SocketAddr, broadcaster: StreamBroadcaster) -> Result<()> {
    let listener = bind(addr).await?;
    axum::serve(listener, router(broadcaster))
        .await
        .map_err(|e| Error::Sink(format!("stream server exited: {e}")))
}

/// Bind `addr` and run the server on a background task. Returns the *actually bound*
/// [`SocketAddr`] (so callers may pass port 0 and learn the real port, as the tests do) plus
/// the task [`JoinHandle`]. Same interface and HTTP/1.1 rules as [`serve`].
pub async fn spawn(
    addr: SocketAddr,
    broadcaster: StreamBroadcaster,
) -> Result<(SocketAddr, JoinHandle<()>)> {
    let listener = bind(addr).await?;
    let bound = listener
        .local_addr()
        .map_err(|e| Error::Sink(format!("stream server local_addr: {e}")))?;
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router(broadcaster)).await {
            tracing::error!("canon-sink stream server exited: {e}");
        }
    });
    Ok((bound, handle))
}

async fn bind(addr: SocketAddr) -> Result<TcpListener> {
    TcpListener::bind(addr)
        .await
        .map_err(|e| Error::Sink(format!("stream server bind {addr}: {e}")))
}

/// The one route handler. Serves the full live stream as `200 OK` for every request.
async fn stream_handler(
    State(broadcaster): State<StreamBroadcaster>,
    headers: HeaderMap,
) -> Response {
    // Range handling is deliberately quarantined: `206 Partial Content` is only ever reachable
    // from inside this guard, because answering 206 to a request that carried no `Range` header
    // confuses renderers (a hard-won interop bug). This live stream is unbounded and
    // non-seekable, so even a `Range: bytes=0-` gets the whole stream as 200 — the guard is the
    // documented home for a real byte-range/206 implementation when DLNA seek lands (a later
    // yak), not something this stream ever emits today.
    if headers.contains_key(header::RANGE) {
        // Extension point (DLNA seek, later yak): construct `206 Partial Content` +
        // `Content-Range` here from the parsed range. Until then we intentionally fall through
        // to the full 200 stream.
        return full_stream_response(&broadcaster);
    }

    full_stream_response(&broadcaster)
}

/// Build the `200 OK`, `audio/flac`, chunked-body response for one subscriber. Each `Bytes`
/// from the subscriber stream is wrapped `Ok` (the stream itself is infallible; it just ends,
/// per the backpressure contract) so it fits [`Body::from_stream`].
fn full_stream_response(broadcaster: &StreamBroadcaster) -> Response {
    let stream = broadcaster.subscribe().map(Ok::<Bytes, Infallible>);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/flac")
        .body(Body::from_stream(stream))
        .expect("status + content-type build a valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    const HDR: &[u8] = b"fLaC\x00\x00\x00\x22STREAMINFO...";

    fn hdr() -> Bytes {
        Bytes::from_static(HDR)
    }

    // --- Layer 1: the broadcast core -----------------------------------------------------

    /// Header-replay-on-join: a subscriber receives the cached header first, then live chunks.
    #[tokio::test]
    async fn header_is_replayed_before_live_chunks() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());

        let s = b.subscribe();
        b.push(Bytes::from_static(b"c1"));
        b.push(Bytes::from_static(b"c2"));
        drop(b); // close the channel so the stream terminates and `collect` is bounded

        let got: Vec<Bytes> = s.collect().await;
        assert_eq!(
            got,
            vec![hdr(), Bytes::from_static(b"c1"), Bytes::from_static(b"c2"),],
            "header must precede live chunks"
        );
    }

    /// Late joiner: a consumer that subscribes after streaming began gets header + the current
    /// edge only, not the backlog the earlier consumer already saw.
    #[tokio::test]
    async fn late_joiner_gets_header_plus_edge_not_history() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());

        let a = b.subscribe();
        b.push(Bytes::from_static(b"c1"));
        let late = b.subscribe(); // joins after c1
        b.push(Bytes::from_static(b"c2"));
        drop(b);

        let a_got: Vec<Bytes> = a.collect().await;
        let late_got: Vec<Bytes> = late.collect().await;

        assert_eq!(
            a_got,
            vec![hdr(), Bytes::from_static(b"c1"), Bytes::from_static(b"c2")],
            "the early subscriber sees the whole live run"
        );
        assert_eq!(
            late_got,
            vec![hdr(), Bytes::from_static(b"c2")],
            "the late joiner sees header + edge only (no c1)"
        );
    }

    /// Lag → resync-by-disconnect: overrun a slow consumer past the ring capacity and assert
    /// its stream *ends* after the header rather than yielding a gap in the frame sequence.
    #[tokio::test]
    async fn lag_ends_the_stream_instead_of_yielding_a_gap() {
        let b = StreamBroadcaster::new(2); // tiny ring
        b.set_header(hdr());

        let s = b.subscribe(); // never polled until collect: guaranteed to fall behind
        for i in 0..8u8 {
            b.push(Bytes::copy_from_slice(&[b'c', i]));
        }
        drop(b);

        let got: Vec<Bytes> = s.collect().await;
        assert_eq!(
            got,
            vec![hdr()],
            "a lagged consumer gets the header then the stream ends — never a mid-FLAC gap"
        );
    }

    // --- Layer 2: axum glue --------------------------------------------------------------

    /// GET returns 200 + `audio/flac`, and the first body bytes are exactly the header.
    #[tokio::test]
    async fn get_serves_200_flac_starting_with_the_header() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());

        let resp = router(b.clone())
            .oneshot(
                Request::builder()
                    .uri(STREAM_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "audio/flac"
        );

        // Drop the producer so the live stream closes and the body read is bounded. `oneshot`
        // has already consumed and dropped the router's state clone, so this is the last
        // sender.
        let body = resp.into_body();
        drop(b);

        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        assert_eq!(
            bytes,
            hdr(),
            "the stream opens with the decoder-init header"
        );
    }

    /// A request with no `Range` header must never draw a `206`. It gets the full 200 stream.
    #[tokio::test]
    async fn no_range_header_never_yields_206() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());

        let resp = router(b)
            .oneshot(
                Request::builder()
                    .uri(STREAM_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_ne!(resp.status(), StatusCode::PARTIAL_CONTENT);
    }

    /// Even a `Range` request gets the full stream as 200 for this live/unbounded body — the
    /// 206 branch is an extension point, not something the live stream emits.
    #[tokio::test]
    async fn range_request_still_gets_full_200_stream() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());

        let resp = router(b)
            .oneshot(
                Request::builder()
                    .uri(STREAM_PATH)
                    .header(header::RANGE, "bytes=0-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_ne!(resp.status(), StatusCode::PARTIAL_CONTENT);
    }

    /// `spawn` binds the requested interface and reports the real port (port 0 → assigned).
    #[tokio::test]
    async fn spawn_binds_and_reports_the_real_port() {
        let b = StreamBroadcaster::new(16);
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (bound, handle) = spawn(addr, b).await.unwrap();

        assert_eq!(bound.ip(), addr.ip(), "bound the requested interface");
        assert_ne!(bound.port(), 0, "an ephemeral port was actually assigned");

        handle.abort();
    }
}
