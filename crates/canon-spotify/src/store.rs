//! The token file, at a path the caller chooses.
//!
//! Losing a refresh token means a trip through the browser, so writes are atomic (a sibling temp
//! file renamed over the target): a crash mid-write can't leave half a file.

use std::path::{Path, PathBuf};

use canon_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::auth::PendingLogin;

/// What is kept between runs. Expiry is absolute so freshness survives a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds since the Unix epoch.
    pub expires_at_unix: u64,
    /// The scopes Spotify granted, space-separated.
    #[serde(default)]
    pub scope: Option<String>,
    /// The client id the tokens were issued to: a refresh token only refreshes with its own client,
    /// so tokens from another app id are as good as none.
    pub client_id: String,
}

/// A JSON token file, plus a sibling file for a login in flight. Cheap to clone.
#[derive(Debug, Clone)]
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved tokens, or `None` before the first login. A file that doesn't parse is an
    /// [`Error::Auth`]: log in again rather than silently run signed out.
    pub async fn load(&self) -> Result<Option<PersistedTokens>> {
        read_json(&self.path).await
    }

    pub async fn save(&self, tokens: &PersistedTokens) -> Result<()> {
        write_json(&self.path, tokens).await
    }

    /// Delete the token file. Already gone is fine.
    ///
    /// # Errors
    /// The file exists and couldn't be removed.
    pub async fn clear(&self) -> Result<()> {
        match tokio::fs::remove_file(&self.path).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::Io(e)),
            _ => Ok(()),
        }
    }

    /// Where a login in flight is parked: `spotify.json` → `spotify.pending.json`.
    fn pending_path(&self) -> PathBuf {
        self.path.with_extension("pending.json")
    }

    pub(crate) async fn load_pending(&self) -> Result<Option<PendingLogin>> {
        read_json(&self.pending_path()).await
    }

    pub(crate) async fn save_pending(&self, pending: &PendingLogin) -> Result<()> {
        write_json(&self.pending_path(), pending).await
    }

    pub(crate) async fn clear_pending(&self) {
        let _ = tokio::fs::remove_file(self.pending_path()).await;
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::Auth(format!("unreadable {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

async fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| Error::Auth(format!("serialize {}: {e}", path.display())))?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "canon-spotify-{name}-{}-{}/spotify.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tokens_round_trip_through_the_file() {
        let store = TokenStore::new(scratch("store"));
        assert_eq!(store.load().await.unwrap(), None);
        let tokens = PersistedTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_unix: 1_800_000_000,
            scope: Some("user-library-read".into()),
            client_id: "client".into(),
        };
        store.save(&tokens).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(tokens));
        assert!(!store.path().with_extension("json.tmp").exists());
    }

    #[tokio::test]
    async fn a_corrupt_file_is_an_auth_error() {
        let store = TokenStore::new(scratch("corrupt"));
        tokio::fs::create_dir_all(store.path().parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(store.path(), b"{not json").await.unwrap();
        assert!(matches!(store.load().await, Err(Error::Auth(_))));
    }
}
