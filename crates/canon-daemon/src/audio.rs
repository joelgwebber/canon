//! The server's audio for players (yak canon-040e): a library track opened through [`Sources`],
//! decoded, and re-encoded as FLAC at the source's own bit depth and sample rate.
//!
//! `canon-api` owns the endpoint (`GET /audio/<track>`) and calls this through
//! [`canon_api::AudioSource`], since it can't link the audio or sink crates.
//!
//! No bit-depth cap applies here: `FLAC_BITS` is what a renderer takes, and stays the player's
//! business. A source with no integer samples of its own (AAC, Vorbis) is served at 16 bits.
//! There is no pacing either. The stream runs as fast as the player reads it, held back by the
//! connection, not by a clock: that is what lets a player buffer ahead.

use std::io;
use std::time::Duration;

use canon_api::{AudioSource, AudioStream, Body, BoxFuture};
use canon_audio::PcmStream;
use canon_core::{EntityId, Error, Quality, Result, Sources};
use canon_library::Library;
use canon_sink::FlacEncoder;
use futures_util::stream;
use tokio::sync::{mpsc, oneshot};

/// How many chunks of FLAC wait for the connection before the decoder stops to let it catch up.
const CHUNKS_IN_FLIGHT: usize = 8;

/// Serves the library's tracks from whichever service holds them.
pub struct ServerAudio {
    library: Library,
    sources: Sources,
    quality: Quality,
}

impl ServerAudio {
    /// Serve the tracks of `library`, opened through `sources` at up to `quality` (a source
    /// clamps it to what it can give).
    #[must_use]
    pub fn new(library: Library, sources: Sources, quality: Quality) -> Self {
        Self {
            library,
            sources,
            quality,
        }
    }

    async fn open_track(&self, id: EntityId, position: Duration) -> Result<AudioStream> {
        let track = self
            .library
            .track_refs(vec![id])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::NotFound(format!("track {id}")))?;
        let resolved = self.sources.open(&track, self.quality, position).await?;
        if let Some(from) = resolved.playing_from() {
            tracing::info!(
                "streaming \"{}\" from {from} at {position:?}",
                track.meta.title
            );
        }

        // Decoding is blocking work (a source's reads block), and it lasts as long as the
        // stream does: a thread of its own, as the engine's feed has.
        let (opened_tx, opened_rx) = oneshot::channel();
        let (chunks_tx, chunks_rx) = mpsc::channel(CHUNKS_IN_FLIGHT);
        std::thread::Builder::new()
            .name("canon-audio-stream".into())
            .spawn(move || {
                let pcm = PcmStream::open(
                    resolved.input,
                    resolved.info.codec.extension_hint(),
                    resolved.start_ms,
                    resolved.seek_to,
                    resolved.info.bit_depth,
                );
                match pcm
                    .map_err(|e| Error::Source(e.to_string()))
                    .and_then(|pcm| {
                        let encoder = FlacEncoder::new(
                            pcm.sample_rate(),
                            pcm.channels(),
                            u16::from(pcm.bit_depth()),
                        )?;
                        Ok((pcm, encoder))
                    }) {
                    Ok((pcm, encoder)) => {
                        let _ = opened_tx.send(Ok(pcm.start_ms()));
                        pump(pcm, encoder, &chunks_tx);
                    }
                    Err(e) => {
                        let _ = opened_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| Error::Source(format!("starting the stream: {e}")))?;
        let start_ms = opened_rx
            .await
            .map_err(|_| Error::Source("the stream's decoder died opening it".into()))??;

        let body = Body::from_stream(stream::unfold(chunks_rx, |mut chunks| async move {
            chunks.recv().await.map(|chunk| (chunk, chunks))
        }));
        Ok(AudioStream { start_ms, body })
    }
}

impl AudioSource for ServerAudio {
    fn open(&self, track: EntityId, position: Duration) -> BoxFuture<'_, Result<AudioStream>> {
        Box::pin(self.open_track(track, position))
    }
}

/// Decode `pcm` to its end and send it as FLAC down `chunks`, a frame at a time. Stops when the
/// player has gone (the receiver is dropped), and ends the body with an error if the decode
/// fails, so a broken track doesn't look like one that ended.
fn pump(mut pcm: PcmStream, mut encoder: FlacEncoder, chunks: &mpsc::Sender<io::Result<Vec<u8>>>) {
    if !send(chunks, encoder.header()) {
        return;
    }
    loop {
        match pcm.next_chunk() {
            Ok(Some(samples)) => {
                let mut frames = Vec::new();
                let encoded = encoder.write(&samples, |frame| frames.push(frame));
                if !frames.into_iter().all(|frame| send(chunks, frame)) {
                    return;
                }
                if let Err(e) = encoded {
                    fail(chunks, &e);
                    return;
                }
            }
            Ok(None) => {
                match encoder.finish() {
                    Ok(Some(last)) => {
                        send(chunks, last);
                    }
                    Ok(None) => {}
                    Err(e) => fail(chunks, &e),
                }
                return;
            }
            Err(e) => {
                fail(chunks, &e);
                return;
            }
        }
    }
}

/// Send one chunk; false once the player has gone.
fn send(chunks: &mpsc::Sender<io::Result<Vec<u8>>>, bytes: impl AsRef<[u8]>) -> bool {
    chunks.blocking_send(Ok(bytes.as_ref().to_vec())).is_ok()
}

fn fail(chunks: &mpsc::Sender<io::Result<Vec<u8>>>, error: &dyn std::fmt::Display) {
    tracing::warn!("audio stream broke: {error}");
    let _ = chunks.blocking_send(Err(io::Error::other(error.to_string())));
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use async_trait::async_trait;
    use canon_api::{AppState, START_MS_HEADER};
    use canon_audio::Codec;
    use canon_core::{
        ControlPlane, PlayerId, Players, ResolvedStream, Service, Source, SourceRef, SourceTrack,
        StreamInfo,
    };
    use canon_library::{Binding, Store, Track};
    use canon_protocol::EntityKind;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    /// How a test source places a stream relative to the position asked for.
    #[derive(Clone, Copy)]
    enum Placing {
        /// Bytes from the start of the file, to be sought into (a local file; librespot).
        Seekable,
        /// Start at the whole second at or before the position (what Tidal's segments do). The
        /// bytes are the whole file, so only the start the source reports is under test.
        Segments,
    }

    /// Local files by path, placed as `placing` says.
    struct Files(Placing);

    #[async_trait]
    impl Source for Files {
        fn service(&self) -> Service {
            Service::Local
        }

        async fn open(
            &self,
            source: &SourceRef,
            _quality: Quality,
            start: Duration,
        ) -> Result<ResolvedStream> {
            let SourceRef::Local { path } = source else {
                return Err(Error::Unsupported("local files only".into()));
            };
            let codec = match path.extension().and_then(|e| e.to_str()) {
                Some("flac") | Some("mp4") => Codec::Flac,
                _ => Codec::Aac,
            };
            let (start_ms, seek_to) = match self.0 {
                Placing::Seekable => (0, Some(start)),
                Placing::Segments => (start.as_millis() as u64 / 1000 * 1000, None),
            };
            Ok(ResolvedStream {
                input: Box::new(std::fs::File::open(path)?),
                info: StreamInfo {
                    codec,
                    sample_rate: 0,
                    bit_depth: None,
                    channels: 0,
                    replaygain: None,
                },
                start_ms,
                from: None,
                seek_to,
            })
        }

        async fn describe(&self, _source: &SourceRef) -> Result<SourceTrack> {
            Err(Error::Unsupported("files aren't described".into()))
        }
    }

    struct NoPlayers;

    #[async_trait]
    impl Players for NoPlayers {
        async fn player(&self, _id: &PlayerId) -> Result<Arc<dyn ControlPlane>> {
            Err(Error::Unsupported("no players here".into()))
        }
    }

    fn asset(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../canon-audio/tests/assets")
            .join(name)
    }

    /// A library holding one track per path, each bound to its file; and a server over it.
    struct Server {
        addr: std::net::SocketAddr,
        tracks: Vec<EntityId>,
    }

    /// The key the test servers sign audio URLs with.
    fn signer() -> canon_api::StreamSigner {
        canon_api::StreamSigner::with_key([3; 32])
    }

    async fn serve(placing: Placing, files: &[PathBuf], with_audio: bool) -> Server {
        serve_as(placing, files, with_audio, None).await
    }

    /// As [`serve`], admitting by `paired` (a token store) when given, as a server off loopback.
    async fn serve_as(
        placing: Placing,
        files: &[PathBuf],
        with_audio: bool,
        paired: Option<canon_api::TokenStore>,
    ) -> Server {
        let library = Library::new(Store::open_in_memory().unwrap());
        let paths = files.to_vec();
        let tracks = library
            .run(move |store| {
                paths
                    .into_iter()
                    .map(|path| {
                        let id = store.add_track(&Track {
                            title: "Test".into(),
                            credit: "Test".into(),
                            artists: Vec::new(),
                            duration_ms: None,
                            isrcs: Vec::new(),
                            mbid: None,
                        })?;
                        store.bind(
                            EntityKind::Track,
                            id,
                            &Binding::direct(SourceRef::Local { path }),
                        )?;
                        Ok(id)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await
            .unwrap();
        let sources = Sources::new().with(Arc::new(Files(placing)));
        let mut state = AppState::new(Arc::new(NoPlayers));
        if let Some(tokens) = paired {
            state = state.with_access(canon_api::Access::Paired, tokens);
        }
        if with_audio {
            state = state.with_audio(
                Arc::new(ServerAudio::new(library, sources, Quality::HiRes)),
                signer(),
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(canon_api::serve(Arc::new(state), listener));
        Server { addr, tracks }
    }

    struct Reply {
        status: u16,
        start_ms: Option<u64>,
        content_type: Option<String>,
        body: Vec<u8>,
    }

    /// `GET path`, read to the end. HTTP/1.0, so the body is the bytes up to the close, with no
    /// chunking to undo.
    async fn get(addr: std::net::SocketAddr, path: &str) -> Reply {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.0\r\nHost: canon\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8(raw[..split].to_vec()).unwrap();
        let mut lines = head.lines();
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let header = |name: &str| {
            lines
                .clone()
                .find_map(|line| {
                    line.split_once(": ")
                        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                })
                .map(|(_, value)| value.to_owned())
        };
        Reply {
            status,
            start_ms: header(START_MS_HEADER).map(|v| v.parse().unwrap()),
            content_type: header("content-type"),
            body: raw[split + 4..].to_vec(),
        }
    }

    /// What a player sees decoding a response: the format, and every sample.
    struct Decoded {
        sample_rate: u32,
        channels: u16,
        bit_depth: u8,
        samples: Vec<i32>,
    }

    fn decode_flac(bytes: Vec<u8>) -> Decoded {
        assert!(
            bytes.starts_with(b"fLaC"),
            "a FLAC stream starts with its magic"
        );
        let mut pcm =
            PcmStream::open(Box::new(Cursor::new(bytes)), Some("flac"), 0, None, None).unwrap();
        let mut samples = Vec::new();
        while let Some(chunk) = pcm.next_chunk().unwrap() {
            samples.extend(chunk);
        }
        Decoded {
            sample_rate: pcm.sample_rate(),
            channels: pcm.channels(),
            bit_depth: pcm.bit_depth(),
            samples,
        }
    }

    /// The samples of a file decoded straight, as the reference for what the server sends.
    fn decode_file(path: &Path, hint: &str) -> Decoded {
        let mut pcm = PcmStream::open(
            Box::new(std::fs::File::open(path).unwrap()),
            Some(hint),
            0,
            None,
            None,
        )
        .unwrap();
        let mut samples = Vec::new();
        while let Some(chunk) = pcm.next_chunk().unwrap() {
            samples.extend(chunk);
        }
        Decoded {
            sample_rate: pcm.sample_rate(),
            channels: pcm.channels(),
            bit_depth: pcm.bit_depth(),
            samples,
        }
    }

    /// A track as Tidal serves it, FLAC in fragmented MP4, comes back as the same samples at
    /// the same rate and depth, from the start of the track.
    #[tokio::test]
    async fn serves_fragmented_flac_sample_for_sample() {
        let file = asset("flac_frag.mp4");
        let server = serve(Placing::Segments, std::slice::from_ref(&file), true).await;
        let reply = get(server.addr, &format!("/audio/{}", server.tracks[0])).await;

        assert_eq!(reply.status, 200);
        assert_eq!(reply.content_type.as_deref(), Some("audio/flac"));
        assert_eq!(reply.start_ms, Some(0));
        let served = decode_flac(reply.body);
        let source = decode_file(&file, "flac");
        assert_eq!(served.sample_rate, 44_100);
        assert_eq!(served.bit_depth, 16);
        assert_eq!(served.channels, 1);
        assert_eq!(served.sample_rate, source.sample_rate);
        assert!(served.samples.len() > 44_100, "two seconds of audio");
        assert_eq!(served.samples, source.samples, "bit-exact");
    }

    /// A source that starts before the position asked for says so, and the stream isn't trimmed.
    #[tokio::test]
    async fn reports_where_a_segmented_source_really_starts() {
        let file = asset("flac_frag.mp4");
        let server = serve(Placing::Segments, std::slice::from_ref(&file), true).await;
        let reply = get(
            server.addr,
            &format!("/audio/{}?position_ms=1500", server.tracks[0]),
        )
        .await;

        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.start_ms,
            Some(1000),
            "the segment holding 1.5s starts at 1s"
        );
        assert_eq!(
            decode_flac(reply.body).samples,
            decode_file(&file, "flac").samples,
            "the source's own bytes, untrimmed"
        );
    }

    /// A seekable source is sought into: the stream starts at or just before the position, and
    /// the header says where, not where it was asked to.
    #[tokio::test]
    async fn seeks_into_a_seekable_source() {
        let file = asset("aac_plain.m4a");
        let server = serve(Placing::Seekable, std::slice::from_ref(&file), true).await;
        let whole = decode_flac(
            get(server.addr, &format!("/audio/{}", server.tracks[0]))
                .await
                .body,
        );
        let reply = get(
            server.addr,
            &format!("/audio/{}?position_ms=1000", server.tracks[0]),
        )
        .await;

        assert_eq!(reply.status, 200);
        let start = reply.start_ms.expect("the start is reported");
        assert!(
            (900..=1000).contains(&start),
            "landed at {start}ms for 1000ms"
        );
        let served = decode_flac(reply.body);
        assert_eq!(served.bit_depth, 16, "lossy sources are served at 16 bits");
        assert_eq!(served.sample_rate, 44_100);
        // What was sent is the tail of the track, from the reported start (to a millisecond).
        let expected = whole.samples.len() - (start as usize * 44_100 / 1000);
        assert!(
            served.samples.len().abs_diff(expected) <= 44,
            "{} samples from {start}ms, expected about {expected}",
            served.samples.len()
        );
    }

    /// Hi-res stays hi-res: 24-bit at 96 kHz, stereo, round-trips exactly, whole and from a seek.
    #[tokio::test]
    async fn serves_hi_res_at_its_native_format() {
        let rate = 96_000u32;
        let frames = rate as usize * 2;
        let original: Vec<i32> = (0..frames)
            .flat_map(|i| {
                let t = i as f64 / f64::from(rate);
                let left = (t * TAU * 440.0).sin() * 0.6;
                let right = (t * TAU * 659.0).sin() * 0.4 + (t * TAU * 5_000.0).sin() * 0.05;
                [left, right].map(|x| (x * 8_388_607.0).round() as i32)
            })
            .collect();
        let mut encoder = FlacEncoder::new(rate, 2, 24).unwrap();
        let mut file = encoder.header().to_vec();
        encoder
            .write(&original, |frame| file.extend_from_slice(&frame))
            .unwrap();
        file.extend_from_slice(&encoder.finish().unwrap().unwrap());
        let path = std::env::temp_dir().join(format!("canon-040e-{}.flac", std::process::id()));
        std::fs::write(&path, &file).unwrap();

        let server = serve(Placing::Seekable, std::slice::from_ref(&path), true).await;
        let whole = get(server.addr, &format!("/audio/{}", server.tracks[0])).await;
        let seeked = get(
            server.addr,
            &format!("/audio/{}?position_ms=500", server.tracks[0]),
        )
        .await;
        std::fs::remove_file(&path).unwrap();

        assert_eq!(whole.start_ms, Some(0));
        let whole = decode_flac(whole.body);
        assert_eq!(
            (whole.sample_rate, whole.channels, whole.bit_depth),
            (96_000, 2, 24)
        );
        assert_eq!(whole.samples, original, "bit-perfect");

        let start = seeked.start_ms.expect("the start is reported");
        assert!((1..=500).contains(&start), "landed at {start}ms for 500ms");
        let tail = decode_flac(seeked.body);
        assert_eq!((tail.sample_rate, tail.bit_depth), (96_000, 24));
        assert_eq!(
            tail.samples,
            original[original.len() - tail.samples.len()..],
            "the rest of the track, from a frame boundary, bit-perfect"
        );
        let from = original.len() - tail.samples.len();
        let landed_ms = from / 2 * 1000 / rate as usize;
        assert!(
            landed_ms.abs_diff(start as usize) <= 1,
            "the reported start, {start}ms, is where the audio is, {landed_ms}ms"
        );
    }

    #[tokio::test]
    async fn refuses_what_it_cannot_serve() {
        let server = serve(Placing::Segments, &[asset("flac_frag.mp4")], true).await;
        let unknown = get(server.addr, &format!("/audio/{}", EntityId::new())).await;
        assert_eq!(unknown.status, 404);
        let not_an_id = get(server.addr, "/audio/not-an-id").await;
        assert_eq!(not_an_id.status, 400);

        let bare = serve(Placing::Segments, &[asset("flac_frag.mp4")], false).await;
        let none = get(bare.addr, &format!("/audio/{}", bare.tracks[0])).await;
        assert_eq!(none.status, 501, "a server with no audio says so");
    }

    /// Off loopback, audio needs a token this server signed, for that track (canon-65d2): none is
    /// 401, another key's or another track's is 403, and the right one plays.
    #[tokio::test]
    async fn a_paired_server_serves_audio_to_signed_urls_only() {
        let dir = std::env::temp_dir().join(format!("canon-audio-auth-{}", EntityId::new().0));
        let server = serve_as(
            Placing::Seekable,
            &[asset("flac_frag.mp4"), asset("aac_plain.m4a")],
            true,
            Some(canon_api::TokenStore::in_dir(&dir)),
        )
        .await;
        let (track, other) = (server.tracks[0], server.tracks[1]);
        let kitchen = PlayerId::parse("kitchen").unwrap();
        let ttl = canon_api::auth::STREAM_TOKEN_TTL;

        assert_eq!(
            get(server.addr, &format!("/audio/{}", track.0))
                .await
                .status,
            401
        );
        let foreign = canon_api::StreamSigner::with_key([4; 32]).sign(&kitchen, track, ttl);
        let path = format!("/audio/{}?token={foreign}", track.0);
        assert_eq!(
            get(server.addr, &path).await.status,
            403,
            "another server's"
        );
        let elsewhere = signer().sign(&kitchen, other, ttl);
        let path = format!("/audio/{}?token={elsewhere}", track.0);
        assert_eq!(get(server.addr, &path).await.status, 403, "another track's");

        let token = signer().sign(&kitchen, track, ttl);
        let reply = get(server.addr, &format!("/audio/{}?token={token}", track.0)).await;
        assert_eq!(reply.status, 200);
        assert_eq!(reply.content_type.as_deref(), Some("audio/flac"));
    }
}
