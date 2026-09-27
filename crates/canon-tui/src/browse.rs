//! Browsing: the library, playlists, search results, albums and artists as *pages* of rows.
//!
//! A page is one list on screen and where it came from, so it can ask for itself (and for more
//! of itself, when the library pages it). Each browsing tab keeps a stack of them: opening an
//! album pushes its page, going back pops it. Rows are the page's items with section headings
//! between them (search results: tracks, albums, artists); the cursor only ever rests on items.

use std::cell::Cell;

use canon_api::ClientMessage;
use canon_api::ReplyData;
use canon_core::EntityId;
use canon_library::{AlbumView, ArtistView, EntityKind, ItemRef, PlaylistView, TrackView};

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

/// Something a row shows, with the id to act on it by.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Track(TrackView),
    Album(AlbumView),
    Artist(ArtistView),
    Playlist(PlaylistView),
}

impl Item {
    #[must_use]
    pub fn id(&self) -> EntityId {
        match self {
            Item::Track(t) => t.id,
            Item::Album(a) => a.id,
            Item::Artist(a) => a.id,
            Item::Playlist(p) => p.id,
        }
    }

    #[must_use]
    pub fn item_ref(&self) -> ItemRef {
        ItemRef::Entity { entity: self.id() }
    }

    /// Its name, for saying what was done with it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Item::Track(t) => &t.title,
            Item::Album(a) => &a.title,
            Item::Artist(a) => &a.name,
            Item::Playlist(p) => &p.name,
        }
    }

    /// Whether it is in the library; `None` for what can't be saved (a playlist is canon's own).
    #[must_use]
    pub fn saved(&self) -> Option<bool> {
        match self {
            Item::Track(t) => Some(t.saved),
            Item::Album(a) => Some(a.saved),
            Item::Artist(a) => Some(a.saved),
            Item::Playlist(_) => None,
        }
    }

    pub(crate) fn set_saved(&mut self, saved: bool) {
        match self {
            Item::Track(t) => t.saved = saved,
            Item::Album(a) => a.saved = saved,
            Item::Artist(a) => a.saved = saved,
            Item::Playlist(_) => {}
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
                self.rows = rows;
            }
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

fn section<T>(rows: &mut Vec<Row>, heading: &str, items: Vec<T>, wrap: fn(T) -> Item) {
    if items.is_empty() {
        return;
    }
    rows.push(Row::Heading(heading.to_owned()));
    rows.extend(items.into_iter().map(|item| Row::Item(wrap(item))));
}
