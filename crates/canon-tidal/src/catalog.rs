//! Tidal's catalog: its v1 JSON, read into canon's service-neutral descriptions, and the
//! [`Catalog`] calls (search, albums, artists, radio, similar artists) over it.
//!
//! Only the fields canon reads are modelled; everything else in a reply is ignored, so a field
//! Tidal adds never breaks a call.

use async_trait::async_trait;
use canon_core::{
    AlbumListing, ArtistListing, Catalog, Error, Result, SearchResults, Seed, Service, SourceAlbum,
    SourceArtist, SourceRef, SourceTrack,
};
use serde::Deserialize;

use crate::{TidalSession, TidalSource};

/// How many entries Tidal returns per page at most.
const PAGE: usize = 100;
/// Releases listed per kind (albums, then EPs and singles) on an artist page.
const ARTIST_RELEASES: usize = 50;
const TOP_TRACKS: usize = 10;
const RADIO_TRACKS: usize = 50;

/// A track, as `/v1/tracks/<id>` and every track list return it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackInfo {
    id: Option<u64>,
    title: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default)]
    track_number: Option<u32>,
    #[serde(default)]
    volume_number: Option<u32>,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(default)]
    artists: Option<Vec<ArtistInfo>>,
    #[serde(default)]
    album: Option<AlbumInfo>,
}

#[derive(Debug, Deserialize)]
struct ArtistInfo {
    #[serde(default)]
    id: Option<u64>,
    name: String,
}

/// An album: in full from `/v1/albums/<id>`, abbreviated (id, title, cover) inside a track.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumInfo {
    #[serde(default)]
    id: Option<u64>,
    title: String,
    #[serde(default)]
    version: Option<String>,
    /// An image id: a UUID whose dashes become path separators in the image URL.
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    upc: Option<String>,
    #[serde(default)]
    artists: Option<Vec<ArtistInfo>>,
}

/// A page of a list.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    total_number_of_items: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchReply {
    #[serde(default)]
    tracks: Option<Page<TrackInfo>>,
    #[serde(default)]
    albums: Option<Page<AlbumInfo>>,
    #[serde(default)]
    artists: Option<Page<ArtistInfo>>,
}

fn tidal(id: u64) -> SourceRef {
    SourceRef::Tidal { id: id.to_string() }
}

/// "Money" and "2011 Remastered Version" as Tidal splits them, as a listener reads them. Some
/// titles already carry their version, and don't get it twice.
fn titled(title: String, version: Option<String>) -> String {
    match version.filter(|v| !v.is_empty()) {
        Some(version) if !title.to_lowercase().contains(&version.to_lowercase()) => {
            format!("{title} ({version})")
        }
        _ => title,
    }
}

impl ArtistInfo {
    fn describe(self) -> SourceArtist {
        SourceArtist {
            source: self.id.map(tidal),
            name: self.name,
        }
    }
}

impl AlbumInfo {
    fn describe(self) -> SourceAlbum {
        SourceAlbum {
            source: self.id.map(tidal),
            title: titled(self.title, self.version),
            artists: self
                .artists
                .unwrap_or_default()
                .into_iter()
                .map(ArtistInfo::describe)
                .collect(),
            release_date: self.release_date.filter(|d| !d.is_empty()),
            barcode: self.upc.filter(|upc| !upc.is_empty()),
            artwork_url: self.cover.as_deref().map(cover_url),
        }
    }
}

impl TrackInfo {
    /// The description of this track. `id` names it when the reply doesn't carry its own.
    fn describe(self, id: Option<&str>) -> Option<SourceTrack> {
        let id = self.id.map(|id| id.to_string()).or(id.map(str::to_owned))?;
        Some(SourceTrack {
            source: SourceRef::Tidal { id },
            title: titled(self.title, self.version),
            artists: self
                .artists
                .unwrap_or_default()
                .into_iter()
                .map(ArtistInfo::describe)
                .collect(),
            album: self.album.map(AlbumInfo::describe),
            disc: self.volume_number,
            position: self.track_number,
            duration_ms: self.duration.map(|s| s * 1000),
            isrc: self.isrc.filter(|isrc| !isrc.is_empty()),
        })
    }
}

fn tracks(items: Vec<TrackInfo>) -> Vec<SourceTrack> {
    items
        .into_iter()
        .filter_map(|track| track.describe(None))
        .collect()
}

/// Tidal's image URL for a cover id, at the largest square size every client accepts.
fn cover_url(cover: &str) -> String {
    format!(
        "https://resources.tidal.com/images/{}/640x640.jpg",
        cover.replace('-', "/")
    )
}

/// The Tidal id in a binding, or why a Tidal call can't take it.
pub(crate) fn tidal_id(source: &SourceRef) -> Result<&str> {
    match source {
        SourceRef::Tidal { id } => Ok(id),
        other => Err(Error::Unsupported(format!(
            "canon-tidal cannot use a {} binding",
            other.service()
        ))),
    }
}

impl TidalSession {
    /// What Tidal says about a track id.
    pub async fn describe(&self, id: &str) -> Result<SourceTrack> {
        let track: TrackInfo = self.api_get(&format!("/v1/tracks/{id}"), &[]).await?;
        track
            .describe(Some(id))
            .ok_or_else(|| Error::Source(format!("tidal track {id} has no id")))
    }

    /// Every page of a list, up to `max` items.
    async fn all<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        max: usize,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        loop {
            let offset = items.len().to_string();
            let limit = PAGE.min(max - items.len()).to_string();
            let mut paged = query.to_vec();
            paged.push(("offset", &offset));
            paged.push(("limit", &limit));
            let page: Page<T> = self.api_get(path, &paged).await?;
            let got = page.items.len();
            items.extend(page.items);
            let total = page.total_number_of_items.unwrap_or(items.len());
            if got == 0 || items.len() >= total.min(max) {
                return Ok(items);
            }
        }
    }
}

#[async_trait]
impl Catalog for TidalSource {
    fn service(&self) -> Service {
        Service::Tidal
    }

    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults> {
        let limit = limit.clamp(1, PAGE).to_string();
        let reply: SearchReply = self
            .session()
            .api_get(
                "/v1/search",
                &[
                    ("query", query),
                    ("types", "TRACKS,ALBUMS,ARTISTS"),
                    ("limit", &limit),
                ],
            )
            .await?;
        Ok(SearchResults {
            tracks: tracks(reply.tracks.map(|p| p.items).unwrap_or_default()),
            albums: reply
                .albums
                .map(|p| p.items)
                .unwrap_or_default()
                .into_iter()
                .map(AlbumInfo::describe)
                .collect(),
            artists: reply
                .artists
                .map(|p| p.items)
                .unwrap_or_default()
                .into_iter()
                .map(ArtistInfo::describe)
                .collect(),
        })
    }

    async fn album(&self, album: &SourceRef) -> Result<AlbumListing> {
        let id = tidal_id(album)?;
        let session = self.session();
        let info: AlbumInfo = session.api_get(&format!("/v1/albums/{id}"), &[]).await?;
        let items: Vec<TrackInfo> = session
            .all(&format!("/v1/albums/{id}/tracks"), &[], 1_000)
            .await?;
        Ok(AlbumListing {
            album: info.describe(),
            tracks: tracks(items),
        })
    }

    async fn artist(&self, artist: &SourceRef) -> Result<ArtistListing> {
        let id = tidal_id(artist)?;
        let session = self.session();
        let info: ArtistInfo = session.api_get(&format!("/v1/artists/{id}"), &[]).await?;
        let albums_path = format!("/v1/artists/{id}/albums");
        let mut albums: Vec<AlbumInfo> = session.all(&albums_path, &[], ARTIST_RELEASES).await?;
        let singles: Vec<AlbumInfo> = session
            .all(
                &albums_path,
                &[("filter", "EPSANDSINGLES")],
                ARTIST_RELEASES,
            )
            .await?;
        albums.extend(singles);
        let top: Page<TrackInfo> = session
            .api_get(
                &format!("/v1/artists/{id}/toptracks"),
                &[("limit", &TOP_TRACKS.to_string())],
            )
            .await?;
        Ok(ArtistListing {
            artist: info.describe(),
            albums: albums.into_iter().map(AlbumInfo::describe).collect(),
            top_tracks: tracks(top.items),
        })
    }

    async fn radio(&self, seed: &Seed) -> Result<Vec<SourceTrack>> {
        let path = match seed {
            Seed::Track(track) => format!("/v1/tracks/{}/radio", tidal_id(track)?),
            Seed::Artist(artist) => format!("/v1/artists/{}/radio", tidal_id(artist)?),
        };
        let page: Page<TrackInfo> = self
            .session()
            .api_get(&path, &[("limit", &RADIO_TRACKS.to_string())])
            .await?;
        Ok(tracks(page.items))
    }

    async fn similar_artists(&self, artist: &SourceRef) -> Result<Vec<SourceArtist>> {
        let id = tidal_id(artist)?;
        let page: Page<ArtistInfo> = self
            .session()
            .api_get(&format!("/v1/artists/{id}/similar"), &[])
            .await?;
        Ok(page.items.into_iter().map(ArtistInfo::describe).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like a `/v1/tracks/<id>` reply (the values are illustrative), trimmed to what
    /// canon reads plus fields it ignores.
    #[test]
    fn a_track_reply_describes_the_recording_and_where_it_sits() {
        let json = r#"{
            "id": 55391792, "title": "Money", "duration": 382, "trackNumber": 6,
            "volumeNumber": 1, "isrc": "GBN9Y1100086", "explicit": false, "version": null,
            "artists": [{"id": 9706, "name": "Pink Floyd", "type": "MAIN"}],
            "album": {"id": 55391786, "title": "The Dark Side of the Moon",
                      "cover": "b3ae83ed-8c5c-4c29-8b0a-fb8d10c8e3e7"}
        }"#;
        let track: TrackInfo = serde_json::from_str(json).unwrap();
        let described = track.describe(None).unwrap();
        assert_eq!(
            described.source,
            SourceRef::Tidal {
                id: "55391792".into()
            }
        );
        assert_eq!(described.title, "Money");
        assert_eq!(described.isrc.as_deref(), Some("GBN9Y1100086"));
        assert_eq!(described.duration_ms, Some(382_000));
        assert_eq!((described.disc, described.position), (Some(1), Some(6)));
        assert_eq!(
            described.artists[0].source,
            Some(SourceRef::Tidal { id: "9706".into() })
        );
        let album = described.album.expect("album");
        assert_eq!(
            album.source,
            Some(SourceRef::Tidal {
                id: "55391786".into()
            })
        );
        assert_eq!(
            album.artwork_url.as_deref(),
            Some(
                "https://resources.tidal.com/images/b3ae83ed/8c5c/4c29/8b0a/fb8d10c8e3e7/640x640.jpg"
            )
        );
    }

    #[test]
    fn an_album_reply_carries_its_credits_date_and_barcode() {
        let json = r#"{
            "id": 55391786, "title": "The Dark Side of the Moon", "version": "2011 Remaster",
            "releaseDate": "1973-03-01", "upc": "5099902987613", "numberOfTracks": 10,
            "cover": "05ccaf43-2f3f-4de2-8d4b-23e9dc56d831",
            "artists": [{"id": 9706, "name": "Pink Floyd", "type": "MAIN"}]
        }"#;
        let album = serde_json::from_str::<AlbumInfo>(json).unwrap().describe();
        assert_eq!(album.title, "The Dark Side of the Moon (2011 Remaster)");
        assert_eq!(album.release_date.as_deref(), Some("1973-03-01"));
        assert_eq!(album.barcode.as_deref(), Some("5099902987613"));
        assert_eq!(album.artists[0].name, "Pink Floyd");
    }

    #[test]
    fn a_version_already_in_the_title_is_not_added_again() {
        assert_eq!(
            titled("Pompeii (2025 Mix)".into(), Some("2025 Mix".into())),
            "Pompeii (2025 Mix)"
        );
        assert_eq!(
            titled("Animals".into(), Some("2018 Remix".into())),
            "Animals (2018 Remix)"
        );
        assert_eq!(titled("Animals".into(), Some(String::new())), "Animals");
    }

    #[test]
    fn a_search_reply_with_a_missing_section_is_just_empty() {
        let json = r#"{"tracks": {"limit": 5, "offset": 0, "totalNumberOfItems": 1,
                        "items": [{"id": 1, "title": "Army of Me"}]}}"#;
        let reply: SearchReply = serde_json::from_str(json).unwrap();
        assert_eq!(tracks(reply.tracks.unwrap().items)[0].title, "Army of Me");
        assert!(reply.albums.is_none());
    }
}
