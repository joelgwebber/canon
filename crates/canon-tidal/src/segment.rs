//! Streaming segment reader (yak canon-e99d): present the resolved segment list to the
//! decoder as a forward byte stream, fetched lazily with a small read-ahead, and
//! transparently re-resolved when a signed segment URL expires.
//!
//! This is the fix for tideway's tide-1100/bd9e: there, an expired (403) segment mid-
//! track crashed playback. Here the async producer re-resolves the manifest and resumes
//! at the *same* segment index, so a long track that outlives its signed URLs keeps
//! playing without a gap the listener can hear.
//!
//! Shape: an async producer task fetches segments into a bounded channel (backpressure =
//! read-ahead depth); the decoder pulls on a plain sync [`Read`]. The channel receiver
//! is wrapped in a `Mutex` so the reader is `Sync`, which Symphonia's `MediaSource`
//! requires.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Mutex;

use canon_core::Error;
use tokio::sync::mpsc::Receiver;

/// A segment handed from the producer to the reader, or a terminal error.
pub(crate) enum Chunk {
    Data(Vec<u8>),
    Err(String),
}

/// The outcome of fetching one segment URL, distinguishing an expired (re-resolvable)
/// URL from a real failure.
pub(crate) enum SegmentFetch {
    Data(Vec<u8>),
    Expired,
    Failed(Error),
}

/// A forward, non-seekable reader over the segment stream.
pub(crate) struct SegmentReader {
    rx: Mutex<Receiver<Chunk>>,
    current: Cursor<Vec<u8>>,
    /// Absolute bytes read so far (answers `stream_position`).
    pos: u64,
    eof: bool,
}

impl SegmentReader {
    pub(crate) fn new(rx: Receiver<Chunk>) -> Self {
        Self {
            rx: Mutex::new(rx),
            current: Cursor::new(Vec::new()),
            pos: 0,
            eof: false,
        }
    }
}

impl Read for SegmentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let n = self.current.read(buf)?;
            if n > 0 {
                self.pos += n as u64;
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            // Current segment drained: block for the next one (or EOF / error).
            let next = self.rx.lock().expect("segment rx lock").blocking_recv();
            match next {
                Some(Chunk::Data(bytes)) => self.current = Cursor::new(bytes),
                Some(Chunk::Err(message)) => {
                    self.eof = true;
                    return Err(std::io::Error::other(message));
                }
                None => {
                    self.eof = true;
                    return Ok(0);
                }
            }
        }
    }
}

impl Seek for SegmentReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // Only the position query is supported; in-track seek over the stream is future
        // work (rebuild sliced from a segment index, canon-e99d).
        match pos {
            SeekFrom::Current(0) => Ok(self.pos),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "tidal streaming source is not seekable",
            )),
        }
    }
}
