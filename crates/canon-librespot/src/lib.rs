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

mod connector;

pub use connector::{LIBRESPOT, LibrespotConnector};

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
use librespot_core::audio_key::AudioKeyError;
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::{Session, SessionConfig, SpotifyId, SpotifyUri};
use librespot_metadata::audio::AudioFileFormat;
use librespot_metadata::{Metadata, Track};

/// Spotify's own desktop client, which librespot signs in as; the only kind of client Spotify
/// hands audio keys to.
pub(crate) const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
/// Where the OAuth redirect comes back to: librespot listens here for the one request.
pub(crate) const REDIRECT_URI: &str = "http://127.0.0.1:5588/login";

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
///
/// A librespot [`Session`] is dead for good once Spotify closes its connection, which it does
/// every so often; nothing in librespot replaces it. So this signs a fresh one in from the cached
/// credentials whenever the current one has gone (canon-e828).
pub struct SpotifyAudio {
    /// The current session. The lock is held across a sign-in, so opens that find the session
    /// gone at once share one replacement.
    session: tokio::sync::Mutex<Session>,
    /// Where the reusable credentials are cached: what a replacement session signs in with.
    dir: PathBuf,
    username: String,
}

impl SpotifyAudio {
    /// Resume the session whose reusable credentials are cached in `dir`, if there are any.
    ///
    /// # Errors
    /// The cache can't be read, or Spotify refused the cached credentials.
    pub async fn restore(dir: &Path) -> Result<Option<Self>> {
        let Some(credentials) = cache(dir)?.credentials() else {
            return Ok(None);
        };
        Ok(Some(Self::connect(dir, credentials).await?))
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
        Self::connect(dir, credentials).await
    }

    /// Sign in with an access token for Spotify's own client carrying the `streaming` scope
    /// (from a login canon ran itself), caching reusable credentials in `dir`.
    ///
    /// # Errors
    /// Spotify refused the token or the account (it must be Premium).
    pub async fn with_access_token(dir: &Path, token: String) -> Result<Self> {
        Self::connect(dir, Credentials::with_access_token(token)).await
    }

    async fn connect(dir: &Path, credentials: Credentials) -> Result<Self> {
        let session = sign_in(dir, credentials).await?;
        Ok(Self {
            username: session.username(),
            session: tokio::sync::Mutex::new(session),
            dir: dir.to_path_buf(),
        })
    }

    /// Who is signed in.
    #[must_use]
    pub fn username(&self) -> String {
        self.username.clone()
    }

    /// Close the session, as Spotify does every so often: the next open signs in again. For
    /// exercising that path (`canon spotify-play --drop-session`).
    pub async fn close_session(&self) {
        self.session.lock().await.shutdown();
    }

    /// The current session, signed in afresh from the cached credentials if Spotify has closed
    /// the last one.
    async fn session(&self) -> Result<Session> {
        let mut session = self.session.lock().await;
        if session.is_invalid() {
            tracing::info!("spotify: the session was closed; signing in again");
            let credentials = cache(&self.dir)?.credentials().ok_or_else(|| {
                Error::Auth("spotify: no cached credentials to sign in again with".into())
            })?;
            *session = sign_in(&self.dir, credentials).await?;
        }
        Ok(session.clone())
    }

    /// Open a Spotify track (base62 id) as a stream for canon's engine to decode.
    ///
    /// # Errors
    /// The id is malformed, the track has no file this account may play, the audio key was
    /// refused (an [`Error::Auth`]: Spotify refusing this account audio), or the fetch failed.
    pub async fn open_track(&self, id: &str) -> Result<ResolvedStream> {
        let session = self.session().await?;
        match open_on(&session, id).await {
            // The key request went over a connection that had died without librespot noticing
            // yet (it only finds out when a send fails, or a keepalive is missed a minute or more
            // later). Retire the session and try once more on a fresh one.
            Err(Opening::Connection(why)) => {
                tracing::warn!("spotify: {why}; retrying on a fresh session");
                session.shutdown();
                let session = self.session().await?;
                open_on(&session, id).await.map_err(Opening::into_error)
            }
            opened => opened.map_err(Opening::into_error),
        }
    }
}

/// Why opening a track on one session failed.
enum Opening {
    /// The audio key didn't come back for a reason other than Spotify refusing it: a timeout, or
    /// the session's connection gone. Worth one retry on a fresh session.
    Connection(String),
    Failed(Error),
}

impl Opening {
    fn into_error(self) -> Error {
        match self {
            Opening::Connection(why) => Error::Source(why),
            Opening::Failed(e) => e,
        }
    }
}

impl From<Error> for Opening {
    fn from(e: Error) -> Self {
        Opening::Failed(e)
    }
}

/// Why an audio key request for `id` failed. Spotify answering with a refusal is the only
/// [`Error::Auth`] — the one that takes streaming away from the login — and a key that never
/// came back is a connection to retry, not a verdict on the account.
fn key_failure(id: &str, e: &librespot_core::Error) -> Opening {
    if matches!(e.error.downcast_ref(), Some(AudioKeyError::AesKey)) {
        Opening::Failed(Error::Auth(format!(
            "spotify refused the audio key for {id}: {e}"
        )))
    } else {
        Opening::Connection(format!("no audio key for {id}: {e}"))
    }
}

/// Sign a new session in with `credentials`, caching reusable ones in `dir`. Only Spotify turning
/// the credentials down is an [`Error::Auth`]; not reaching Spotify is not.
async fn sign_in(dir: &Path, credentials: Credentials) -> Result<Session> {
    let session = Session::new(SessionConfig::default(), Some(cache(dir)?));
    session.connect(credentials, true).await.map_err(|e| {
        use librespot_core::error::ErrorKind;
        match e.kind {
            ErrorKind::Unauthenticated | ErrorKind::PermissionDenied => {
                Error::Auth(format!("spotify: {e}"))
            }
            _ => Error::Source(format!("spotify: {e}")),
        }
    })?;
    Ok(session)
}

/// Open a Spotify track on `session`. Only Spotify answering the key request with a refusal is an
/// [`Error::Auth`]; a key that never arrives is [`Opening::Connection`].
async fn open_on(session: &Session, id: &str) -> std::result::Result<ResolvedStream, Opening> {
    let track_id = SpotifyId::from_base62(id)
        .map_err(|e| Error::NotFound(format!("spotify track {id}: {e}")))?;
    let uri = SpotifyUri::Track { id: track_id };
    let track = Track::get(session, &uri)
        .await
        .map_err(|e| Error::Source(format!("spotify track {id}: {e}")))?;
    // A track can come with no files of its own and point at alternatives: the same
    // recording relinked under another id (another release, another market). librespot's
    // player plays the first alternative that has files; so does this.
    let mut candidates = vec![track];
    for alternative in candidates[0].alternatives.0.clone() {
        match Track::get(session, &alternative).await {
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

    let key = session
        .audio_key()
        .request(track_id, file)
        .await
        .map_err(|e| key_failure(id, &e))?;
    let encrypted = AudioFile::open(session, file, bytes_per_second(format))
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
        from: None,
        seek_to: None,
    })
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

    /// The whole file, decrypted and seekable: a position is reached by the engine seeking into
    /// it (`seek_to`), not by fetching from there.
    async fn open(
        &self,
        source: &SourceRef,
        _quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream> {
        match source {
            SourceRef::Spotify { id } => {
                let mut stream = self.open_track(id).await?;
                stream.seek_to = (!start.is_zero()).then_some(start);
                Ok(stream)
            }
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

    /// A refused key takes streaming away from the login; a key that never arrived, or a
    /// connection that died under the request, is only worth a retry (canon-e828: a dead
    /// connection's timeout was once recorded as a refusal, and Spotify stayed off until the next
    /// sign-in).
    #[test]
    fn only_a_refused_key_is_an_auth_failure() {
        let refused = librespot_core::Error::from(AudioKeyError::AesKey);
        assert!(matches!(
            key_failure("x", &refused),
            Opening::Failed(Error::Auth(_))
        ));
        for transient in [AudioKeyError::Timeout, AudioKeyError::Channel] {
            let e = librespot_core::Error::from(transient);
            assert!(matches!(key_failure("x", &e), Opening::Connection(_)));
        }
    }

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
