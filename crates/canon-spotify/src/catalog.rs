//! Spotify's catalog as a development-mode app sees it, read into canon's service-neutral
//! descriptions.
//!
//! The February 2026 rules for development-mode apps
//! (developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide) shape
//! every call here:
//!
//! * Search returns at most 10 per type per request, so it pages by offset.
//! * The batch GETs (`/tracks?ids=`, `/albums?ids=`, `/artists?ids=`) are gone: items are fetched
//!   one at a time.
//! * `/artists/{id}/top-tracks`, recommendations, related artists and browse are gone: an artist's
//!   top tracks are empty, and radio, similar artists and mixes are [`Error::Unsupported`].
//! * Playlist contents moved to `/playlists/{id}/items`, each entry's `track` became `item`, and
//!   they come back only for playlists the user owns or collaborates on.
//! * `external_ids` (ISRC on tracks, UPC on albums) were removed and then restored in March 2026
//!   (references/changes/march-2026). Simplified tracks (an album's listing) never had them.
//!
//! Only the fields canon reads are modelled; anything else in a reply is ignored.

use async_trait::async_trait;
use canon_core::{
    AlbumListing, ArtistListing, Catalog, Error, Favorite, Favorites, Result, SearchResults, Seed,
    Service, SourceAlbum, SourceArtist, SourceMix, SourcePlaylist, SourceRef, SourceTrack,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::SpotifySession;
use crate::session::api_url;

/// Search's per-request ceiling for development-mode apps.
const SEARCH_PAGE: usize = 10;
/// The most search results of each kind one call pages through (five requests).
const SEARCH_MAX: usize = 50;
/// The per-request ceiling on the library and playlist lists.
const PAGE: usize = 50;
/// `/artists/{id}/albums` allows 10 per request.
const ARTIST_ALBUMS_PAGE: usize = 10;
/// Releases listed per kind (albums, then singles and EPs) on an artist page.
const ARTIST_RELEASES: usize = 50;
/// Albums up to this long get each track fetched on its own for its ISRC; see [`Catalog::album`].
const ALBUM_ISRC_LOOKUPS: usize = 40;
/// Ceilings on what an import pulls, so a vast account can't stall the daemon.
const FAVORITES: usize = 5_000;
const PLAYLISTS: usize = 500;
const PLAYLIST_TRACKS: usize = 5_000;

const UNAVAILABLE: &str = "not available to Spotify development-mode apps";

#[derive(Debug, Deserialize)]
struct Image {
    url: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct ExternalIds {
    #[serde(default)]
    isrc: Option<String>,
    #[serde(default)]
    upc: Option<String>,
    #[serde(default)]
    ean: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistInfo {
    #[serde(default)]
    id: Option<String>,
    name: String,
}

/// An album: full from `/albums/{id}` and `/me/albums`, simplified inside a track or a list.
#[derive(Debug, Deserialize)]
struct AlbumInfo {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    artists: Vec<ArtistInfo>,
    /// `1981`, `1981-12` or `1981-12-02`, by `release_date_precision`.
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    images: Vec<Image>,
    #[serde(default)]
    external_ids: Option<ExternalIds>,
    /// The first page of the tracklist, on the full object only.
    #[serde(default)]
    tracks: Option<Page<TrackInfo>>,
}

/// A track: full (with `album` and `external_ids`) or simplified (an album's listing).
#[derive(Debug, Deserialize)]
struct TrackInfo {
    /// Null for a local file in a playlist.
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    artists: Vec<ArtistInfo>,
    #[serde(default)]
    album: Option<AlbumInfo>,
    #[serde(default)]
    disc_number: Option<u32>,
    #[serde(default)]
    track_number: Option<u32>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    external_ids: Option<ExternalIds>,
    #[serde(default)]
    is_local: bool,
}

/// Spotify's paging object. `next` is the whole URL of the next page, or null at the end.
#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    total: Option<usize>,
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

#[derive(Debug, Deserialize)]
struct SavedTrack {
    #[serde(default)]
    added_at: Option<String>,
    track: TrackInfo,
}

#[derive(Debug, Deserialize)]
struct SavedAlbum {
    #[serde(default)]
    added_at: Option<String>,
    album: AlbumInfo,
}

/// `/me/following?type=artist` wraps its cursor-paged list in `artists`.
#[derive(Debug, Deserialize)]
struct FollowedReply {
    artists: Page<ArtistInfo>,
}

#[derive(Debug, Deserialize)]
struct PlaylistInfo {
    id: String,
    name: String,
    #[serde(default)]
    collaborative: bool,
    owner: Owner,
}

#[derive(Debug, Deserialize)]
struct Owner {
    id: String,
}

/// A playlist entry. `item` was `track` before February 2026; both are read.
#[derive(Debug, Deserialize)]
struct PlaylistEntry {
    #[serde(default, alias = "track")]
    item: Option<serde_json::Value>,
    #[serde(default)]
    is_local: bool,
}

fn spotify(id: String) -> SourceRef {
    SourceRef::Spotify { id }
}

fn present(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// The largest image. Spotify lists them widest first, but says so nowhere, so this doesn't rely
/// on it; an image without dimensions loses to one with.
fn largest(images: Vec<Image>) -> Option<String> {
    images
        .into_iter()
        .enumerate()
        .max_by_key(|(i, image)| {
            let area = u64::from(image.width.unwrap_or(0)) * u64::from(image.height.unwrap_or(0));
            (area, std::cmp::Reverse(*i))
        })
        .map(|(_, image)| image.url)
}

impl ArtistInfo {
    fn describe(self) -> SourceArtist {
        SourceArtist {
            source: present(self.id).map(spotify),
            name: self.name,
        }
    }
}

impl AlbumInfo {
    fn describe(self) -> SourceAlbum {
        let ids = self.external_ids.unwrap_or_default();
        SourceAlbum {
            source: present(self.id).map(spotify),
            title: self.name,
            artists: self.artists.into_iter().map(ArtistInfo::describe).collect(),
            release_date: present(self.release_date),
            barcode: present(ids.upc).or(present(ids.ean)),
            artwork_url: largest(self.images),
        }
    }
}

impl TrackInfo {
    /// The description of this track, or `None` for a local file (no Spotify id).
    fn describe(self) -> Option<SourceTrack> {
        if self.is_local {
            return None;
        }
        let id = present(self.id)?;
        Some(SourceTrack {
            source: spotify(id),
            title: self.name,
            artists: self.artists.into_iter().map(ArtistInfo::describe).collect(),
            album: self.album.map(AlbumInfo::describe),
            disc: self.disc_number,
            position: self.track_number,
            duration_ms: self.duration_ms,
            isrc: present(self.external_ids.and_then(|ids| ids.isrc)),
        })
    }
}

fn tracks(items: Vec<TrackInfo>) -> Vec<SourceTrack> {
    items.into_iter().filter_map(TrackInfo::describe).collect()
}

/// Playlist entries' tracks, skipping episodes, local files and removed tracks (null).
fn entry_tracks(entries: Vec<PlaylistEntry>) -> Vec<SourceTrack> {
    entries
        .into_iter()
        .filter(|entry| !entry.is_local)
        .filter_map(|entry| entry.item)
        .filter(|item| item.get("type").and_then(|t| t.as_str()).unwrap_or("track") == "track")
        .filter_map(|item| serde_json::from_value::<TrackInfo>(item).ok())
        .filter_map(TrackInfo::describe)
        .collect()
}

/// Release dates newest first. Their precision varies (`1981`, `1981-12-02`), but as text they
/// still sort by date; an undated release goes last.
fn newest_first(albums: &mut [SourceAlbum]) {
    albums.sort_by(|a, b| b.release_date.cmp(&a.release_date));
}

/// Spotify's timestamps (`2024-01-15T10:30:00Z`) as milliseconds since the Unix epoch.
fn epoch_ms(stamp: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let millis = if stamp.get(19..20) == Some(".") {
        let digits: String = stamp[20..]
            .chars()
            .take_while(char::is_ascii_digit)
            .chain("000".chars())
            .take(3)
            .collect();
        digits.parse::<i64>().unwrap_or(0)
    } else {
        0
    };
    // Days from the civil calendar (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    // Spotify sends UTC (`Z`); honour an explicit `+hh:mm` offset anyway.
    let offset = stamp
        .rfind(['+', '-'])
        .filter(|&at| at > 18)
        .and_then(|at| {
            let sign = if stamp.as_bytes()[at] == b'-' { -1 } else { 1 };
            let rest = stamp[at + 1..].replace(':', "");
            let hours = rest.get(0..2)?.parse::<i64>().ok()?;
            let minutes = rest
                .get(2..4)
                .and_then(|m| m.parse::<i64>().ok())
                .unwrap_or(0);
            Some(sign * (hours * 60 + minutes) * 60)
        })
        .unwrap_or(0);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset;
    Some(seconds * 1_000 + millis)
}

/// The Spotify id in a binding, or why a Spotify call can't take it.
fn spotify_id(source: &SourceRef) -> Result<&str> {
    match source {
        SourceRef::Spotify { id } => Ok(id),
        other => Err(Error::Unsupported(format!(
            "canon-spotify cannot use a {} binding",
            other.service()
        ))),
    }
}

impl SpotifySession {
    /// Every page of a list, following `next` links, up to `max` items. `page_of` finds the page in
    /// a reply, for the lists that wrap theirs.
    async fn all<R: DeserializeOwned, T>(
        &self,
        first: String,
        max: usize,
        page_of: fn(R) -> Page<T>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next.take() {
            let page = page_of(self.get(&url).await?);
            let got = page.items.len();
            items.extend(page.items);
            tracing::debug!(url, got, total = ?page.total, "spotify page");
            if items.len() >= max {
                items.truncate(max);
                break;
            }
            if got > 0 {
                next = page.next;
            }
        }
        Ok(items)
    }

    /// What Spotify says about one track, ISRC included.
    pub async fn track(&self, id: &str) -> Result<SourceTrack> {
        let track: TrackInfo = self.get(&api_url(&format!("/tracks/{id}"), &[])).await?;
        track
            .describe()
            .ok_or_else(|| Error::NotFound(format!("spotify track {id} is not a catalog track")))
    }

    /// The tracks Spotify has with this ISRC, by the `isrc:` search filter.
    pub async fn tracks_by_isrc(&self, isrc: &str) -> Result<Vec<SourceTrack>> {
        let query = format!("isrc:{isrc}");
        let limit = SEARCH_PAGE.to_string();
        let reply: SearchReply = self
            .get(&api_url(
                "/search",
                &[("q", &query), ("type", "track"), ("limit", &limit)],
            ))
            .await?;
        Ok(tracks(reply.tracks.map(|p| p.items).unwrap_or_default()))
    }

    /// One kind of an artist's releases, newest first.
    async fn releases(&self, id: &str, groups: &str) -> Result<Vec<SourceAlbum>> {
        let limit = ARTIST_ALBUMS_PAGE.to_string();
        let url = api_url(
            &format!("/artists/{id}/albums"),
            &[("include_groups", groups), ("limit", &limit)],
        );
        let albums: Vec<AlbumInfo> = self.all(url, ARTIST_RELEASES, |page| page).await?;
        let mut albums: Vec<SourceAlbum> = albums.into_iter().map(AlbumInfo::describe).collect();
        newest_first(&mut albums);
        Ok(albums)
    }
}

#[async_trait]
impl Catalog for SpotifySession {
    fn service(&self) -> Service {
        Service::Spotify
    }

    /// Up to `limit` (at most [`SEARCH_MAX`]) of each kind, ten per request.
    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults> {
        let want = limit.clamp(1, SEARCH_MAX);
        let mut results = SearchResults::default();
        let mut offset = 0;
        while offset < want {
            let page = SEARCH_PAGE.min(want - offset);
            let (offset_text, limit_text) = (offset.to_string(), page.to_string());
            let reply: SearchReply = self
                .get(&api_url(
                    "/search",
                    &[
                        ("q", query),
                        ("type", "track,album,artist"),
                        ("limit", &limit_text),
                        ("offset", &offset_text),
                    ],
                ))
                .await?;
            let mut more = false;
            if let Some(found) = reply.tracks {
                more |= found.items.len() == page;
                results.tracks.extend(tracks(found.items));
            }
            if let Some(found) = reply.albums {
                more |= found.items.len() == page;
                results
                    .albums
                    .extend(found.items.into_iter().map(AlbumInfo::describe));
            }
            if let Some(found) = reply.artists {
                more |= found.items.len() == page;
                results
                    .artists
                    .extend(found.items.into_iter().map(ArtistInfo::describe));
            }
            if !more {
                break;
            }
            offset += page;
        }
        results.tracks.truncate(want);
        results.albums.truncate(want);
        results.artists.truncate(want);
        Ok(results)
    }

    /// The album and its tracks, each with its ISRC when the album is of ordinary length.
    ///
    /// An album's own track list is of simplified tracks, which carry no `external_ids`, and the
    /// batch `/tracks?ids=` that used to fill them in is gone. The ISRC is the whole reason a
    /// Spotify binding is useful to canon (it is what matches the recording onto a service that
    /// can stream it), so each track of an album up to [`ALBUM_ISRC_LOOKUPS`] long is fetched on
    /// its own: a dozen quick GETs for a normal album. Longer ones (box sets, compilations) skip
    /// it, and their tracks can be described one by one later with
    /// [`SpotifySession::track`].
    async fn album(&self, album: &SourceRef) -> Result<AlbumListing> {
        let id = spotify_id(album)?;
        let mut info: AlbumInfo = self.get(&api_url(&format!("/albums/{id}"), &[])).await?;
        let first = info.tracks.take().unwrap_or(Page {
            items: Vec::new(),
            next: None,
            total: None,
        });
        let mut items = first.items;
        if let Some(next) = first.next.filter(|_| !items.is_empty()) {
            items.extend(self.all(next, 1_000, |page: Page<TrackInfo>| page).await?);
        }
        let album = info.describe();
        let mut listing = tracks(items);
        if listing.len() <= ALBUM_ISRC_LOOKUPS {
            for track in &mut listing {
                let SourceRef::Spotify { id } = &track.source else {
                    continue;
                };
                track.isrc = self.track(id).await?.isrc;
            }
        }
        for track in &mut listing {
            track.album = Some(album.clone());
        }
        listing.sort_by_key(|t| (t.disc.unwrap_or(1), t.position.unwrap_or(0)));
        Ok(AlbumListing {
            album,
            tracks: listing,
        })
    }

    /// The artist and their albums, then singles and EPs. Top tracks are always empty: the
    /// endpoint is gone for development-mode apps.
    async fn artist(&self, artist: &SourceRef) -> Result<ArtistListing> {
        let id = spotify_id(artist)?;
        let info: ArtistInfo = self.get(&api_url(&format!("/artists/{id}"), &[])).await?;
        let mut albums = self.releases(id, "album").await?;
        albums.extend(self.releases(id, "single").await?);
        Ok(ArtistListing {
            artist: info.describe(),
            albums,
            top_tracks: Vec::new(),
        })
    }

    async fn radio(&self, _seed: &Seed) -> Result<Vec<SourceTrack>> {
        Err(Error::Unsupported(format!("spotify radio: {UNAVAILABLE}")))
    }

    async fn similar_artists(&self, _artist: &SourceRef) -> Result<Vec<SourceArtist>> {
        Err(Error::Unsupported(format!(
            "spotify related artists: {UNAVAILABLE}"
        )))
    }

    /// Saved tracks and albums (newest first, with when), and followed artists (Spotify doesn't say
    /// when an artist was followed).
    async fn favorites(&self) -> Result<Favorites> {
        let limit = PAGE.to_string();
        let saved_tracks: Vec<SavedTrack> = self
            .all(
                api_url("/me/tracks", &[("limit", &limit)]),
                FAVORITES,
                |page| page,
            )
            .await?;
        let saved_albums: Vec<SavedAlbum> = self
            .all(
                api_url("/me/albums", &[("limit", &limit)]),
                FAVORITES,
                |page| page,
            )
            .await?;
        let followed: Vec<ArtistInfo> = self
            .all(
                api_url("/me/following", &[("type", "artist"), ("limit", &limit)]),
                FAVORITES,
                |reply: FollowedReply| reply.artists,
            )
            .await?;
        let when = |stamp: &Option<String>| stamp.as_deref().and_then(epoch_ms);
        Ok(Favorites {
            tracks: saved_tracks
                .into_iter()
                .filter_map(|saved| {
                    let added_ms = when(&saved.added_at);
                    Some(Favorite {
                        item: saved.track.describe()?,
                        added_ms,
                    })
                })
                .collect(),
            albums: saved_albums
                .into_iter()
                .map(|saved| Favorite {
                    added_ms: when(&saved.added_at),
                    item: saved.album.describe(),
                })
                .collect(),
            artists: followed
                .into_iter()
                .map(|artist| Favorite {
                    item: artist.describe(),
                    added_ms: None,
                })
                .collect(),
        })
    }

    /// The playlists the user owns or collaborates on, with their tracks. Others' playlists the
    /// user merely follows are left out: Spotify returns no items for them.
    async fn playlists(&self) -> Result<Vec<SourcePlaylist>> {
        let me = self.user_id().await?;
        let limit = PAGE.to_string();
        let listed: Vec<PlaylistInfo> = self
            .all(
                api_url("/me/playlists", &[("limit", &limit)]),
                PLAYLISTS,
                |page| page,
            )
            .await?;
        let mut playlists = Vec::new();
        for playlist in listed
            .into_iter()
            .filter(|p| p.owner.id == me || p.collaborative)
        {
            let url = api_url(
                &format!("/playlists/{}/items", playlist.id),
                &[("limit", &limit), ("additional_types", "track")],
            );
            let entries: Vec<PlaylistEntry> =
                match self.all(url, PLAYLIST_TRACKS, |page| page).await {
                    Ok(entries) => entries,
                    // Marked collaborative but not shared with this user: skip it, not the import.
                    Err(Error::Unsupported(why)) => {
                        tracing::warn!(playlist = %playlist.id, %why, "skipping spotify playlist");
                        continue;
                    }
                    Err(other) => return Err(other),
                };
            playlists.push(SourcePlaylist {
                source: spotify(playlist.id),
                name: playlist.name,
                tracks: entry_tracks(entries),
            });
        }
        Ok(playlists)
    }

    async fn mixes(&self) -> Result<Vec<SourceMix>> {
        Err(Error::Unsupported(format!("spotify mixes: {UNAVAILABLE}")))
    }

    async fn mix(&self, _id: &str) -> Result<Vec<SourceTrack>> {
        Err(Error::Unsupported(format!("spotify mixes: {UNAVAILABLE}")))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::http::HttpResponse;
    use crate::session::testing::{Scripted, signed_in};

    /// A full track object as `GET /tracks/{id}` returns it, trimmed.
    const ARMY_OF_ME: &str = r#"{
        "album": {"album_type": "album", "total_tracks": 11, "id": "0ZIbq3tPlkLlvPxHkAtwf7",
                  "images": [{"url": "https://i.scdn.co/image/300", "height": 300, "width": 300},
                             {"url": "https://i.scdn.co/image/640", "height": 640, "width": 640},
                             {"url": "https://i.scdn.co/image/64", "height": 64, "width": 64}],
                  "name": "Post", "release_date": "1995-06-13", "release_date_precision": "day",
                  "type": "album", "uri": "spotify:album:0ZIbq3tPlkLlvPxHkAtwf7",
                  "artists": [{"id": "7w29UYBi0qsHi5RTcv3lmA", "name": "Björk", "type": "artist"}]},
        "artists": [{"external_urls": {"spotify": "https://open.spotify.com/artist/7w29"},
                     "id": "7w29UYBi0qsHi5RTcv3lmA", "name": "Björk", "type": "artist"}],
        "disc_number": 1, "duration_ms": 234200, "explicit": false,
        "external_ids": {"isrc": "GBAYE9500001"}, "id": "4XJ5mf4ZqQD4qNkCyP1rjd",
        "is_playable": true, "name": "Army of Me", "track_number": 1, "type": "track",
        "uri": "spotify:track:4XJ5mf4ZqQD4qNkCyP1rjd", "is_local": false
    }"#;

    fn sp(id: &str) -> SourceRef {
        SourceRef::Spotify { id: id.into() }
    }

    #[test]
    fn a_track_describes_the_recording_and_its_release() {
        let track = serde_json::from_str::<TrackInfo>(ARMY_OF_ME)
            .unwrap()
            .describe()
            .unwrap();
        assert_eq!(track.source, sp("4XJ5mf4ZqQD4qNkCyP1rjd"));
        assert_eq!(track.title, "Army of Me");
        assert_eq!(track.isrc.as_deref(), Some("GBAYE9500001"));
        assert_eq!(track.duration_ms, Some(234_200));
        assert_eq!((track.disc, track.position), (Some(1), Some(1)));
        assert_eq!(track.artists[0].source, Some(sp("7w29UYBi0qsHi5RTcv3lmA")));
        let album = track.album.unwrap();
        assert_eq!(album.title, "Post");
        assert_eq!(album.release_date.as_deref(), Some("1995-06-13"));
        assert_eq!(
            album.artwork_url.as_deref(),
            Some("https://i.scdn.co/image/640")
        );
    }

    #[test]
    fn a_local_file_is_not_a_track() {
        let json = r#"{"id": null, "name": "demo.mp3", "is_local": true, "type": "track",
                       "artists": [{"id": null, "name": ""}]}"#;
        assert!(
            serde_json::from_str::<TrackInfo>(json)
                .unwrap()
                .describe()
                .is_none()
        );
    }

    #[test]
    fn an_album_carries_its_barcode() {
        let json = r#"{"id": "a1", "name": "Post", "external_ids": {"upc": "075596153329"},
                       "images": [], "artists": [], "release_date": "1995"}"#;
        let album = serde_json::from_str::<AlbumInfo>(json).unwrap().describe();
        assert_eq!(album.barcode.as_deref(), Some("075596153329"));
        assert_eq!(album.artwork_url, None);
        let ean = r#"{"name": "X", "external_ids": {"ean": "5099902987613"}}"#;
        let album = serde_json::from_str::<AlbumInfo>(ean).unwrap().describe();
        assert_eq!(album.barcode.as_deref(), Some("5099902987613"));
    }

    #[test]
    fn spotify_timestamps_read_as_epoch_milliseconds() {
        assert_eq!(epoch_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_ms("2021-03-14T21:05:30Z"), Some(1_615_755_930_000));
        assert_eq!(epoch_ms("2021-03-14T21:05:30.25Z"), Some(1_615_755_930_250));
        assert_eq!(
            epoch_ms("2021-03-14T22:05:30+01:00"),
            Some(1_615_755_930_000)
        );
        assert_eq!(epoch_ms("2000-02-29T00:00:00Z"), Some(951_782_400_000));
        assert_eq!(epoch_ms("garbage"), None);
    }

    #[test]
    fn playlist_entries_keep_tracks_only() {
        // `item` (current) and `track` (before February 2026), an episode, a local file, and a
        // track since removed from the catalog (null).
        let json = format!(
            r#"[{{"added_at": "2024-01-01T00:00:00Z", "is_local": false, "item": {ARMY_OF_ME}}},
                {{"added_at": "2024-01-01T00:00:00Z", "is_local": false, "track": {ARMY_OF_ME}}},
                {{"is_local": false, "item": {{"id": "ep1", "name": "A podcast", "type": "episode"}}}},
                {{"is_local": true, "item": {{"id": null, "name": "demo", "type": "track", "is_local": true}}}},
                {{"is_local": false, "item": null}}]"#
        );
        let entries: Vec<PlaylistEntry> = serde_json::from_str(&json).unwrap();
        let tracks = entry_tracks(entries);
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.title == "Army of Me"));
    }

    fn search_url(offset: usize, limit: usize) -> String {
        api_url(
            "/search",
            &[
                ("q", "bjork"),
                ("type", "track,album,artist"),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
            ],
        )
    }

    fn search_page(tracks: usize, artists: usize) -> String {
        let track = |i: usize| format!(r#"{{"id": "t{i}", "name": "Track {i}"}}"#);
        let artist = |i: usize| format!(r#"{{"id": "a{i}", "name": "Artist {i}"}}"#);
        format!(
            r#"{{"tracks": {{"items": [{}], "total": 100, "next": null}},
                "albums": {{"items": [], "total": 0}},
                "artists": {{"items": [{}], "total": 3}}}}"#,
            (0..tracks).map(track).collect::<Vec<_>>().join(","),
            (0..artists).map(artist).collect::<Vec<_>>().join(","),
        )
    }

    fn count(http: &Scripted, needle: &str) -> usize {
        http.requested()
            .iter()
            .filter(|r| r.contains(needle))
            .count()
    }

    #[tokio::test]
    async fn search_pages_ten_at_a_time_up_to_the_limit() {
        let http = Arc::new(Scripted::default());
        http.json(&search_url(0, 10), &search_page(10, 3))
            .json(&search_url(10, 10), &search_page(10, 0))
            .json(&search_url(20, 5), &search_page(5, 0));
        let session = signed_in(http.clone()).await;
        let found = session.search("bjork", 25).await.unwrap();
        assert_eq!(found.tracks.len(), 25);
        assert_eq!(found.artists.len(), 3);
        assert!(found.albums.is_empty());
        assert_eq!(count(&http, "/search"), 3);
    }

    #[tokio::test]
    async fn search_stops_when_every_kind_runs_out() {
        let http = Arc::new(Scripted::default());
        http.json(&search_url(0, 10), &search_page(4, 2));
        let session = signed_in(http.clone()).await;
        let found = session.search("bjork", 50).await.unwrap();
        assert_eq!((found.tracks.len(), found.artists.len()), (4, 2));
        assert_eq!(count(&http, "/search"), 1);
    }

    #[tokio::test]
    async fn search_asks_for_no_more_than_it_needs() {
        let http = Arc::new(Scripted::default());
        http.json(&search_url(0, 3), &search_page(3, 0));
        let session = signed_in(http).await;
        assert_eq!(session.search("bjork", 3).await.unwrap().tracks.len(), 3);
    }

    #[tokio::test]
    async fn an_album_pages_its_tracks_and_fills_in_isrcs() {
        let http = Arc::new(Scripted::default());
        let next = api_url("/albums/a1/tracks", &[("offset", "2"), ("limit", "2")]);
        http.json(
            &api_url("/albums/a1", &[]),
            &format!(
                r#"{{"id": "a1", "name": "Post", "release_date": "1995-06-13",
                    "external_ids": {{"upc": "075596153329"}},
                    "images": [{{"url": "https://i.scdn.co/image/640", "width": 640, "height": 640}}],
                    "artists": [{{"id": "7w29UYBi0qsHi5RTcv3lmA", "name": "Björk"}}],
                    "tracks": {{"href": "x", "limit": 2, "offset": 0, "total": 3, "next": "{next}",
                        "items": [
                            {{"id": "t1", "name": "Army of Me", "disc_number": 1, "track_number": 1, "duration_ms": 234200}},
                            {{"id": "t2", "name": "Hyperballad", "disc_number": 1, "track_number": 2}}]}}}}"#
            ),
        )
        .json(
            &next,
            r#"{"items": [{"id": "t3", "name": "The Modern Things", "disc_number": 1, "track_number": 3}],
                "total": 3, "next": null}"#,
        );
        for (id, isrc) in [("t1", "GBAYE9500001"), ("t2", "GBAYE9500002")] {
            http.json(
                &api_url(&format!("/tracks/{id}"), &[]),
                &format!(r#"{{"id": "{id}", "name": "x", "external_ids": {{"isrc": "{isrc}"}}}}"#),
            );
        }
        http.json(
            &api_url("/tracks/t3", &[]),
            r#"{"id": "t3", "name": "The Modern Things"}"#,
        );
        let session = signed_in(http).await;
        let listing = session.album(&sp("a1")).await.unwrap();
        assert_eq!(listing.album.barcode.as_deref(), Some("075596153329"));
        let isrcs: Vec<_> = listing.tracks.iter().map(|t| t.isrc.as_deref()).collect();
        assert_eq!(isrcs, [Some("GBAYE9500001"), Some("GBAYE9500002"), None]);
        let positions: Vec<_> = listing.tracks.iter().map(|t| t.position).collect();
        assert_eq!(positions, [Some(1), Some(2), Some(3)]);
        assert_eq!(listing.tracks[0].title, "Army of Me");
        assert_eq!(
            listing.tracks[0].album.as_ref().unwrap().barcode.as_deref(),
            Some("075596153329")
        );
    }

    #[tokio::test]
    async fn an_artist_lists_albums_then_singles_newest_first_with_no_top_tracks() {
        let http = Arc::new(Scripted::default());
        let albums = api_url(
            "/artists/ar1/albums",
            &[("include_groups", "album"), ("limit", "10")],
        );
        let more = api_url(
            "/artists/ar1/albums",
            &[
                ("include_groups", "album"),
                ("offset", "10"),
                ("limit", "10"),
            ],
        );
        http.json(
            &api_url("/artists/ar1", &[]),
            r#"{"id": "ar1", "name": "Björk", "genres": [], "type": "artist"}"#,
        )
        .json(
            &albums,
            &format!(
                r#"{{"items": [{{"id": "d", "name": "Debut", "release_date": "1993-07-05", "album_type": "album"}}],
                    "next": "{more}", "total": 2}}"#
            ),
        )
        .json(
            &more,
            r#"{"items": [{"id": "h", "name": "Homogenic", "release_date": "1997", "album_type": "album"}],
                "next": null, "total": 2}"#,
        )
        .json(
            &api_url(
                "/artists/ar1/albums",
                &[("include_groups", "single"), ("limit", "10")],
            ),
            r#"{"items": [{"id": "s", "name": "Army of Me", "release_date": "1995-04-24"}], "next": null}"#,
        );
        let session = signed_in(http).await;
        let listing = session.artist(&sp("ar1")).await.unwrap();
        assert_eq!(listing.artist.name, "Björk");
        let titles: Vec<_> = listing.albums.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Homogenic", "Debut", "Army of Me"]);
        assert!(listing.top_tracks.is_empty());
    }

    #[tokio::test]
    async fn favorites_read_saved_tracks_albums_and_followed_artists() {
        let http = Arc::new(Scripted::default());
        let page2 = api_url("/me/tracks", &[("offset", "1"), ("limit", "50")]);
        http.json(
            &api_url("/me/tracks", &[("limit", "50")]),
            &format!(
                r#"{{"href": "x", "limit": 50, "offset": 0, "total": 2, "next": "{page2}",
                    "items": [{{"added_at": "2021-03-14T21:05:30Z", "track": {ARMY_OF_ME}}}]}}"#
            ),
        )
        .json(
            &page2,
            r#"{"total": 2, "next": null, "items": [{"added_at": "1970-01-01T00:00:01Z",
                "track": {"id": "t2", "name": "Hyperballad", "external_ids": {"isrc": "GBAYE9500002"}}}]}"#,
        )
        .json(
            &api_url("/me/albums", &[("limit", "50")]),
            r#"{"total": 1, "next": null, "items": [{"added_at": "2020-01-01T00:00:00Z",
                "album": {"id": "a1", "name": "Post", "external_ids": {"upc": "075596153329"},
                          "tracks": {"items": [], "next": null, "total": 11}}}]}"#,
        );
        let followed_next = api_url(
            "/me/following",
            &[("type", "artist"), ("after", "ar1"), ("limit", "50")],
        );
        http.json(
            &api_url("/me/following", &[("type", "artist"), ("limit", "50")]),
            &format!(
                r#"{{"artists": {{"href": "x", "limit": 50, "next": "{followed_next}",
                    "cursors": {{"after": "ar1"}}, "total": 2,
                    "items": [{{"id": "ar1", "name": "Björk", "type": "artist"}}]}}}}"#
            ),
        )
        .json(
            &followed_next,
            r#"{"artists": {"next": null, "cursors": {"after": null}, "total": 2,
                "items": [{"id": "ar2", "name": "The Dresden Dolls"}]}}"#,
        );
        let session = signed_in(http).await;
        let favorites = session.favorites().await.unwrap();
        assert_eq!(favorites.tracks.len(), 2);
        assert_eq!(favorites.tracks[0].added_ms, Some(1_615_755_930_000));
        assert_eq!(favorites.tracks[1].added_ms, Some(1_000));
        assert_eq!(
            favorites.tracks[1].item.isrc.as_deref(),
            Some("GBAYE9500002")
        );
        assert_eq!(
            favorites.albums[0].item.barcode.as_deref(),
            Some("075596153329")
        );
        let artists: Vec<_> = favorites
            .artists
            .iter()
            .map(|f| (f.item.name.as_str(), f.added_ms))
            .collect();
        assert_eq!(artists, [("Björk", None), ("The Dresden Dolls", None)]);
    }

    #[tokio::test]
    async fn playlists_are_the_users_own_and_collaborative_ones() {
        let http = Arc::new(Scripted::default());
        http.json(&api_url("/me", &[]), r#"{"id": "joel", "display_name": "Joel"}"#)
            .json(
                &api_url("/me/playlists", &[("limit", "50")]),
                r#"{"total": 4, "next": null, "items": [
                    {"id": "mine", "name": "Mine", "collaborative": false, "public": true,
                     "owner": {"id": "joel", "display_name": "Joel"}, "items": {"href": "x", "total": 1}},
                    {"id": "shared", "name": "Shared", "collaborative": true,
                     "owner": {"id": "friend"}, "items": {"href": "x", "total": 1}},
                    {"id": "gone", "name": "Unshared", "collaborative": true,
                     "owner": {"id": "stranger"}, "items": {"href": "x", "total": 1}},
                    {"id": "37i9dQZF1DXcBWIGoYBM5M", "name": "Today's Top Hits", "collaborative": false,
                     "owner": {"id": "spotify"}, "items": {"href": "x", "total": 50}}]}"#,
            );
        let items = |id: &str| {
            api_url(
                &format!("/playlists/{id}/items"),
                &[("limit", "50"), ("additional_types", "track")],
            )
        };
        let entries = format!(
            r#"{{"total": 1, "next": null, "items": [{{"is_local": false, "item": {ARMY_OF_ME}}}]}}"#
        );
        http.json(&items("mine"), &entries)
            .json(&items("shared"), &entries)
            .on(
                &items("gone"),
                HttpResponse::new(403, r#"{"error": {"status": 403, "message": "Forbidden"}}"#),
            );
        let session = signed_in(http.clone()).await;
        let playlists = session.playlists().await.unwrap();
        let names: Vec<_> = playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mine", "Shared"]);
        assert_eq!(playlists[0].source, sp("mine"));
        assert_eq!(playlists[0].tracks[0].title, "Army of Me");
        assert_eq!(
            count(&http, "37i9dQZF1DXcBWIGoYBM5M"),
            0,
            "never asks for a playlist it can't read"
        );
    }

    #[tokio::test]
    async fn tracks_by_isrc_use_the_search_filter() {
        let http = Arc::new(Scripted::default());
        http.json(
            &api_url(
                "/search",
                &[
                    ("q", "isrc:GBAYE9500001"),
                    ("type", "track"),
                    ("limit", "10"),
                ],
            ),
            &format!(r#"{{"tracks": {{"items": [{ARMY_OF_ME}], "total": 1, "next": null}}}}"#),
        );
        let session = signed_in(http).await;
        let found = session.tracks_by_isrc("GBAYE9500001").await.unwrap();
        assert_eq!(found[0].source, sp("4XJ5mf4ZqQD4qNkCyP1rjd"));
    }

    #[tokio::test]
    async fn what_dev_mode_apps_lack_is_unsupported() {
        let http = Arc::new(Scripted::default());
        let session = signed_in(http.clone()).await;
        let unavailable =
            |e: Error| matches!(e, Error::Unsupported(ref m) if m.contains("development-mode"));
        let seed = Seed::Artist(sp("ar1"));
        assert!(unavailable(session.radio(&seed).await.unwrap_err()));
        assert!(unavailable(
            session.similar_artists(&sp("ar1")).await.unwrap_err()
        ));
        assert!(unavailable(session.mixes().await.unwrap_err()));
        assert!(unavailable(session.mix("m").await.unwrap_err()));
        assert!(http.requested().is_empty());
        let tidal = SourceRef::Tidal { id: "1".into() };
        assert!(matches!(
            session.album(&tidal).await,
            Err(Error::Unsupported(_))
        ));
    }
}
