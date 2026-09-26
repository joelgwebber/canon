//! The MusicBrainz web service, as the identifier needs it: recordings by ISRC and releases by
//! barcode.
//!
//! MusicBrainz asks for at most one request a second from a client, and a User-Agent naming the
//! application and how to reach whoever runs it
//! (<https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting>). The client paces itself to
//! that, and names canon's repository rather than anything about its user.

use std::time::Duration;

use async_trait::async_trait;
use canon_core::{Error, Result};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;
use uuid::Uuid;

/// A recording as MusicBrainz knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub mbid: Uuid,
    pub title: String,
    pub length_ms: Option<u64>,
    /// Every ISRC the recording is released under.
    pub isrcs: Vec<String>,
    /// The credited artists, in order.
    pub artists: Vec<CreditedArtist>,
}

/// An artist credited on a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedArtist {
    pub mbid: Uuid,
    pub name: String,
}

/// A release (one edition of an album) as MusicBrainz knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub mbid: Uuid,
    /// The release group: every edition of the album.
    pub group: Option<Uuid>,
    pub title: String,
    pub barcode: Option<String>,
}

/// What the identifier asks of MusicBrainz. A trait so it can be scripted in tests.
#[async_trait]
pub trait Lookup: Send + Sync {
    /// The recordings released under `isrc`: usually one, none when MusicBrainz doesn't know it,
    /// and occasionally several (an ISRC reused by mistake, or a recording entered twice).
    async fn recordings_by_isrc(&self, isrc: &str) -> Result<Vec<Recording>>;

    /// The releases carrying `barcode`. Barcodes are compared as numbers, since services pad
    /// them to 13 or 14 digits and MusicBrainz keeps them as printed.
    async fn releases_by_barcode(&self, barcode: &str) -> Result<Vec<Release>>;
}

/// The MusicBrainz web service at musicbrainz.org.
pub struct MusicBrainz {
    http: reqwest::Client,
    base: String,
    /// When the next request may go out.
    next: Mutex<Instant>,
}

/// MusicBrainz allows one request a second on average; a little over keeps clear of it.
const SPACING: Duration = Duration::from_millis(1100);
/// How many times a throttled request is retried, waiting twice as long each time.
const RETRIES: usize = 4;
/// How a throttled (503) request's error ends.
const SLOW_DOWN: &str = "503 (slow down)";

impl MusicBrainz {
    /// A client for musicbrainz.org.
    ///
    /// # Errors
    /// The HTTP client couldn't be built.
    pub fn new() -> Result<Self> {
        Self::at("https://musicbrainz.org/ws/2")
    }

    /// A client for the web service at `base` (a mirror).
    ///
    /// # Errors
    /// The HTTP client couldn't be built.
    pub fn at(base: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!(
                "canon/",
                env!("CARGO_PKG_VERSION"),
                " ( https://github.com/jgw/canon )"
            ))
            .build()
            .map_err(|e| Error::Source(format!("musicbrainz: build client: {e}")))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            next: Mutex::new(Instant::now()),
        })
    }

    /// GET `path` as JSON, no sooner than the pacing allows. `None` for a 404, which is how
    /// MusicBrainz says it has no such ISRC.
    ///
    /// A 503, MusicBrainz's "slow down" (it throttles by overall load, not only per client), is
    /// retried after a growing wait before being given up on as [`Error::Transient`].
    async fn get(&self, path: &str) -> Result<Option<serde_json::Value>> {
        let mut wait = SPACING;
        for _ in 0..RETRIES {
            match self.get_once(path).await {
                Err(Error::Transient(why)) if why.ends_with(SLOW_DOWN) => {
                    tracing::debug!("musicbrainz: {why}; retrying in {wait:?}");
                    tokio::time::sleep(wait).await;
                    wait *= 2;
                }
                reply => return reply,
            }
        }
        self.get_once(path).await
    }

    async fn get_once(&self, path: &str) -> Result<Option<serde_json::Value>> {
        {
            let mut next = self.next.lock().await;
            tokio::time::sleep_until(*next).await;
            *next = Instant::now() + SPACING;
        }
        let url = format!("{}{path}", self.base);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Transient(format!("musicbrainz GET {url}: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Err(Error::Transient(format!(
                "musicbrainz GET {url}: {SLOW_DOWN}"
            )));
        }
        if status.is_server_error() {
            return Err(Error::Transient(format!("musicbrainz GET {url}: {status}")));
        }
        if !status.is_success() {
            return Err(Error::Source(format!("musicbrainz GET {url}: {status}")));
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| Error::Transient(format!("musicbrainz: read body: {e}")))?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| Error::Source(format!("musicbrainz: unexpected reply: {e}")))
    }
}

#[async_trait]
impl Lookup for MusicBrainz {
    async fn recordings_by_isrc(&self, isrc: &str) -> Result<Vec<Recording>> {
        if isrc.is_empty() || !isrc.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Ok(Vec::new());
        }
        let reply = self
            .get(&format!("/isrc/{isrc}?fmt=json&inc=isrcs+artist-credits"))
            .await?;
        reply.map_or(Ok(Vec::new()), recordings)
    }

    async fn releases_by_barcode(&self, barcode: &str) -> Result<Vec<Release>> {
        if barcode.is_empty() || !barcode.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Vec::new());
        }
        let wanted = barcode.trim_start_matches('0');
        let reply = self
            .get(&format!(
                "/release?query=barcode:{wanted}&fmt=json&limit=25"
            ))
            .await?;
        let releases = reply.map_or(Ok(Vec::new()), releases)?;
        Ok(releases
            .into_iter()
            .filter(|release| {
                release
                    .barcode
                    .as_deref()
                    .is_some_and(|found| same_barcode(found, barcode))
            })
            .collect())
    }
}

/// Whether two barcodes are the same number: services pad UPCs to EAN-13 or GTIN-14.
#[must_use]
pub fn same_barcode(a: &str, b: &str) -> bool {
    let a = a.trim().trim_start_matches('0');
    !a.is_empty() && a == b.trim().trim_start_matches('0')
}

#[derive(Deserialize)]
struct IsrcReply {
    #[serde(default)]
    recordings: Vec<RecordingJson>,
}

#[derive(Deserialize)]
struct RecordingJson {
    id: Uuid,
    #[serde(default)]
    title: String,
    length: Option<u64>,
    #[serde(default)]
    isrcs: Vec<String>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<CreditJson>,
}

#[derive(Deserialize)]
struct CreditJson {
    artist: ArtistJson,
}

#[derive(Deserialize)]
struct ArtistJson {
    id: Uuid,
    name: String,
}

#[derive(Deserialize)]
struct ReleaseSearch {
    #[serde(default)]
    releases: Vec<ReleaseJson>,
}

#[derive(Deserialize)]
struct ReleaseJson {
    id: Uuid,
    #[serde(default)]
    title: String,
    barcode: Option<String>,
    #[serde(rename = "release-group")]
    release_group: Option<GroupJson>,
}

#[derive(Deserialize)]
struct GroupJson {
    id: Uuid,
}

fn recordings(json: serde_json::Value) -> Result<Vec<Recording>> {
    let reply: IsrcReply = serde_json::from_value(json)
        .map_err(|e| Error::Source(format!("musicbrainz: unexpected ISRC reply: {e}")))?;
    Ok(reply
        .recordings
        .into_iter()
        .map(|r| Recording {
            mbid: r.id,
            title: r.title,
            length_ms: r.length,
            isrcs: r.isrcs,
            artists: r
                .artist_credit
                .into_iter()
                .map(|credit| CreditedArtist {
                    mbid: credit.artist.id,
                    name: credit.artist.name,
                })
                .collect(),
        })
        .collect())
}

fn releases(json: serde_json::Value) -> Result<Vec<Release>> {
    let reply: ReleaseSearch = serde_json::from_value(json)
        .map_err(|e| Error::Source(format!("musicbrainz: unexpected release search: {e}")))?;
    Ok(reply
        .releases
        .into_iter()
        .map(|r| Release {
            mbid: r.id,
            group: r.release_group.map(|g| g.id),
            title: r.title,
            barcode: r.barcode.filter(|b| !b.is_empty()),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `/isrc/USEE10301026?inc=isrcs+artist-credits` reply.
    #[test]
    fn an_isrc_reply_lists_each_recording_with_all_its_isrcs() {
        let json = serde_json::json!({
            "recordings": [{
                "first-release-date": "1972-09-13",
                "title": "Siberian Khatru",
                "length": 536000,
                "artist-credit": [{
                    "artist": {"name": "Yes", "id": "c1d4f2ba-cf39-460c-9528-6b827d3417a1",
                               "sort-name": "Yes", "type": "Group"},
                    "name": "Yes", "joinphrase": ""
                }],
                "isrcs": ["USEE10301026", "USEW20000054", "USRH12402797"],
                "id": "d8b28a71-9a40-49b1-aa7d-685a2212c3a6",
                "video": false,
                "disambiguation": "original mix"
            }],
            "isrc": "USEE10301026"
        });
        let found = recordings(json).unwrap();
        assert_eq!(found.len(), 1);
        let khatru = &found[0];
        assert_eq!(
            khatru.mbid.to_string(),
            "d8b28a71-9a40-49b1-aa7d-685a2212c3a6"
        );
        assert_eq!(khatru.length_ms, Some(536_000));
        assert_eq!(
            khatru.isrcs,
            ["USEE10301026", "USEW20000054", "USRH12402797"]
        );
        assert_eq!(khatru.artists[0].name, "Yes");
    }

    /// Trimmed from a real `/release?query=barcode:198704317422` reply.
    #[test]
    fn a_barcode_search_names_each_release_and_its_group() {
        let json = serde_json::json!({
            "count": 1,
            "releases": [{
                "id": "cb7ab115-e6d9-48da-a05f-316ded9fca8b",
                "score": 100,
                "title": "Phantom Island",
                "barcode": "198704317422",
                "release-group": {"id": "716f0986-f131-4e3c-a140-55845bbded3c",
                                  "primary-type": "Album"}
            }]
        });
        let found = releases(json).unwrap();
        assert_eq!(found[0].title, "Phantom Island");
        assert_eq!(
            found[0].group.map(|g| g.to_string()).as_deref(),
            Some("716f0986-f131-4e3c-a140-55845bbded3c")
        );
    }

    #[test]
    fn barcodes_compare_as_numbers() {
        assert!(same_barcode("00198704317422", "198704317422"));
        assert!(!same_barcode("198704317422", "198704317423"));
        assert!(!same_barcode("000", "0"));
    }
}
