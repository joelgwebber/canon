//! A player's audio, fetched from the server that keeps its queue (canon-1ad8).
//!
//! Each start the server orders comes with the path to fetch the track from (`/audio/<id>?token=…`,
//! canon-040e and canon-65d2). This opens it from a position over plain HTTP and hands the engine
//! the body as a forward-only stream: FLAC at the source's own depth and rate, whose header says
//! what it is (a renderer fed from it needs the rate and channels before the first frame) and
//! whose `x-canon-start-ms` says where in the track it starts.
//!
//! A connection that can't be made is tried again, backing off, before the start fails; once
//! streaming, a dropped connection ends the track with an error (the engine reports `Failed`), and
//! the player's queue logic takes it from there.

use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use canon_core::{Codec, Error, ResolvedStream, Result, StreamInfo};

/// How many times to try reaching the server for a track, and the pause before the first retry
/// (doubling each time).
const ATTEMPTS: u32 = 4;
const FIRST_RETRY: Duration = Duration::from_millis(250);
/// How long to wait for the server to answer, and for each read of the body.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Where a player fetches its audio: the server's address (`host:port`).
#[derive(Debug, Clone)]
pub struct RemoteAudio {
    addr: String,
}

impl RemoteAudio {
    #[must_use]
    pub fn new(addr: String) -> Self {
        Self { addr }
    }

    /// Open `path` on the server from about `position`.
    ///
    /// # Errors
    /// The server couldn't be reached after a few tries, refused, or didn't send FLAC.
    pub async fn open(&self, path: &str, position: Duration) -> Result<ResolvedStream> {
        let addr = self.addr.clone();
        let path = with_position(path, position);
        tokio::task::spawn_blocking(move || {
            let mut wait = FIRST_RETRY;
            let mut attempt = 1;
            loop {
                match fetch(&addr, &path) {
                    Ok(stream) => return Ok(stream),
                    // The server answered and said no: asking again won't change its mind.
                    Err(Fetch::Refused(e)) => return Err(e),
                    Err(Fetch::Unreachable(e)) if attempt >= ATTEMPTS => {
                        return Err(Error::Transient(format!(
                            "couldn't reach the server for the track after {ATTEMPTS} tries: {e}"
                        )));
                    }
                    Err(Fetch::Unreachable(e)) => {
                        tracing::info!(
                            attempt,
                            "the server didn't answer for a track ({e}); retrying"
                        );
                        std::thread::sleep(wait);
                        wait *= 2;
                        attempt += 1;
                    }
                }
            }
        })
        .await
        .map_err(|e| Error::Source(format!("fetching a track: {e}")))?
    }
}

/// `path` asking to start at `position`.
fn with_position(path: &str, position: Duration) -> String {
    if position.is_zero() {
        return path.to_owned();
    }
    let joiner = if path.contains('?') { '&' } else { '?' };
    format!("{path}{joiner}position_ms={}", position.as_millis())
}

enum Fetch {
    /// Couldn't get an answer: worth trying again.
    Unreachable(io::Error),
    /// An answer that says no, or isn't audio.
    Refused(Error),
}

fn fetch(addr: &str, path: &str) -> std::result::Result<ResolvedStream, Fetch> {
    let unreachable = Fetch::Unreachable;
    let target = addr
        .to_socket_addrs()
        .map_err(unreachable)?
        .next()
        .ok_or_else(|| {
            Fetch::Unreachable(io::Error::other(format!("{addr} resolves to nothing")))
        })?;
    let mut stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(unreachable)?;
    // HTTP/1.0: the body is everything up to the close, with no chunking to undo.
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {addr}\r\n\r\n").map_err(unreachable)?;
    let mut reader = BufReader::new(stream);

    let mut status = String::new();
    reader.read_line(&mut status).map_err(unreachable)?;
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Fetch::Unreachable(io::Error::other(format!("not HTTP: {status:?}"))))?;
    let mut start_ms = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(unreachable)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case(canon_api::START_MS_HEADER)
        {
            start_ms = value.trim().parse().unwrap_or(0);
        }
    }
    if code != 200 {
        let mut why = String::new();
        let _ = reader.take(512).read_to_string(&mut why);
        let why = format!("the server refused the track ({code}): {}", why.trim());
        return Err(Fetch::Refused(match code {
            401 | 403 => Error::Auth(why),
            404 => Error::NotFound(why),
            502 | 503 => Error::Transient(why),
            _ => Error::Source(why),
        }));
    }

    // The FLAC header leads: its STREAMINFO says the rate, channels and depth.
    let mut head = [0u8; 42];
    reader.read_exact(&mut head).map_err(unreachable)?;
    let info = streaminfo(&head)
        .ok_or_else(|| Fetch::Refused(Error::Source("the server's audio isn't FLAC".into())))?;
    Ok(ResolvedStream {
        input: Box::new(Body {
            head: io::Cursor::new(head.to_vec()),
            rest: reader,
        }),
        info,
        start_ms,
        from: None,
        seek_to: None,
    })
}

/// The stream format from a FLAC stream's first 42 bytes: `fLaC`, then the STREAMINFO block.
fn streaminfo(head: &[u8; 42]) -> Option<StreamInfo> {
    if &head[..4] != b"fLaC" || head[4] & 0x7f != 0 {
        return None;
    }
    // STREAMINFO body starts at 8: min/max block (4), min/max frame (6), then 20 bits of rate,
    // 3 bits of channels - 1, 5 bits of bits-per-sample - 1.
    let info = &head[8..];
    let rate =
        (u32::from(info[10]) << 12) | (u32::from(info[11]) << 4) | (u32::from(info[12]) >> 4);
    let channels = u16::from((info[12] >> 1) & 0x07) + 1;
    let bits = (((info[12] & 0x01) << 4) | (info[13] >> 4)) + 1;
    (rate > 0).then_some(StreamInfo {
        codec: Codec::Flac,
        sample_rate: rate,
        bit_depth: Some(bits),
        channels,
        replaygain: None,
    })
}

/// The response body, with the header bytes already read put back in front. Forward only: a
/// seek is a new request from the new position, which the server answers with a new stream.
struct Body {
    head: io::Cursor<Vec<u8>>,
    rest: BufReader<TcpStream>,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.head.read(buf)? {
            0 => self.rest.read(buf),
            n => Ok(n),
        }
    }
}

impl Seek for Body {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("a streamed track can't be sought ({pos:?}); ask the server for a new one"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_joins_the_query() {
        let token = "/audio/abc?token=t";
        assert_eq!(with_position(token, Duration::ZERO), token);
        assert_eq!(
            with_position(token, Duration::from_millis(61_500)),
            "/audio/abc?token=t&position_ms=61500"
        );
        assert_eq!(
            with_position("/audio/abc", Duration::from_secs(2)),
            "/audio/abc?position_ms=2000"
        );
    }

    /// The format read from a header canon's own encoder wrote: 24-bit, 96 kHz, stereo.
    #[test]
    fn streaminfo_reads_what_the_encoder_wrote() {
        let encoder = canon_sink::FlacEncoder::new(96_000, 2, 24).unwrap();
        let header = encoder.header();
        let head: [u8; 42] = header[..42].try_into().unwrap();
        let info = streaminfo(&head).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bit_depth),
            (96_000, 2, Some(24))
        );
        assert!(streaminfo(&[0; 42]).is_none());
    }
}
