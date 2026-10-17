//! `canon play-file`: a local file through the real player actor and the real engine, with no
//! daemon, no sources, and no state directory.
//!
//! This is the smallest executor there is — it carries out the actor's [`Effect`]s on the local
//! output and feeds every [`EngineEvent`] back tagged with its generation, as the daemon's
//! playback controller does — so what it prints is what a client would see: position as the
//! actor derives it from the engine's reports (canon-5ce4). It exists to check local-output
//! position on real hardware without touching the shared library.
//!
//! With `--script`, commands come one per line on stdin, like `canon control`:
//! `sleep <secs>`, `seek <secs>|+<secs>|-<secs>`, `pause`, `play`, `vol <percent>`, `next`,
//! `reopen` (force the device-recovery path), `quit`. Without it, the queue plays to its end.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use canon_audio::{AudioPlayer, Output};
use canon_core::{
    Command, Effect, EngineEvent, EntityId, MediaInput, PlaybackState, PlayerHandle,
    PlayerSnapshot, SourceRef, TrackMeta, TrackRef,
};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, mpsc};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// How often the harness prints the actor's snapshot while it waits.
const PRINT_EVERY: Duration = Duration::from_millis(500);

pub(crate) async fn run(path: PathBuf, times: usize, script: bool) -> Result<(), BoxError> {
    let duration_ms = duration_ms(&path)?;
    let title = path
        .file_name()
        .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
    let tracks: Vec<TrackRef> = (0..times.max(1))
        .map(|i| TrackRef {
            id: EntityId::new(),
            meta: TrackMeta {
                title: format!("{title} #{}", i + 1),
                duration_ms: Some(duration_ms),
                ..TrackMeta::default()
            },
            sources: vec![SourceRef::Local { path: path.clone() }],
        })
        .collect();

    let (player, effects) = PlayerHandle::spawn_with_effects();
    let executor = Arc::new(Executor {
        player: player.clone(),
        path,
        audio: Mutex::new(None),
    });
    tokio::spawn(Arc::clone(&executor).run(effects));

    println!(
        "playing {} × {} ({duration_ms} ms each) through the player actor …",
        executor.path.display(),
        tracks.len()
    );
    player
        .command(Command::Replace { tracks, start: 0 })
        .await?;

    if script {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            println!("> {line}");
            if !step(&player, &executor, line).await? {
                break;
            }
        }
    } else {
        watch(&player, None).await;
    }
    player.command(Command::Stop).await?;
    print(&player.snapshot());
    Ok(())
}

/// Carry out one script line. `false` ends the script.
async fn step(player: &PlayerHandle, executor: &Executor, line: &str) -> Result<bool, BoxError> {
    let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
    let arg = arg.trim();
    let outcome = match verb {
        "quit" => return Ok(false),
        "sleep" => {
            let secs: f64 = arg.parse()?;
            watch(player, Some(Duration::from_secs_f64(secs))).await;
            Ok(())
        }
        "pause" => player.command(Command::Pause).await,
        "play" => player.command(Command::Play).await,
        "next" => player.command(Command::Next).await,
        "vol" => {
            let percent: f32 = arg.parse()?;
            player.command(Command::SetVolume(percent / 100.0)).await
        }
        "seek" => {
            let now = player.snapshot().position_ms as f64 / 1000.0;
            let secs: f64 = if let Some(by) = arg.strip_prefix('+') {
                now + by.parse::<f64>()?
            } else if let Some(by) = arg.strip_prefix('-') {
                (now - by.parse::<f64>()?).max(0.0)
            } else {
                arg.parse()?
            };
            player
                .command(Command::Seek(Duration::from_secs_f64(secs)))
                .await
        }
        "reopen" => {
            if let Some((_, audio)) = executor.audio.lock().await.as_ref() {
                audio.request_reopen();
            }
            Ok(())
        }
        other => return Err(format!("unknown command {other:?}").into()),
    };
    if let Err(e) = outcome {
        println!("refused: {e}");
    }
    print(&player.snapshot());
    Ok(true)
}

/// Print the snapshot every [`PRINT_EVERY`], and on every transition, for `for` (or until the
/// queue ends or fails, without one).
async fn watch(player: &PlayerHandle, until: Option<Duration>) {
    let deadline = until.map(|d| tokio::time::Instant::now() + d);
    let mut snapshots = player.subscribe();
    let mut tick = tokio::time::interval(PRINT_EVERY);
    let mut last_seq = snapshots.borrow().seq;
    loop {
        let ended = tokio::select! {
            _ = tick.tick() => {
                print(&player.snapshot());
                false
            }
            changed = snapshots.changed() => {
                if changed.is_err() {
                    return;
                }
                let snapshot = snapshots.borrow_and_update().clone();
                if snapshot.seq != last_seq {
                    last_seq = snapshot.seq;
                    print(&snapshot);
                }
                false
            }
            () = sleep_until(deadline) => true,
        };
        let state = player.snapshot().state;
        if ended
            || (deadline.is_none() && matches!(state, PlaybackState::Ended | PlaybackState::Error))
        {
            return;
        }
    }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn print(snapshot: &PlayerSnapshot) {
    let title = snapshot
        .track
        .as_ref()
        .map_or("-", |t| t.meta.title.as_str());
    println!(
        "seq={:<3} {:<8} {:>8.3}s rate={} entry={}/{} {title}",
        snapshot.seq,
        format!("{:?}", snapshot.state).to_lowercase(),
        snapshot.position_ms as f64 / 1000.0,
        snapshot.rate,
        snapshot.queue.index + 1,
        snapshot.queue.len,
    );
}

/// The track's length, found by decoding it: a local file is cheap to read through, and the
/// actor needs a duration to prepare a gapless successor.
fn duration_ms(path: &Path) -> Result<u64, BoxError> {
    let hint = path.extension().and_then(|e| e.to_str());
    let summary = canon_audio::decode(std::fs::File::open(path)?, hint)?;
    if summary.sample_rate == 0 {
        return Err("the file reported no sample rate".into());
    }
    Ok(summary.frames_decoded * 1000 / u64::from(summary.sample_rate))
}

/// Carries out the actor's effects on the local output: one file, so every track is that file.
struct Executor {
    player: PlayerHandle,
    path: PathBuf,
    /// The running engine and the generation of its run.
    audio: Mutex<Option<(u64, AudioPlayer)>>,
}

impl Executor {
    fn open(&self) -> std::io::Result<(Box<dyn MediaInput>, Option<String>)> {
        let hint = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_owned);
        Ok((Box::new(std::fs::File::open(&self.path)?), hint))
    }

    async fn run(self: Arc<Self>, mut effects: mpsc::UnboundedReceiver<Effect>) {
        while let Some(effect) = effects.recv().await {
            self.apply(effect).await;
        }
    }

    async fn apply(&self, effect: Effect) {
        let mut audio = self.audio.lock().await;
        match effect {
            Effect::Start {
                generation,
                position,
                ..
            } => {
                if let Some((_, old)) = audio.take() {
                    old.stop();
                }
                let (input, hint) = match self.open() {
                    Ok(opened) => opened,
                    Err(e) => {
                        drop(audio);
                        let failed = EngineEvent::Failed(e.to_string());
                        self.player.engine(generation, failed).await;
                        return;
                    }
                };
                let (events_tx, mut events_rx) = mpsc::unbounded_channel();
                let player = self.player.clone();
                tokio::spawn(async move {
                    while let Some(event) = events_rx.recv().await {
                        player.engine(generation, event).await;
                    }
                });
                let start_ms = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
                let engine = AudioPlayer::start(
                    input,
                    hint,
                    events_tx,
                    start_ms,
                    Some(position),
                    Output::Local,
                );
                let snapshot = self.player.snapshot();
                engine.set_volume(snapshot.volume);
                engine.set_muted(snapshot.muted);
                *audio = Some((generation, engine));
            }
            Effect::Halt { .. } => {
                if let Some((_, old)) = audio.take() {
                    old.stop();
                }
            }
            Effect::Prepare {
                generation,
                next_generation,
                ..
            } => {
                if let Some((current, engine)) = audio.as_ref()
                    && *current == generation
                    && let Ok((input, hint)) = self.open()
                {
                    engine.prepare_next(input, hint, next_generation);
                }
            }
            Effect::Unprepare { next_generation } => {
                if let Some((_, engine)) = audio.as_ref() {
                    engine.cancel_next(next_generation);
                }
            }
            Effect::Pause => {
                if let Some((_, engine)) = audio.as_ref() {
                    engine.pause();
                }
            }
            Effect::Resume => {
                if let Some((_, engine)) = audio.as_ref() {
                    engine.resume();
                }
            }
            Effect::SetVolume(volume) => {
                if let Some((_, engine)) = audio.as_ref() {
                    engine.set_volume(volume);
                }
            }
            Effect::SetMuted(muted) => {
                if let Some((_, engine)) = audio.as_ref() {
                    engine.set_muted(muted);
                }
            }
        }
    }
}
