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
//! ## One stream per load, and it ends
//!
//! Each track (each load on the renderer) gets its own path, `/stream/<n>.flac`, and its own
//! broadcaster, which is [`finish`](StreamBroadcaster::finish)ed once the track has been fed in
//! full. The body then *ends*, and the renderer reaches a real end of stream: that is the only
//! way it can ever report the track finished, and so the only way the queue auto-advances on a
//! network output. (One endless stream per session — the first design — never ended, and the
//! renderer just starved silently at the end of every track.) Distinct paths are also what lets
//! the next track be addressable while the current one plays, which gapless hand-off needs.
//! [`StreamRoutes`] is the per-session registry of those paths.
//!
//! ## Two decoupled layers
//!
//! Layer 1 ([`StreamBroadcaster`]) is protocol-agnostic and holds all the hard join /
//! backpressure logic, so it is unit-testable without a socket or a renderer. Layer 2
//! ([`router`] / [`serve`] / [`spawn`]) is a thin axum shim that maps one subscriber stream
//! onto one HTTP response. PCM→FLAC *encoding* is out of scope here (it is wired in the Cast
//! yak); this module deals only in already-encoded header + frame [`Bytes`].

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
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

/// How many recent streams a session keeps addressable: the current one, and the one before it
/// (still draining to a renderer that has not fetched the next yet).
const KEEP_STREAMS: usize = 2;

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
    /// Set by [`finish`](Self::finish): the track has been fed in full.
    finished: Arc<AtomicBool>,
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
            finished: Arc::new(AtomicBool::new(false)),
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

    /// End the stream: the track has been fed in full. Every subscriber's body ends after the
    /// chunks already pushed, so the renderer plays to the end and reports the track finished; a
    /// subscriber arriving afterwards gets the header and an immediate end. Idempotent.
    pub fn finish(&self) {
        if !self.finished.swap(true, Ordering::AcqRel) {
            // An empty chunk is the end marker on the live edge (a real chunk is never empty).
            let _ = self.tx.send(Bytes::new());
        }
    }

    /// How many consumers are currently pulling the stream.
    ///
    /// This is **ground truth for whether our audio is actually reaching a renderer**, and it is
    /// protocol-agnostic: it counts bytes being consumed, not what a control protocol claims. That
    /// matters because a multi-protocol speaker can be taken over by something the control channel
    /// cannot see at all — observed live, a KEF playing our Cast stream was grabbed over Spotify
    /// Connect while every Cast status poll still reported our own session happily `Playing`. The
    /// receiver's HTTP connection dropping is what actually reveals that (yak canon-2dbf).
    #[must_use]
    pub fn consumers(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Subscribe to the stream. The returned async stream **always emits the cached header
    /// first** (if one has been set), then live chunks from the *current* edge onward.
    ///
    /// This is the tideway fix: a consumer joining mid-stream — including a renderer that
    /// dropped and reopened its HTTP connection — gets header + live edge, never a headerless
    /// ring and never the whole backlog.
    ///
    /// Each live subscriber holds a broadcast receiver, so [`consumers`](Self::consumers) counts
    /// exactly the renderers currently pulling the stream.
    ///
    /// On lag (a consumer falling past the ring capacity) the stream **ends** rather than
    /// skipping chunks: [`LiveEdge`] stops at the first
    /// [`RecvError::Lagged`](tokio::sync::broadcast::error::RecvError::Lagged), which drops the
    /// HTTP body and prompts the renderer to reconnect and replay the header. [`finish`] and a
    /// closed channel (all producers gone) end the stream normally.
    ///
    /// [`finish`]: Self::finish
    pub fn subscribe(&self) -> impl Stream<Item = Bytes> + Send + 'static {
        // Join the live edge *before* looking at `finished`: a finish that lands after this is
        // seen as the end marker on the ring, and one that landed before is seen in the flag, so
        // no subscriber can miss the end and wait forever.
        let mut live = LiveEdge::new(self.tx.subscribe());
        live.done = self.finished.load(Ordering::Acquire);
        // Snapshot the header; it is chained *before* live, so it is emitted first regardless of
        // what arrives on the ring in between.
        let header = self.header.lock().expect("header mutex poisoned").clone();
        tokio_stream::iter(header).chain(live)
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
            // The end marker: the track was fed in full (see `StreamBroadcaster::finish`).
            Poll::Ready((_, Ok(chunk))) if chunk.is_empty() => {
                self.done = true;
                Poll::Ready(None)
            }
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

/// One network session's streams: a fresh, numbered stream per load, of which the most recent
/// [`KEEP_STREAMS`] stay addressable. Clone it freely; clones share the registry.
#[derive(Clone, Default)]
pub struct StreamRoutes {
    inner: Arc<Mutex<Routes>>,
}

#[derive(Default)]
struct Routes {
    next: u64,
    streams: VecDeque<(u64, StreamBroadcaster)>,
}

impl StreamRoutes {
    /// Open the stream for the next load. Returns the path to hand the renderer (relative to the
    /// server's address) and the broadcaster to feed. Streams older than the previous one are
    /// retired, and their bodies ended.
    pub fn open(&self) -> (String, StreamBroadcaster) {
        let mut routes = self.inner.lock().expect("routes mutex poisoned");
        routes.next += 1;
        let id = routes.next;
        let broadcaster = StreamBroadcaster::default();
        routes.streams.push_back((id, broadcaster.clone()));
        while routes.streams.len() > KEEP_STREAMS {
            if let Some((_, retired)) = routes.streams.pop_front() {
                retired.finish();
            }
        }
        (format!("/stream/{id}.flac"), broadcaster)
    }

    fn get(&self, id: u64) -> Option<StreamBroadcaster> {
        let routes = self.inner.lock().expect("routes mutex poisoned");
        routes
            .streams
            .iter()
            .find(|(stream, _)| *stream == id)
            .map(|(_, broadcaster)| broadcaster.clone())
    }

    /// Whether the newest stream has been fed in full. The renderer is then expected to stop
    /// pulling — it has the whole track and is playing out its buffer — so an absence of
    /// consumers means nothing.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        let routes = self.inner.lock().expect("routes mutex poisoned");
        routes
            .streams
            .back()
            .is_some_and(|(_, b)| b.finished.load(Ordering::Acquire))
    }

    /// How many consumers are pulling any of this session's streams — the session-level "are our
    /// bytes being taken" signal (see [`StreamBroadcaster::consumers`]). Summed, because at a track
    /// boundary the renderer is briefly between the old stream and the new one.
    #[must_use]
    pub fn consumers(&self) -> usize {
        let routes = self.inner.lock().expect("routes mutex poisoned");
        routes.streams.iter().map(|(_, b)| b.consumers()).sum()
    }
}

/// Layer 2: build the axum [`Router`] that serves a session's streams.
///
/// The renderer pulls a path from [`StreamRoutes::open`] and gets a chunked `audio/flac` body. The
/// router is state-complete (`Router<()>`), so it can be handed straight to [`axum::serve`] or
/// exercised with `tower`'s `oneshot` in tests.
pub fn router(routes: StreamRoutes) -> Router {
    Router::new()
        .route("/stream/{file}", get(stream_handler))
        .with_state(routes)
}

/// Serve the live FLAC stream on `addr` until the server stops. `addr` must be a specific LAN
/// interface IP (never `0.0.0.0`): the renderer reaches us on exactly the interface discovery
/// chose, and we do not expose the stream on every interface. This call runs until the server
/// exits and is the blocking entry point; use [`spawn`] to run it in the background.
///
/// The response is HTTP/1.1: on a plain (non-TLS) [`TcpListener`], hyper serves HTTP/1.1 by
/// default and nothing here negotiates HTTP/2 (no ALPN, no `http2` opt-in), which is what
/// chunked-transfer renderers expect.
pub async fn serve(addr: SocketAddr, routes: StreamRoutes) -> Result<()> {
    let listener = bind(addr).await?;
    axum::serve(listener, router(routes))
        .await
        .map_err(|e| Error::Sink(format!("stream server exited: {e}")))
}

/// Bind `addr` and run the server on a background task. Returns the *actually bound*
/// [`SocketAddr`] (so callers may pass port 0 and learn the real port, as the tests do) plus
/// the task [`JoinHandle`]. Same interface and HTTP/1.1 rules as [`serve`].
pub async fn spawn(addr: SocketAddr, routes: StreamRoutes) -> Result<(SocketAddr, JoinHandle<()>)> {
    let listener = bind(addr).await?;
    let bound = listener
        .local_addr()
        .map_err(|e| Error::Sink(format!("stream server local_addr: {e}")))?;
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router(routes)).await {
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

/// The one route handler. Serves the named stream in full as `200 OK`, or `404` for a stream this
/// session does not have (never opened, or retired).
async fn stream_handler(
    State(routes): State<StreamRoutes>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    tracing::debug!(%file, ?headers, "stream server: request");
    let Some(broadcaster) = file
        .strip_suffix(".flac")
        .and_then(|id| id.parse().ok())
        .and_then(|id| routes.get(id))
    else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .expect("a bare 404 builds");
    };
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
    // A renderer joining (or re-joining after a drop) is the event this whole module exists to
    // serve correctly, so it is worth a log line: it distinguishes "the device never fetched"
    // from "the device fetched and then rejected the audio".
    tracing::info!("stream server: consumer connected, replaying header + live edge");
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

    /// The whole point of per-load streams: once the track is fed, the body ends, so the renderer
    /// reaches end of stream and can report the track finished.
    #[tokio::test]
    async fn finish_ends_every_subscriber_after_what_was_pushed() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());
        let s = b.subscribe();
        b.push(Bytes::from_static(b"c1"));
        b.finish();
        b.push(Bytes::from_static(b"after"));

        let got: Vec<Bytes> = s.collect().await;
        assert_eq!(got, vec![hdr(), Bytes::from_static(b"c1")]);
    }

    /// A renderer (re)connecting after the track was fed gets the header and an immediate end,
    /// never a body that hangs open waiting for audio that will not come.
    #[tokio::test]
    async fn a_subscriber_after_finish_gets_the_header_and_an_end() {
        let b = StreamBroadcaster::new(16);
        b.set_header(hdr());
        b.finish();
        let got: Vec<Bytes> = b.subscribe().collect().await;
        assert_eq!(got, vec![hdr()]);
    }

    #[test]
    fn a_session_keeps_the_current_and_previous_streams() {
        let routes = StreamRoutes::default();
        let (first, a) = routes.open();
        let (second, _) = routes.open();
        let (third, _) = routes.open();
        assert_eq!(first, "/stream/1.flac");
        assert_eq!(second, "/stream/2.flac");
        assert_eq!(third, "/stream/3.flac");
        assert!(routes.get(1).is_none(), "the oldest stream is retired");
        assert!(routes.get(2).is_some() && routes.get(3).is_some());
        assert!(a.finished.load(Ordering::Acquire), "and its body ended");
    }

    #[test]
    fn a_session_is_drained_once_its_newest_stream_is_fed() {
        let routes = StreamRoutes::default();
        assert!(!routes.is_drained(), "nothing opened yet");
        let (_, first) = routes.open();
        first.finish();
        assert!(routes.is_drained());
        let (_, _second) = routes.open();
        assert!(!routes.is_drained(), "the next track is still being fed");
    }

    #[test]
    fn session_consumers_sum_across_its_streams() {
        let routes = StreamRoutes::default();
        let (_, a) = routes.open();
        let (_, b) = routes.open();
        let _x = a.subscribe();
        let _y = b.subscribe();
        assert_eq!(routes.consumers(), 2);
    }

    // --- Layer 2: axum glue --------------------------------------------------------------

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    /// GET returns 200 + `audio/flac`, the body opens with exactly the header, and it ends when
    /// the track is finished.
    #[tokio::test]
    async fn get_serves_200_flac_starting_with_the_header() {
        let routes = StreamRoutes::default();
        let (path, b) = routes.open();
        b.set_header(hdr());

        let resp = router(routes).oneshot(get(&path)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "audio/flac"
        );

        b.finish();
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(
            bytes,
            hdr(),
            "the stream opens with the decoder-init header"
        );
    }

    #[tokio::test]
    async fn an_unknown_stream_is_404() {
        let routes = StreamRoutes::default();
        let _ = routes.open();
        for uri in ["/stream/9.flac", "/stream/x.flac", "/stream/1.mp3"] {
            let resp = router(routes.clone()).oneshot(get(uri)).await.unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    /// A request with no `Range` header must never draw a `206`. It gets the full 200 stream.
    #[tokio::test]
    async fn no_range_header_never_yields_206() {
        let routes = StreamRoutes::default();
        let (path, b) = routes.open();
        b.set_header(hdr());

        let resp = router(routes).oneshot(get(&path)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Even a `Range` request gets the full stream as 200 for this live/unbounded body — the
    /// 206 branch is an extension point, not something the live stream emits.
    #[tokio::test]
    async fn range_request_still_gets_full_200_stream() {
        let routes = StreamRoutes::default();
        let (path, b) = routes.open();
        b.set_header(hdr());

        let resp = router(routes)
            .oneshot(
                Request::builder()
                    .uri(&path)
                    .header(header::RANGE, "bytes=0-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Consumer presence is the protocol-agnostic "are our bytes actually being taken?" signal the
    /// takeover watchdog keys on (yak canon-2dbf). It must count live subscribers and, crucially,
    /// drop back to zero when a consumer goes away.
    #[tokio::test]
    async fn consumer_count_tracks_live_subscribers() {
        let broadcaster = StreamBroadcaster::default();
        broadcaster.set_header(hdr());
        assert_eq!(broadcaster.consumers(), 0, "nobody is pulling yet");

        let first = broadcaster.subscribe();
        assert_eq!(broadcaster.consumers(), 1);
        let second = broadcaster.subscribe();
        assert_eq!(broadcaster.consumers(), 2);

        drop(second);
        assert_eq!(
            broadcaster.consumers(),
            1,
            "a departed consumer is not counted"
        );
        drop(first);
        assert_eq!(
            broadcaster.consumers(),
            0,
            "zero consumers is what reveals a renderer that stopped pulling"
        );
    }

    /// `spawn` binds the requested interface and reports the real port (port 0 → assigned).
    #[tokio::test]
    async fn spawn_binds_and_reports_the_real_port() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (bound, handle) = spawn(addr, StreamRoutes::default()).await.unwrap();

        assert_eq!(bound.ip(), addr.ip(), "bound the requested interface");
        assert_ne!(bound.port(), 0, "an ephemeral port was actually assigned");

        handle.abort();
    }
}
