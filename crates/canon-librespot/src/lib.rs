//! canon-librespot — Spotify audio through librespot (a spike, yak canon-52fb).
//!
//! Spotify's Web API has no audio (docs/connections.md). librespot, a reverse-engineered Spotify
//! client, fetches a track's encrypted file and its audio key the way Spotify's own apps do. This
//! crate wraps the smallest part of it that canon needs: sign in (OAuth, Premium only), and open a
//! Spotify track as a [`canon_core::Source`] stream that canon's own engine decodes, so Spotify
//! audio goes through the same outputs, flow mode and gapless joins as Tidal's.
//!
//! Unofficial, and Spotify's terms likely forbid it; audio-key refusals have been reported since
//! Nov 2025 (librespot #1649). Treat everything here as liable to stop working.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{
    Codec, Error, MediaInput, Quality, ResolvedStream, Result, Service, Source, SourceRef,
    SourceTrack, StreamInfo,
};
use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::{Session, SessionConfig, SpotifyId, SpotifyUri};
use librespot_metadata::audio::AudioFileFormat;
use librespot_metadata::{Metadata, Track};

/// Spotify's own desktop client, which librespot signs in as; the only kind of client Spotify
/// hands audio keys to.
const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
/// Where the OAuth redirect comes back to: librespot listens here for the one request.
const REDIRECT_URI: &str = "http://127.0.0.1:5588/login";

/// Spotify's Ogg files open with a header page of its own, which a decoder must not see.
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

/// The files to try, best first: lossless where the account and track have it (canon's engine
/// decodes FLAC natively), then Ogg Vorbis.
const PREFERRED: [AudioFileFormat; 5] = [
    AudioFileFormat::FLAC_FLAC_24BIT,
    AudioFileFormat::FLAC_FLAC,
    AudioFileFormat::OGG_VORBIS_320,
    AudioFileFormat::OGG_VORBIS_160,
    AudioFileFormat::OGG_VORBIS_96,
];

/// A signed-in librespot session: Spotify audio for one Premium account.
pub struct SpotifyAudio {
    session: Session,
}

impl SpotifyAudio {
    /// Resume the session whose reusable credentials are cached in `dir`, if there are any.
    ///
    /// # Errors
    /// The cache can't be read, or Spotify refused the cached credentials.
    pub async fn restore(dir: &Path) -> Result<Option<Self>> {
        let cache = cache(dir)?;
        let Some(credentials) = cache.credentials() else {
            return Ok(None);
        };
        Ok(Some(Self::connect(cache, credentials).await?))
    }

    /// Sign in in a browser (librespot's OAuth: it opens the page, and listens on
    /// `127.0.0.1:5588` for the redirect), then cache reusable credentials in `dir` so later
    /// sessions need no browser.
    ///
    /// # Errors
    /// The login was declined or failed, or Spotify refused the account (it must be Premium).
    pub async fn login(dir: &Path) -> Result<Self> {
        let client =
            librespot_oauth::OAuthClientBuilder::new(CLIENT_ID, REDIRECT_URI, vec!["streaming"])
                .open_in_browser()
                .build()
                .map_err(|e| Error::Auth(format!("spotify oauth: {e}")))?;
        let token = client
            .get_access_token_async()
            .await
            .map_err(|e| Error::Auth(format!("spotify oauth: {e}")))?;
        let credentials = Credentials::with_access_token(token.access_token);
        Self::connect(cache(dir)?, credentials).await
    }

    async fn connect(cache: Cache, credentials: Credentials) -> Result<Self> {
        let session = Session::new(SessionConfig::default(), Some(cache));
        session
            .connect(credentials, true)
            .await
            .map_err(|e| Error::Auth(format!("spotify: {e}")))?;
        Ok(Self { session })
    }

    /// Who is signed in.
    #[must_use]
    pub fn username(&self) -> String {
        self.session.username()
    }

    /// Open a Spotify track (base62 id) as a stream for canon's engine to decode.
    ///
    /// # Errors
    /// The id is malformed, the track has no file this account may play, the audio key was
    /// refused (an [`Error::Auth`]: Spotify refusing this account audio), or the fetch failed.
    pub async fn open_track(&self, id: &str) -> Result<ResolvedStream> {
        let track_id = SpotifyId::from_base62(id)
            .map_err(|e| Error::NotFound(format!("spotify track {id}: {e}")))?;
        let uri = SpotifyUri::Track { id: track_id };
        let track = Track::get(&self.session, &uri)
            .await
            .map_err(|e| Error::Source(format!("spotify track {id}: {e}")))?;
        // A track can come with no files of its own and point at alternatives: the same
        // recording relinked under another id (another release, another market). librespot's
        // player plays the first alternative that has files; so does this.
        let mut candidates = vec![track];
        for alternative in candidates[0].alternatives.0.clone() {
            match Track::get(&self.session, &alternative).await {
                Ok(track) => candidates.push(track),
                Err(e) => tracing::debug!("spotify alternative {alternative:?}: {e}"),
            }
        }
        let (track, format, file) = candidates
            .iter()
            .find_map(|track| {
                PREFERRED
                    .iter()
                    .find_map(|format| track.files.get(format).map(|file| (track, *format, *file)))
            })
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "spotify track {id} ({}) has no playable file ({} alternatives, offered: {:?})",
                    candidates[0].name,
                    candidates.len() - 1,
                    candidates
                        .iter()
                        .flat_map(|t| t.files.keys().copied())
                        .collect::<Vec<_>>()
                ))
            })?;
        let track_id = match &track.id {
            SpotifyUri::Track { id } => *id,
            _ => track_id,
        };
        tracing::info!(
            "spotify: {} as {format:?} (offered {:?})",
            track.name,
            track.files.keys().collect::<Vec<_>>()
        );

        let key = self
            .session
            .audio_key()
            .request(track_id, file)
            .await
            .map_err(|e| Error::Auth(format!("spotify refused the audio key for {id}: {e}")))?;
        let encrypted = AudioFile::open(&self.session, file, bytes_per_second(format))
            .await
            .map_err(|e| Error::Source(format!("spotify file {file}: {e}")))?;
        if let Ok(controller) = encrypted.get_stream_loader_controller() {
            controller.set_stream_mode();
        }
        let decrypted = AudioDecrypt::new(Some(key), encrypted);
        let (codec, bit_depth, skip) = match format {
            AudioFileFormat::FLAC_FLAC_24BIT => (Codec::Flac, Some(24), 0),
            AudioFileFormat::FLAC_FLAC => (Codec::Flac, Some(16), 0),
            _ => (Codec::Vorbis, None, SPOTIFY_OGG_HEADER_END),
        };
        let input = Skipped::new(decrypted, skip).map_err(Error::Io)?;
        Ok(ResolvedStream {
            input: Box::new(Shared(Mutex::new(input))) as Box<dyn MediaInput>,
            info: StreamInfo {
                codec,
                // Spotify serves everything at 44.1 kHz stereo; the decoder reports the truth
                // either way.
                sample_rate: 44_100,
                bit_depth,
                channels: 2,
                replaygain: None,
            },
            start_ms: 0,
        })
    }
}

fn cache(dir: &Path) -> Result<Cache> {
    let dir: PathBuf = dir.to_path_buf();
    Cache::new(Some(&dir), None, None, None)
        .map_err(|e| Error::Source(format!("spotify cache: {e}")))
}

/// How fast librespot should fetch ahead of playback, from the format's bitrate.
fn bytes_per_second(format: AudioFileFormat) -> usize {
    let kbps = match format {
        AudioFileFormat::FLAC_FLAC_24BIT => 2_400,
        AudioFileFormat::FLAC_FLAC => 1_100,
        AudioFileFormat::OGG_VORBIS_320 => 320,
        AudioFileFormat::OGG_VORBIS_160 => 160,
        _ => 96,
    };
    kbps * 1000 / 8
}

#[async_trait]
impl Source for SpotifyAudio {
    fn service(&self) -> Service {
        Service::Spotify
    }

    /// Always from the start: a spike. (A seek restarts the track at 0 and says so via
    /// `start_ms`, so the clock stays true.)
    async fn open(
        &self,
        source: &SourceRef,
        _quality: Quality,
        _start: Duration,
    ) -> Result<ResolvedStream> {
        match source {
            SourceRef::Spotify { id } => self.open_track(id).await,
            other => Err(Error::Unsupported(format!(
                "librespot cannot play a {} binding",
                other.service()
            ))),
        }
    }

    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        Err(Error::Unsupported(format!(
            "describe {source} through the Spotify Web API connection"
        )))
    }
}

/// A reader with its first `skip` bytes hidden: position 0 is byte `skip` of the inner stream.
struct Skipped<R> {
    inner: R,
    skip: u64,
}

impl<R: Read + Seek> Skipped<R> {
    fn new(mut inner: R, skip: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(skip))?;
        Ok(Self { inner, skip })
    }
}

impl<R: Read> Read for Skipped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Skipped<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(n) => SeekFrom::Start(n + self.skip),
            other => other,
        };
        let at = self.inner.seek(pos)?;
        at.checked_sub(self.skip)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))
    }
}

/// librespot's file reader is `Send` but not `Sync`; the decoder boundary wants both. Every
/// access goes through `&mut self`, so the lock is never contended.
struct Shared<R>(Mutex<R>);

impl<R: Read> Read for Shared<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .get_mut()
            .map_err(|_| io::Error::other("poisoned"))?
            .read(buf)
    }
}

impl<R: Seek> Seek for Shared<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0
            .get_mut()
            .map_err(|_| io::Error::other("poisoned"))?
            .seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn skipped_bytes_are_invisible_to_the_reader() {
        let mut data = vec![0xFFu8; 0xa7];
        data.extend_from_slice(b"OggS rest");
        let mut input = Skipped::new(Cursor::new(data), SPOTIFY_OGG_HEADER_END).unwrap();
        let mut head = [0u8; 4];
        input.read_exact(&mut head).unwrap();
        assert_eq!(&head, b"OggS");
        assert_eq!(input.seek(SeekFrom::Start(0)).unwrap(), 0);
        assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), 9);
        assert_eq!(input.seek(SeekFrom::Current(-4)).unwrap(), 5);
        assert!(input.seek(SeekFrom::Current(-100)).is_err());
    }
}
