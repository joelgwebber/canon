//! Browsing: the library, playlists, search results, albums and artists as *pages* of rows.
//!
//! A page is one list on screen and where it came from, so it can ask for itself (and for more
//! of itself, when the library pages it). Each browsing tab keeps a stack of them: opening an
//! album pushes its page, going back pops it. Rows are the page's items with section headings
//! between them (search results: tracks, albums, artists); the cursor only ever rests on items.

use std::cell::Cell;

use canon_api::ClientMessage;
use canon_api::ReplyData;
use canon_core::{EntityId, Service};
use canon_library::{
    AlbumView, ArtistView, EntityKind, ItemRef, MixView, PlaylistView, ServicePlaylistView,
    TrackView,
};

/// How many saved items a library page asks for at a time.
pub const LIBRARY_PAGE: usize = 200;
/// How many results of each kind a search asks for.
pub const SEARCH_RESULTS: usize = 25;
/// How close to the end of what's loaded the cursor gets before the next page is asked for.
const LOAD_AHEAD: usize = 40;

/// The client's views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Queue,
    Library,
    Playlists,
    Search,
    Outputs,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Queue,
        Tab::Library,
        Tab::Playlists,
        Tab::Search,
        Tab::Outputs,
        Tab::Settings,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Tab::Queue => "Queue",
            Tab::Library => "Library",
            Tab::Playlists => "Playlists",
            Tab::Search => "Search",
            Tab::Outputs => "Outputs",
            Tab::Settings => "Settings",
        }
    }

    /// Which page stack this tab browses, if it browses.
    pub(crate) fn stack(self) -> Option<usize> {
        match self {
            Tab::Queue | Tab::Outputs | Tab::Settings => None,
            Tab::Library => Some(0),
            Tab::Playlists => Some(1),
            Tab::Search => Some(2),
        }
    }
}

/// A playlist or a mix that belongs to a service: a list to browse, play, copy or merge, but
/// never a library entity — canon doesn't mint an id for one, so it has no canon id to name it
/// by and nothing to save (canon-5b2b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// What names it to the daemon: `ItemRef::Service` with kind `playlist`, or `ItemRef::Mix`.
    pub item: ItemRef,
    pub name: String,
    /// How many tracks, when the listing says without the tracklist being fetched.
    pub track_count: Option<usize>,
    /// What a mix says it is; empty for a playlist.
    pub note: String,
}

impl Remote {
    fn playlist(view: ServicePlaylistView) -> Self {
        Self {
            item: ItemRef::Service {
                service: view.service,
                id: view.id,
                kind: EntityKind::Playlist,
            },
            name: view.name,
            track_count: Some(view.track_count),
            note: String::new(),
        }
    }

    fn mix(view: MixView) -> Self {
        Self {
            item: ItemRef::Mix {
                service: view.service,
                mix: view.mix,
            },
            name: view.name,
            track_count: None,
            note: view.description,
        }
    }
}

/// Something a row shows, with the id to act on it by.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Track(TrackView),
    Album(AlbumView),
    Artist(ArtistView),
    Playlist(PlaylistView),
    /// A service's own playlist or mix, which has no canon id.
    Remote(Remote),
}

impl Item {
    /// Its canon id; `None` for a service's playlist or mix, which never becomes an entity.
    #[must_use]
    pub fn id(&self) -> Option<EntityId> {
        match self {
            Item::Track(t) => Some(t.id),
            Item::Album(a) => Some(a.id),
            Item::Artist(a) => Some(a.id),
            Item::Playlist(p) => Some(p.id),
            Item::Remote(_) => None,
        }
    }

    #[must_use]
    pub fn item_ref(&self) -> ItemRef {
        match self {
            Item::Remote(remote) => remote.item.clone(),
            _ => ItemRef::Entity {
                entity: self.id().expect("every other item has a canon id"),
            },
        }
    }

    /// Its name, for saying what was done with it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Item::Track(t) => &t.title,
            Item::Album(a) => &a.title,
            Item::Artist(a) => &a.name,
            Item::Playlist(p) => &p.name,
            Item::Remote(r) => &r.name,
        }
    }

    /// Whether it is in the library; `None` for what can't be saved (a playlist is canon's own,
    /// and a service's playlist or mix is never ingested at all).
    #[must_use]
    pub fn saved(&self) -> Option<bool> {
        match self {
            Item::Track(t) => Some(t.saved),
            Item::Album(a) => Some(a.saved),
            Item::Artist(a) => Some(a.saved),
            Item::Playlist(_) | Item::Remote(_) => None,
        }
    }

    pub(crate) fn set_saved(&mut self, saved: bool) {
        match self {
            Item::Track(t) => t.saved = saved,
            Item::Album(a) => a.saved = saved,
            Item::Artist(a) => a.saved = saved,
            Item::Playlist(_) | Item::Remote(_) => {}
        }
    }

    /// This item as a list of tracks to copy or merge somewhere, with its name; `None` for what
    /// isn't a list (a track, an artist).
    #[must_use]
    pub fn list(&self) -> Option<(ItemRef, String)> {
        match self {
            Item::Album(_) | Item::Playlist(_) | Item::Remote(_) => {
                Some((self.item_ref(), self.name().to_owned()))
            }
            Item::Track(_) | Item::Artist(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Heading(String),
    Item(Item),
}

/// Where a page's rows come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// The saved library, of one kind, paged.
    Library(EntityKind),
    /// Every playlist.
    Playlists,
    Playlist(EntityId),
    Album(EntityId),
    Artist(EntityId),
    Search(String),
    /// The playlists the user keeps on a service, read-only.
    ServicePlaylists(Service),
    /// The mixes a service made for the user.
    ServiceMixes(Service),
    /// One service playlist's tracks, by the service's own id for it.
    ServicePlaylist(Service, String),
    /// One mix's tracks.
    Mix(Service, String),
}

impl Source {
    /// The page this item opens.
    #[must_use]
    pub fn of(item: &Item) -> Option<Source> {
        match item {
            Item::Track(_) => None,
            Item::Album(a) => Some(Source::Album(a.id)),
            Item::Artist(a) => Some(Source::Artist(a.id)),
            Item::Playlist(p) => Some(Source::Playlist(p.id)),
            Item::Remote(r) => match &r.item {
                ItemRef::Service { service, id, .. } => {
                    Some(Source::ServicePlaylist(*service, id.clone()))
                }
                ItemRef::Mix { service, mix } => Some(Source::Mix(*service, mix.clone())),
                ItemRef::Entity { .. } => None,
            },
        }
    }

    /// This page's own contents as a list to copy or merge somewhere; `None` for what isn't one
    /// (a library listing, a search, an artist).
    #[must_use]
    pub fn list(&self) -> Option<ItemRef> {
        match self {
            Source::Playlist(id) | Source::Album(id) => Some(ItemRef::Entity { entity: *id }),
            Source::ServicePlaylist(service, id) => Some(ItemRef::Service {
                service: *service,
                id: id.clone(),
                kind: EntityKind::Playlist,
            }),
            Source::Mix(service, mix) => Some(ItemRef::Mix {
                service: *service,
                mix: mix.clone(),
            }),
            _ => None,
        }
    }
}

/// What the Playlists tab's root page lists: canon's own, or one service's.
///
/// Spotify is deliberately absent from the mixes: `Catalog::mixes` is `Unsupported` there, so
/// offering the combination would only ever be an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// Canon's own playlists.
    Mine,
    ServicePlaylists(Service),
    ServiceMixes(Service),
}

impl Shelf {
    /// The shelves `[` and `]` cycle through, in order.
    pub const ALL: [Shelf; 4] = [
        Shelf::Mine,
        Shelf::ServicePlaylists(Service::Tidal),
        Shelf::ServiceMixes(Service::Tidal),
        Shelf::ServicePlaylists(Service::Spotify),
    ];

    /// Its heading, which is also the page's title.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Shelf::Mine => "Playlists".to_owned(),
            Shelf::ServicePlaylists(service) => format!("{} playlists", service_name(service)),
            Shelf::ServiceMixes(service) => format!("{} mixes", service_name(service)),
        }
    }

    /// Its one-word label in the switcher hint.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Shelf::Mine => "mine".to_owned(),
            Shelf::ServicePlaylists(service) => service.to_string(),
            Shelf::ServiceMixes(service) => format!("{service} mixes"),
        }
    }

    #[must_use]
    pub fn source(self) -> Source {
        match self {
            Shelf::Mine => Source::Playlists,
            Shelf::ServicePlaylists(service) => Source::ServicePlaylists(service),
            Shelf::ServiceMixes(service) => Source::ServiceMixes(service),
        }
    }
}

pub struct Page {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) source: Source,
    pub(crate) rows: Vec<Row>,
    /// Index into `rows`; always an item's row when there are any.
    pub(crate) cursor: usize,
    pub(crate) scroll: Cell<usize>,
    /// For a paged source: how many there are in all.
    pub(crate) total: Option<usize>,
    pub(crate) loading: bool,
}

impl Page {
    pub(crate) fn new(id: u64, title: String, source: Source) -> Self {
        Self {
            id,
            title,
            source,
            rows: Vec::new(),
            cursor: 0,
            scroll: Cell::new(0),
            total: None,
            loading: false,
        }
    }

    /// How many items have arrived.
    pub(crate) fn loaded(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, Row::Item(_)))
            .count()
    }

    /// The request for the page's next rows: its first, or (for the library) the next page.
    pub(crate) fn request(&self) -> ClientMessage {
        let entity = |id| ItemRef::Entity { entity: id };
        match &self.source {
            Source::Library(kind) => ClientMessage::Library {
                kind: *kind,
                query: None,
                limit: Some(LIBRARY_PAGE),
                offset: self.loaded(),
            },
            Source::Playlists => ClientMessage::Library {
                kind: EntityKind::Playlist,
                query: None,
                limit: Some(LIBRARY_PAGE),
                offset: self.loaded(),
            },
            Source::Playlist(id) => ClientMessage::Playlist { playlist: *id },
            Source::Album(id) => ClientMessage::Album { item: entity(*id) },
            Source::Artist(id) => ClientMessage::Artist { item: entity(*id) },
            Source::Search(query) => ClientMessage::Search {
                query: query.clone(),
                service: None,
                limit: Some(SEARCH_RESULTS),
            },
            Source::ServicePlaylists(service) => ClientMessage::ServicePlaylists {
                service: Some(*service),
            },
            Source::ServiceMixes(service) => ClientMessage::Mixes {
                service: Some(*service),
            },
            Source::ServicePlaylist(service, id) => ClientMessage::ServicePlaylist {
                item: ItemRef::Service {
                    service: *service,
                    id: id.clone(),
                    kind: EntityKind::Playlist,
                },
            },
            Source::Mix(service, mix) => ClientMessage::Mix {
                service: Some(*service),
                mix: mix.clone(),
            },
        }
    }

    /// Whether the cursor is near the end of a paged list that has more.
    pub(crate) fn wants_more(&self) -> bool {
        !self.loading
            && self.total.is_some_and(|total| self.loaded() < total)
            && self.cursor + LOAD_AHEAD >= self.rows.len()
    }

    /// Take in a reply for this page.
    pub(crate) fn fold(&mut self, data: ReplyData) {
        self.loading = false;
        let first = self.rows.is_empty();
        match data {
            ReplyData::Library(page) => {
                self.total = Some(page.total);
                let items = page
                    .tracks
                    .into_iter()
                    .map(Item::Track)
                    .chain(page.albums.into_iter().map(Item::Album))
                    .chain(page.artists.into_iter().map(Item::Artist))
                    .chain(page.playlists.into_iter().map(Item::Playlist));
                self.rows.extend(items.map(Row::Item));
            }
            ReplyData::Playlist(detail) => {
                self.title = detail.playlist.name.clone();
                self.rows = tracks(detail.tracks);
            }
            ReplyData::Album(detail) => {
                self.title = format!("{} — {}", detail.album.title, detail.album.credit);
                let discs = detail.tracks.iter().map(|t| t.disc).max().unwrap_or(1);
                let mut rows = Vec::new();
                let mut disc = 0;
                for listed in detail.tracks {
                    if discs > 1 && listed.disc != disc {
                        disc = listed.disc;
                        rows.push(Row::Heading(format!("Disc {disc}")));
                    }
                    rows.push(Row::Item(Item::Track(listed.track)));
                }
                self.rows = rows;
            }
            ReplyData::Artist(detail) => {
                self.title = detail.artist.name.clone();
                let mut rows = Vec::new();
                section(&mut rows, "Top tracks", detail.top_tracks, Item::Track);
                section(&mut rows, "Releases", detail.albums, Item::Album);
                self.rows = rows;
            }
            ReplyData::Search(found) => {
                let mut rows = Vec::new();
                section(&mut rows, "Tracks", found.tracks, Item::Track);
                section(&mut rows, "Albums", found.albums, Item::Album);
                section(&mut rows, "Artists", found.artists, Item::Artist);
                section(&mut rows, "Playlists", found.playlists, |view| {
                    Item::Remote(Remote::playlist(view))
                });
                self.rows = rows;
            }
            ReplyData::ServicePlaylists { playlists } => {
                self.rows = playlists
                    .into_iter()
                    .map(|view| Row::Item(Item::Remote(Remote::playlist(view))))
                    .collect();
            }
            ReplyData::Mixes { mixes } => {
                self.rows = mixes
                    .into_iter()
                    .map(|view| Row::Item(Item::Remote(Remote::mix(view))))
                    .collect();
            }
            // Opening a service playlist or a mix: its tracks, in the service's order.
            ReplyData::Tracks { tracks: listed } => self.rows = tracks(listed),
            _ => {}
        }
        if first {
            self.cursor = self.first_item().unwrap_or(0);
        }
    }

    fn first_item(&self) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, Row::Item(_)))
    }

    pub(crate) fn selected(&self) -> Option<&Item> {
        match self.rows.get(self.cursor) {
            Some(Row::Item(item)) => Some(item),
            _ => None,
        }
    }

    /// Move the cursor `by` items, stepping over headings and stopping at the ends.
    pub(crate) fn move_by(&mut self, by: isize) {
        let items: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| matches!(row, Row::Item(_)).then_some(i))
            .collect();
        let Some(at) = items.iter().position(|i| *i >= self.cursor) else {
            return;
        };
        let to = at.saturating_add_signed(by).min(items.len() - 1);
        self.cursor = items[to];
    }

    pub(crate) fn go_top(&mut self) {
        self.cursor = self.first_item().unwrap_or(0);
    }

    pub(crate) fn go_bottom(&mut self) {
        if let Some(last) = self
            .rows
            .iter()
            .rposition(|row| matches!(row, Row::Item(_)))
        {
            self.cursor = last;
        }
    }

    /// The tracks of the cursor's section, as queue items, and where the cursor is among them:
    /// what "play from here" queues.
    pub(crate) fn section_tracks(&self) -> (Vec<ItemRef>, usize) {
        let start = self.rows[..self.cursor]
            .iter()
            .rposition(|row| matches!(row, Row::Heading(_)))
            .map_or(0, |heading| heading + 1);
        let end = self.rows[self.cursor..]
            .iter()
            .position(|row| matches!(row, Row::Heading(_)))
            .map_or(self.rows.len(), |heading| self.cursor + heading);
        let mut tracks = Vec::new();
        let mut at = 0;
        for (index, row) in self.rows.iter().enumerate().take(end).skip(start) {
            if let Row::Item(item @ Item::Track(_)) = row {
                if index == self.cursor {
                    at = tracks.len();
                }
                tracks.push(item.item_ref());
            }
        }
        (tracks, at)
    }
}

fn tracks(tracks: Vec<TrackView>) -> Vec<Row> {
    tracks
        .into_iter()
        .map(|track| Row::Item(Item::Track(track)))
        .collect()
}

/// A service's name as a heading says it.
fn service_name(service: Service) -> &'static str {
    match service {
        Service::Local => "Local",
        Service::Tidal => "Tidal",
        Service::Spotify => "Spotify",
    }
}

fn section<T>(rows: &mut Vec<Row>, heading: &str, items: Vec<T>, wrap: fn(T) -> Item) {
    if items.is_empty() {
        return;
    }
    rows.push(Row::Heading(heading.to_owned()));
    rows.extend(items.into_iter().map(|item| Row::Item(wrap(item))));
}
