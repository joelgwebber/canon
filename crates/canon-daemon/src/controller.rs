//! The playback controller (yak canon-a7d6): the daemon-level glue that turns a
//! [`Command`] into actual sound and keeps the player state actor authoritative.
//!
//! It implements [`ControlPlane`], so `canon-api` drives it exactly like the bare
//! player — but here a `Load` resolves the track through the Tidal [`Source`], starts
//! the controllable audio engine on the player's shared [`FrameClock`], and forwards the
//! engine's [`EngineEvent`]s back into the player. Transport (pause/resume/stop/volume)
//! fans out to both the engine (real audio) and the player (state), so the emitted
//! snapshot never diverges from what's actually playing.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use canon_audio::AudioPlayer;
use canon_core::{
    Codec, Command, ControlPlane, EngineEvent, Error, PlayerHandle, PlayerSnapshot, Quality,
    Result, SourceRef, TrackRef,
};
use canon_tidal::TidalSession;
use tokio::sync::watch;

pub struct PlaybackController {
    player: PlayerHandle,
    session: Arc<TidalSession>,
    quality: Quality,
    /// The currently-playing engine, if any. Replaced on each load; dropping the old one
    /// stops it.
    audio: Mutex<Option<AudioPlayer>>,
}

impl PlaybackController {
    pub fn new(player: PlayerHandle, session: Arc<TidalSession>, quality: Quality) -> Arc<Self> {
        Arc::new(Self {
            player,
            session,
            quality,
            audio: Mutex::new(None),
        })
    }

    /// Resolve a track and start streaming playback of it.
    async fn load(&self, track: TrackRef) -> Result<()> {
        // Stop any current playback before starting the next.
        if let Some(previous) = self.audio.lock().expect("audio lock").take() {
            previous.stop();
        }

        // Reflect "loading" immediately, carrying the track's metadata/duration.
        self.player.command(Command::Load(track.clone())).await;

        let Some(id) = tidal_id(&track) else {
            let message = "track has no Tidal source".to_string();
            self.player
                .engine(EngineEvent::Failed(message.clone()))
                .await;
            return Err(Error::Unsupported(message));
        };

        // Open the streaming input (starts fast; segments fetched with read-ahead).
        let resolved = match self.session.clone().open_stream(&id, self.quality).await {
            Ok(resolved) => resolved,
            Err(e) => {
                // Surface as playback state, not just a return value — the client's
                // snapshot goes to Error with the reason.
                self.player.engine(EngineEvent::Failed(e.to_string())).await;
                return Err(e);
            }
        };

        let hint = codec_hint(resolved.info.codec);
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();

        // Bridge engine events into the player actor until the engine ends.
        let player = self.player.clone();
        tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                player.engine(event).await;
            }
        });

        let audio = AudioPlayer::start(
            resolved.input,
            hint.map(str::to_owned),
            self.player.clock(),
            events_tx,
        );
        *self.audio.lock().expect("audio lock") = Some(audio);
        Ok(())
    }

    fn with_audio(&self, f: impl FnOnce(&AudioPlayer)) {
        if let Some(audio) = self.audio.lock().expect("audio lock").as_ref() {
            f(audio);
        }
    }
}

#[async_trait]
impl ControlPlane for PlaybackController {
    async fn dispatch(&self, command: Command) -> Result<()> {
        match command {
            Command::Load(track) => self.load(track).await,
            Command::Play => {
                self.with_audio(AudioPlayer::resume);
                self.player.command(Command::Play).await;
                Ok(())
            }
            Command::Pause => {
                self.with_audio(AudioPlayer::pause);
                self.player.command(Command::Pause).await;
                Ok(())
            }
            Command::Stop => {
                if let Some(previous) = self.audio.lock().expect("audio lock").take() {
                    previous.stop();
                }
                self.player.command(Command::Stop).await;
                Ok(())
            }
            Command::SetVolume(volume) => {
                self.with_audio(|audio| audio.set_volume(volume));
                self.player.command(Command::SetVolume(volume)).await;
                Ok(())
            }
            Command::SetMuted(muted) => {
                self.with_audio(|audio| audio.set_muted(muted));
                self.player.command(Command::SetMuted(muted)).await;
                Ok(())
            }
            Command::SelectSink(id) => {
                // Only local output exists today; record it as state.
                self.player.command(Command::SelectSink(id)).await;
                Ok(())
            }
            Command::Seek(_) => Err(Error::Unsupported(
                "seek is not yet supported for streaming playback".into(),
            )),
        }
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        ControlPlane::subscribe(&self.player)
    }

    fn snapshot(&self) -> PlayerSnapshot {
        ControlPlane::snapshot(&self.player)
    }
}

/// The first Tidal binding on a track, if any.
fn tidal_id(track: &TrackRef) -> Option<String> {
    track.sources.iter().find_map(|source| match source {
        SourceRef::Tidal { id } => Some(id.clone()),
        _ => None,
    })
}

/// A Symphonia probe hint for a codec.
fn codec_hint(codec: Codec) -> Option<&'static str> {
    match codec {
        Codec::Flac => Some("flac"),
        Codec::Aac | Codec::Alac => Some("m4a"),
        _ => None,
    }
}
