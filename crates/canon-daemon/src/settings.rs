//! The settings file: `settings.yaml`, the one home of [`Settings`].
//!
//! It lives in the platform's config directory (`~/.config/canon` on Linux, `~/Library/Application
//! Support/canon` on macOS), unless the state directory was named explicitly, in which case it
//! lives there: a daemon given its own `--state-dir` (a test, a signed-out check) gets its own
//! settings too. See [`path`].
//!
//! The file is written on first start with every setting and a comment for each, so a user edits
//! a value rather than writing a document from scratch. Every later write renders the same
//! commented document, so changing a setting from a client keeps the comments.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use async_trait::async_trait;
use canon_core::{Error, Result, Settings, SettingsStore};
use serde::Serialize;
use tokio::sync::Mutex;

/// The file's name, wherever it lives.
const FILE: &str = "settings.yaml";

/// Where the settings file lives. `explicit_state_dir` is the state directory if the user named
/// one (`--state-dir` / `CANON_STATE_DIR`); `state_dir` is the one in use either way.
pub fn path(explicit_state_dir: Option<&Path>, state_dir: &Path) -> PathBuf {
    if let Some(dir) = explicit_state_dir {
        return dir.join(FILE);
    }
    directories::ProjectDirs::from("", "", "canon")
        .map_or_else(
            || state_dir.to_path_buf(),
            |dirs| dirs.config_dir().to_path_buf(),
        )
        .join(FILE)
}

/// Settings held in memory and persisted to a YAML file on every change.
pub struct FileSettings {
    path: PathBuf,
    current: RwLock<Settings>,
    /// Serialises writers, so two replacements can't interleave their writes of the file.
    writing: Mutex<()>,
}

impl FileSettings {
    /// Load the settings file. If there is none yet, write one: the settings in `legacy` (the
    /// `settings.json` older builds kept in the state directory) if that exists, else the
    /// defaults. `legacy` is left in place, so an older build still finds its settings.
    ///
    /// # Errors
    /// A file that exists but does not parse is an error, not a reason to start from defaults: a
    /// daemon that quietly discards a hand-edited file (and then overwrites it) is worse than one
    /// that refuses to start and says where the problem is.
    pub async fn load(path: &Path, legacy: &Path) -> Result<Self> {
        let existing = match tokio::fs::read(path).await {
            Ok(bytes) => Some(parse(path, &bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Io(e)),
        };
        let store = Self {
            path: path.to_path_buf(),
            current: RwLock::new(existing.clone().unwrap_or_default()),
            writing: Mutex::new(()),
        };
        if existing.is_none() {
            // JSON is YAML, so the old file parses as it is.
            let settings = match tokio::fs::read(legacy).await {
                Ok(bytes) => {
                    tracing::info!(
                        "settings: copying {} to {}",
                        legacy.display(),
                        path.display()
                    );
                    parse(legacy, &bytes)?
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
                Err(e) => return Err(Error::Io(e)),
            };
            store.set(settings).await?;
        }
        Ok(store)
    }
}

fn parse(path: &Path, bytes: &[u8]) -> Result<Settings> {
    serde_norway::from_slice(bytes)
        .map_err(|e| Error::Unsupported(format!("unreadable settings at {}: {e}", path.display())))
}

#[async_trait]
impl SettingsStore for FileSettings {
    fn get(&self) -> Settings {
        self.current.read().expect("settings lock poisoned").clone()
    }

    /// Persist first, then publish: if the write fails, nothing has changed.
    async fn set(&self, settings: Settings) -> Result<()> {
        let _writing = self.writing.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = render(&settings)?;
        // A sibling temp file renamed over the target, so a reader never sees half a file.
        let tmp = self.path.with_extension("yaml.tmp");
        tokio::fs::write(&tmp, text).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        *self.current.write().expect("settings lock poisoned") = settings;
        Ok(())
    }

    fn location(&self) -> Option<PathBuf> {
        Some(self.path.clone())
    }
}

/// The settings as the commented document the file holds: every section, each under a comment
/// saying what it does, with its current value (defaults included, so every key is there to edit).
fn render(settings: &Settings) -> Result<String> {
    // Named field by field, so a new setting can't be added without deciding how it reads here.
    let Settings {
        outputs,
        queue,
        spotify,
        streaming,
        library,
    } = settings;
    let mut out = String::from(
        "# canon's settings. canon reads this file when it starts, and writes it whenever a\n\
         # setting is changed from a client (the TUI, `canon control`, ...), keeping these\n\
         # comments but nothing else you add. Edit it while canon is stopped.\n",
    );

    out.push_str(
        "\n# Spotify: your library and browsing (spotify.web). Spotify only serves developer apps\n\
         # their users register, so canon needs yours: create one at\n\
         # https://developer.spotify.com/dashboard (its owner needs Premium), add\n\
         # http://127.0.0.1:8898/spotify/callback as a redirect URI, and put its client id here.\n\
         # Read at each sign-in, so it needs no restart.\n",
    );
    match &spotify.client_id {
        Some(_) => out.push_str(&section("spotify", spotify)?),
        // An empty value is YAML's null, so the key is there to fill in and still reads as unset.
        None => out.push_str("spotify:\n  client_id:\n"),
    }

    out.push_str(
        "\n# Streaming services, most preferred first. A track plays from the first of these that\n\
         # has it, and one queued from a later service is matched (by ISRC) onto an earlier one.\n\
         # Local files always come first. Applies from the next track queued.\n",
    );
    out.push_str(&section("streaming", streaming)?);

    out.push_str(
        "\n# autoplay: when the last track in the queue starts, add tracks like it (the service's\n\
         # radio), so playback carries on instead of stopping.\n",
    );
    out.push_str(&section("queue", queue)?);

    out.push_str(
        "\n# identify: look tracks and albums up in MusicBrainz in the background, by ISRC and\n\
         # barcode, to learn their MusicBrainz ids and every ISRC a recording is released under.\n\
         # Sends those ISRCs and barcodes to musicbrainz.org, at most one request a second.\n",
    );
    out.push_str(&section("library", library)?);

    out.push_str(
        "\n# Per-output preferences, keyed by output id (`mode <output> flow|standard` in\n\
         # `canon control` sets these). mode is `flow`, one continuous gapless stream (the\n\
         # speaker's display shows the first track), or `standard`, a stream per track (the\n\
         # display is right, with a gap between tracks). For example:\n\
         #   outputs:\n\
         #     LS50-Wireless-II-0123456789abcdef:\n\
         #       mode: standard\n",
    );
    out.push_str(&section("outputs", outputs)?);
    Ok(out)
}

/// `key: value` as YAML.
fn section<T: Serialize>(key: &str, value: &T) -> Result<String> {
    serde_norway::to_string(&std::collections::BTreeMap::from([(key, value)]))
        .map_err(|e| Error::Unsupported(format!("serialize settings: {e}")))
}

#[cfg(test)]
mod tests {
    use canon_core::{OutputMode, OutputSettings, Service, SinkId};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("canon-settings-{name}-{nanos}"))
    }

    fn everything_changed() -> Settings {
        let mut settings = Settings::default();
        settings.outputs.insert(
            "Kitchen".into(),
            OutputSettings {
                mode: OutputMode::Standard,
            },
        );
        settings.queue.autoplay = true;
        settings.spotify.client_id = Some("abc123".into());
        settings.streaming.order = vec![Service::Spotify, Service::Tidal];
        settings.library.identify = false;
        settings
    }

    #[test]
    fn the_document_reads_back_as_what_was_written() {
        for settings in [Settings::default(), everything_changed()] {
            let text = render(&settings).unwrap();
            assert_eq!(
                parse(Path::new(FILE), text.as_bytes()).unwrap(),
                settings,
                "{text}"
            );
        }
    }

    /// The point of writing the file at first start: every setting is there to edit.
    #[test]
    fn the_default_document_names_every_setting() {
        let text = render(&Settings::default()).unwrap();
        for key in [
            "\nspotify:\n  client_id:\n",
            "\nstreaming:\n  order:\n  - tidal\n  - spotify\n",
            "\nqueue:\n  autoplay: false\n",
            "\nlibrary:\n  identify: true\n",
            "\noutputs: {}\n",
        ] {
            assert!(text.contains(key), "{key:?} missing from\n{text}");
        }
    }

    #[tokio::test]
    async fn first_start_writes_the_defaults_and_settings_survive_a_restart() {
        let dir = scratch("round-trip");
        let path = dir.join(FILE);
        let store = FileSettings::load(&path, &dir.join("settings.json"))
            .await
            .unwrap();
        assert_eq!(store.get(), Settings::default(), "no file: defaults");
        assert_eq!(store.location(), Some(path.clone()));
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(written, render(&Settings::default()).unwrap());

        store.set(everything_changed()).await.unwrap();
        let reloaded = FileSettings::load(&path, &dir.join("settings.json"))
            .await
            .unwrap();
        assert_eq!(reloaded.get(), everything_changed());
        assert_eq!(
            reloaded.get().output(&SinkId("Kitchen".into())).mode,
            OutputMode::Standard
        );
    }

    #[tokio::test]
    async fn an_old_settings_json_is_copied_and_left_in_place() {
        let dir = scratch("legacy");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let legacy = dir.join("settings.json");
        let json = "{\n  \"queue\": {\n    \"autoplay\": true\n  }\n}";
        tokio::fs::write(&legacy, json).await.unwrap();

        let path = dir.join("config").join(FILE);
        let store = FileSettings::load(&path, &legacy).await.unwrap();
        assert!(store.get().queue.autoplay);
        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            written.contains("\nqueue:\n  autoplay: true\n"),
            "{written}"
        );
        assert_eq!(tokio::fs::read_to_string(&legacy).await.unwrap(), json);
    }

    /// Once the YAML file exists, the old one is never read again.
    #[tokio::test]
    async fn an_existing_yaml_file_wins_over_the_old_json() {
        let dir = scratch("both");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let legacy = dir.join("settings.json");
        tokio::fs::write(&legacy, r#"{"queue":{"autoplay":true}}"#)
            .await
            .unwrap();
        let path = dir.join(FILE);
        tokio::fs::write(&path, "library:\n  identify: false\n")
            .await
            .unwrap();

        let store = FileSettings::load(&path, &legacy).await.unwrap();
        assert!(!store.get().queue.autoplay);
        assert!(!store.get().library.identify);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "library:\n  identify: false\n",
            "a file that parses is not rewritten on load"
        );
    }

    #[tokio::test]
    async fn an_unreadable_file_stops_the_daemon_instead_of_being_replaced() {
        let dir = scratch("corrupt");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(FILE);
        tokio::fs::write(&path, "outputs:\n  x:\n    moed: flow\n")
            .await
            .unwrap();
        let error = FileSettings::load(&path, &dir.join("settings.json"))
            .await
            .err()
            .expect("refused");
        assert!(error.to_string().contains(FILE), "{error}");
    }

    #[test]
    fn an_explicit_state_dir_keeps_the_settings_with_it() {
        let dir = Path::new("/srv/canon");
        assert_eq!(path(Some(dir), dir), dir.join(FILE));
        let elsewhere = path(None, dir);
        assert!(
            elsewhere.ends_with(Path::new("canon").join(FILE)),
            "{}",
            elsewhere.display()
        );
    }
}
