//! The settings file: `settings.json` in the state directory, the one home of [`Settings`].

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use async_trait::async_trait;
use canon_core::{Error, Result, Settings, SettingsStore};
use tokio::sync::Mutex;

/// Settings held in memory and persisted to a JSON file on every change.
pub struct FileSettings {
    path: PathBuf,
    current: RwLock<Settings>,
    /// Serialises writers, so two replacements can't interleave their writes of the file.
    writing: Mutex<()>,
}

impl FileSettings {
    /// Load the settings file, or start from the defaults if there is none yet.
    ///
    /// # Errors
    /// A file that exists but does not parse is an error, not a reason to start from defaults: a
    /// daemon that quietly discards a hand-edited file (and then overwrites it) is worse than one
    /// that refuses to start and says where the problem is.
    pub async fn load(path: &Path) -> Result<Self> {
        let current = match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                Error::Unsupported(format!("unreadable settings at {}: {e}", path.display()))
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(Error::Io(e)),
        };
        Ok(Self {
            path: path.to_path_buf(),
            current: RwLock::new(current),
            writing: Mutex::new(()),
        })
    }
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
        let bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|e| Error::Unsupported(format!("serialize settings: {e}")))?;
        // A sibling temp file renamed over the target, so a reader never sees half a file.
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        *self.current.write().expect("settings lock poisoned") = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use canon_core::{OutputMode, OutputSettings, SinkId};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("canon-settings-{name}-{nanos}/settings.json"))
    }

    #[tokio::test]
    async fn settings_survive_a_restart() {
        let path = scratch("round-trip");
        let store = FileSettings::load(&path).await.unwrap();
        assert_eq!(store.get(), Settings::default(), "no file: defaults");

        let mut settings = Settings::default();
        settings.outputs.insert(
            "Kitchen".into(),
            OutputSettings {
                mode: OutputMode::Standard,
            },
        );
        store.set(settings.clone()).await.unwrap();

        let reloaded = FileSettings::load(&path).await.unwrap();
        assert_eq!(reloaded.get(), settings);
        assert_eq!(
            reloaded.get().output(&SinkId("Kitchen".into())).mode,
            OutputMode::Standard
        );
    }

    #[tokio::test]
    async fn an_unreadable_file_stops_the_daemon_instead_of_being_replaced() {
        let path = scratch("corrupt");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, br#"{"outputs": {"x": {"moed": "flow"}}}"#)
            .await
            .unwrap();
        let error = FileSettings::load(&path).await.err().expect("refused");
        assert!(error.to_string().contains("settings.json"), "{error}");
    }
}
