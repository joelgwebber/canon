//! Who may drive the server (canon-65d2): player tokens, and signed stream URLs.
//!
//! * A **player token** is issued once, by `canon pair <player>` on the server's own machine, and
//!   presented on every connection as `Authorization: Bearer <token>`. The token *is* the player's
//!   identity: a connection carrying one drives that player and no other. Only a SHA-256 of each
//!   token is kept ([`TokenStore`]), so the file holding them grants nothing if read; the store is
//!   read on every connection, so pairing or revoking takes effect at once, without a restart.
//! * A **stream URL** carries its own short-lived token ([`StreamSigner`]): native players want a
//!   bare URL they can hand to their media stack, which can't add headers. It names the player and
//!   the track and expires, signed with HMAC-SHA256 under a key only the server holds.
//!
//! Whether a connection must authenticate is the server's [`Access`] policy: a server bound to
//! loopback trusts whoever can reach it, as it always has (they can read its state directory too);
//! one reachable from the network admits paired players only.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use canon_core::{EntityId, Error, PlayerId, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Who may connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone who can reach the server: it is bound to loopback, so that is this machine.
    Open,
    /// Paired players only, each by its token.
    Paired,
}

/// The server's paired players: a SHA-256 of each one's token, kept in a JSON file.
#[derive(Debug, Clone)]
pub struct TokenStore {
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Paired {
    #[serde(default)]
    players: Vec<PairedPlayer>,
}

/// One paired player, as listed by `canon pair --list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedPlayer {
    pub player: PlayerId,
    /// Hex SHA-256 of the token.
    sha256: String,
    /// Milliseconds since the Unix epoch.
    pub paired_at: u64,
}

impl TokenStore {
    /// The store in `state_dir` (`players.json`).
    #[must_use]
    pub fn in_dir(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join("players.json"),
        }
    }

    /// The player `token` belongs to, if any.
    ///
    /// # Errors
    /// The store exists and can't be read.
    pub fn player_for(&self, token: &str) -> Result<Option<PlayerId>> {
        let hash = hex_sha256(token.as_bytes());
        Ok(self
            .read()?
            .players
            .into_iter()
            .find(|paired| paired.sha256 == hash)
            .map(|paired| paired.player))
    }

    /// Every paired player.
    ///
    /// # Errors
    /// The store exists and can't be read.
    pub fn list(&self) -> Result<Vec<PairedPlayer>> {
        Ok(self.read()?.players)
    }

    /// Pair `player`, returning its new token. A player paired before gets a new token, and its old
    /// one stops working.
    ///
    /// # Errors
    /// The store can't be read or written, or the OS has no randomness.
    pub fn pair(&self, player: &PlayerId) -> Result<String> {
        let token = random_token()?;
        let mut paired = self.read()?;
        paired.players.retain(|p| &p.player != player);
        paired.players.push(PairedPlayer {
            player: player.clone(),
            sha256: hex_sha256(token.as_bytes()),
            paired_at: now_ms(),
        });
        self.write(&paired)?;
        Ok(token)
    }

    /// Unpair `player`: its token stops working. Whether it was paired.
    ///
    /// # Errors
    /// The store can't be read or written.
    pub fn revoke(&self, player: &PlayerId) -> Result<bool> {
        let mut paired = self.read()?;
        let before = paired.players.len();
        paired.players.retain(|p| &p.player != player);
        if paired.players.len() == before {
            return Ok(false);
        }
        self.write(&paired)?;
        Ok(true)
    }

    fn read(&self) -> Result<Paired> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Auth(format!("{}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Paired::default()),
            Err(e) => Err(Error::Auth(format!("{}: {e}", self.path.display()))),
        }
    }

    fn write(&self, paired: &Paired) -> Result<()> {
        let json = serde_json::to_vec_pretty(paired).map_err(|e| Error::Auth(e.to_string()))?;
        write_private(&self.path, &json)
    }
}

/// What a stream URL's token says: which player may fetch which track, until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamClaim {
    pub player: PlayerId,
    pub track: EntityId,
    /// Seconds since the Unix epoch.
    pub expires: u64,
}

/// Signs and checks stream URL tokens, under a key only the server holds.
#[derive(Clone)]
pub struct StreamSigner {
    key: [u8; 32],
}

impl std::fmt::Debug for StreamSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StreamSigner(..)")
    }
}

/// How long a stream URL stays good: long enough to start and to reconnect within a track, short
/// enough that one leaked in a log is soon worthless.
pub const STREAM_TOKEN_TTL: Duration = Duration::from_secs(15 * 60);

impl StreamSigner {
    /// The signer whose key is kept in `state_dir` (`stream.key`), made on first use.
    ///
    /// # Errors
    /// The key can't be read or written, or the OS has no randomness.
    pub fn in_dir(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("stream.key");
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0; 32];
                key.copy_from_slice(&bytes);
                Ok(Self { key })
            }
            Ok(_) => Err(Error::Auth(format!(
                "{} is not a key (want 32 bytes); delete it to make a new one",
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0; 32];
                getrandom::getrandom(&mut key).map_err(|e| Error::Auth(format!("rng: {e}")))?;
                write_private(&path, &key)?;
                Ok(Self { key })
            }
            Err(e) => Err(Error::Auth(format!("{}: {e}", path.display()))),
        }
    }

    /// A signer with `key`, for tests.
    #[must_use]
    pub fn with_key(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// A token letting `player` fetch `track` for the next `ttl`.
    #[must_use]
    pub fn sign(&self, player: &PlayerId, track: EntityId, ttl: Duration) -> String {
        let claim = StreamClaim {
            player: player.clone(),
            track,
            expires: now_secs().saturating_add(ttl.as_secs()),
        };
        let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claim).unwrap_or_default());
        let mac = URL_SAFE_NO_PAD.encode(hmac_sha256(&self.key, body.as_bytes()));
        format!("{body}.{mac}")
    }

    /// What `token` grants, if this server signed it and it hasn't expired.
    ///
    /// # Errors
    /// [`Error::Auth`]: malformed, not signed here, or expired.
    pub fn verify(&self, token: &str) -> Result<StreamClaim> {
        let bad = || Error::Auth("not a stream token from this server".into());
        let (body, mac) = token.split_once('.').ok_or_else(bad)?;
        let mac = URL_SAFE_NO_PAD.decode(mac).map_err(|_| bad())?;
        if !constant_time_eq(&mac, &hmac_sha256(&self.key, body.as_bytes())) {
            return Err(bad());
        }
        let claim: StreamClaim = URL_SAFE_NO_PAD
            .decode(body)
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .ok_or_else(bad)?;
        if claim.expires < now_secs() {
            return Err(Error::Auth("the stream token has expired".into()));
        }
        Ok(claim)
    }
}

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut padded = [0u8; BLOCK];
    padded[..key.len()].copy_from_slice(key);
    let mut inner = Sha256::new();
    inner.update(padded.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(padded.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| Error::Auth(format!("rng: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write `bytes` to `path` atomically, readable by this user only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let fail = |e: std::io::Error| Error::Auth(format!("{}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(fail)?;
    }
    std::fs::rename(&tmp, path).map_err(fail)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("canon-auth-{name}-{}", EntityId::new().0))
    }

    /// Against an independent implementation: `printf canon | openssl dgst -sha256 -mac HMAC
    /// -macopt hexkey:000102…1f`.
    #[test]
    fn hmac_matches_a_known_vector() {
        let key: [u8; 32] = std::array::from_fn(|i| u8::try_from(i).unwrap());
        let mac: String = hmac_sha256(&key, b"canon")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(mac, HMAC_CANON);
    }
    const HMAC_CANON: &str = "efbd97b106420bffa0cfa6d706b197ee2d5fe06a97a2bc2208e3f02e6fddc9d0";

    #[test]
    fn a_paired_token_names_its_player_until_revoked() {
        let store = TokenStore::in_dir(&scratch("pair"));
        let kitchen = PlayerId::parse("kitchen").unwrap();
        assert_eq!(store.player_for("anything").unwrap(), None, "no store yet");
        let token = store.pair(&kitchen).unwrap();
        assert_eq!(store.player_for(&token).unwrap(), Some(kitchen.clone()));
        assert_eq!(store.player_for("guess").unwrap(), None);

        let again = store.pair(&kitchen).unwrap();
        assert_eq!(
            store.player_for(&token).unwrap(),
            None,
            "re-pairing replaces"
        );
        assert_eq!(store.player_for(&again).unwrap(), Some(kitchen.clone()));
        assert_eq!(store.list().unwrap().len(), 1);

        assert!(store.revoke(&kitchen).unwrap());
        assert_eq!(store.player_for(&again).unwrap(), None);
        assert!(!store.revoke(&kitchen).unwrap());
    }

    #[test]
    fn the_store_keeps_no_token() {
        let dir = scratch("hashed");
        let token = TokenStore::in_dir(&dir)
            .pair(&PlayerId::parse("kitchen").unwrap())
            .unwrap();
        let file = std::fs::read_to_string(dir.join("players.json")).unwrap();
        assert!(!file.contains(&token), "{file}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join("players.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "private to this user");
        }
    }

    #[test]
    fn a_stream_token_grants_its_claim_and_nothing_else() {
        let signer = StreamSigner::with_key([7; 32]);
        let kitchen = PlayerId::parse("kitchen").unwrap();
        let track = EntityId::new();
        let token = signer.sign(&kitchen, track, STREAM_TOKEN_TTL);
        let claim = signer.verify(&token).unwrap();
        assert_eq!((claim.player, claim.track), (kitchen.clone(), track));

        assert!(
            StreamSigner::with_key([8; 32]).verify(&token).is_err(),
            "another server's"
        );
        let (body, mac) = token.split_once('.').unwrap();
        let mut forged =
            serde_json::from_slice::<StreamClaim>(&URL_SAFE_NO_PAD.decode(body).unwrap()).unwrap();
        forged.track = EntityId::new();
        let forged = format!(
            "{}.{mac}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap())
        );
        assert!(signer.verify(&forged).is_err(), "a changed claim");
        assert!(signer.verify("nonsense").is_err());

        let expired = signer.sign(&kitchen, track, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1100));
        assert!(signer.verify(&expired).is_err(), "expired");
    }

    #[test]
    fn the_stream_key_is_made_once_and_kept() {
        let dir = scratch("key");
        let first = StreamSigner::in_dir(&dir).unwrap();
        let token = first.sign(&PlayerId::default(), EntityId::new(), STREAM_TOKEN_TTL);
        let second = StreamSigner::in_dir(&dir).unwrap();
        assert!(
            second.verify(&token).is_ok(),
            "the same key after a restart"
        );
    }
}
