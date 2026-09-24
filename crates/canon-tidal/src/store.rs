//! Token persistence for the Tidal session (yak canon-8bab).
//!
//! The whole point of capturing a *rotating* refresh token (see [`crate::auth`]) is
//! lost if it evaporates when the daemon restarts, so the session writes its token pair
//! to a small JSON file and reloads it on startup. Losing the persisted refresh token
//! is exactly the "silently logged out after a restart" failure this guards against, so
//! writes are atomic (temp file + rename) — a crash mid-write can't leave a truncated
//! file that reads back as a corrupt, unrefreshable session.

use std::path::{Path, PathBuf};

use canon_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// The on-disk token record. Access-token expiry is stored as an absolute unix time so
/// a freshness check survives across restarts (a relative `expires_in` would not).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Absolute access-token expiry, seconds since the unix epoch.
    pub expires_at_unix: u64,
    #[serde(default)]
    pub user_id: Option<i64>,
    /// Whether these tokens were minted by the PKCE (streaming) client. Refresh uses a
    /// different client for PKCE vs device-code, so this must survive a restart.
    #[serde(default)]
    pub is_pkce: bool,
}

/// A JSON-file token store. Cheap to clone (just a path); all I/O is explicit.
#[derive(Debug, Clone)]
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    /// Persist tokens to `path`. The parent directory is created on first save.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Where tokens are stored.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the persisted tokens, or `None` if none have been saved yet. A malformed
    /// file is an [`Error::Auth`]: the caller must re-run the device flow rather than
    /// silently proceed unauthenticated.
    pub async fn load(&self) -> Result<Option<PersistedTokens>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                let tokens = serde_json::from_slice(&bytes).map_err(|e| {
                    Error::Auth(format!(
                        "corrupt token store at {}: {e}",
                        self.path.display()
                    ))
                })?;
                Ok(Some(tokens))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Remove the saved tokens, if any.
    ///
    /// # Errors
    /// The file exists but couldn't be removed.
    pub async fn clear(&self) -> Result<()> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Persist tokens atomically: write a sibling temp file, then rename it over the
    /// target so a reader never sees a half-written file.
    pub async fn save(&self, tokens: &PersistedTokens) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(tokens)
            .map_err(|e| Error::Auth(format!("serialize tokens: {e}")))?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("canon-store-{}", unique()));
        let store = TokenStore::new(dir.join("tidal.json"));

        assert!(store.load().await.unwrap().is_none()); // absent → None

        let tokens = PersistedTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_unix: 1_700_000_000,
            user_id: Some(42),
            is_pkce: true,
        };
        store.save(&tokens).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tokens));

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn corrupt_file_is_an_auth_error() {
        let dir = std::env::temp_dir().join(format!("canon-store-{}", unique()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("tidal.json");
        tokio::fs::write(&path, b"{not json").await.unwrap();

        let store = TokenStore::new(&path);
        assert!(matches!(store.load().await, Err(Error::Auth(_))));

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    // A unique-per-call suffix: a process-wide counter defeats same-nanosecond
    // collisions when tests run in parallel.
    fn unique() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }
}
