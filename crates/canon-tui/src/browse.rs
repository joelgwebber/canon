//! Browsing: the library, playlists, search results, albums and artists as *pages* of rows.
//!
//! A page is one list on screen and where it came from, so it can ask for itself (and for more
//! of itself, when the library pages it). Each browsing tab keeps a stack of them: opening an
//! album pushes its page, going back pops it. Rows are the page's items with section headings
//! between them (search results: tracks, albums, artists); the cursor only ever rests on items.

use std::cell::Cell;

use canon_core::{EntityId, Service};
use canon_protocol::{
    AlbumView, ArtistView, ClientMessage, EntityKind, ItemRef, MixView, PlaylistView, ReplyData,
    SearchView, ServicePlaylistView, TrackView,
};

/// How many saved items a library page asks for at a time.
pub const LIBRARY_PAGE: usize = 200;
/// How many results of each kind a search asks for.
pub const SEARCH_RESULTS: usize = 25;
/// How close to the end of what's loaded the cursor gets before the next page is asked for.
const LOAD_AHEAD: usize = 40;

/// The client's places (doks/design/tui-places.md): what you are doing, not where the data
/// lives. Now is the queue; Library is what is yours, playlists included; Sources is everything
/// from outside, one grouped list of each service's search, playlists and mixes. The outputs are
/// not a place: choosing where to play is a picker over whatever is showing (canon-5cb7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Now,
    Library,
    Sources,
    /// Services, sign-ins and preferences: a view like the others, but reached by its own key
    /// rather than in `Tab`'s cycle, since it is visited rarely.
    Settings,
}

impl Tab {
    /// The places `Tab` cycles through and `1`-`3` name, in order.
    pub const ALL: [Tab; 3] = [Tab::Now, Tab::Library, Tab::Sources];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Tab::Now => "Now",
            Tab::Library => "Library",
            Tab::Sources => "Sources",
            Tab::Settings => "Settings",
        }
    }

    /// Which list this place browses, if it browses.
    pub(crate) fn stack(self) -> Option<usize> {
        match self {
            Tab::Now | Tab::Settings => None,
            Tab::Library => Some(0),
            Tab::Sources => Some(1),
        }
    }
}

/// The kinds Library lists, in the order `[` and `]` step through them.
pub const LIBRARY_KINDS: [EntityKind; 4] = [
    EntityKind::Track,
    EntityKind::Album,
    EntityKind::Artist,
    EntityKind::Playlist,
];

/// One row of Sources: a listing from outside the library, opened in the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A search of one service's catalog, or (`None`) of every connected one.
    Search(Option<Service>),
    ServicePlaylists(Service),
    ServiceMixes(Service),
}

impl Entry {
    /// Sources' rows, grouped by service. Spotify has no mixes: `Catalog::mixes` is
    /// `Unsupported` there, so offering it would only ever be an error.
    pub const ALL: [Entry; 6] = [
        Entry::Search(Some(Service::Tidal)),
        Entry::ServicePlaylists(Service::Tidal),
        Entry::ServiceMixes(Service::Tidal),
        Entry::Search(Some(Service::Spotify)),
        Entry::ServicePlaylists(Service::Spotify),
        Entry::Search(None),
    ];

    /// The group it sits under: its service, or Everywhere.
    #[must_use]
    pub fn group(self) -> &'static str {
        match self {
            Entry::Search(Some(service))
            | Entry::ServicePlaylists(service)
            | Entry::ServiceMixes(service) => service_name(service),
            Entry::Search(None) => "Everywhere",
        }
    }

    /// What it lists, within its group.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Entry::Search(_) => "Search",
            Entry::ServicePlaylists(_) => "Playlists",
            Entry::ServiceMixes(_) => "Mixes",
        }
    }

    /// Its listing's title in the pane.
    #[must_use]
    pub fn title(self) -> String {
        format!("{} {}", self.group(), self.label().to_lowercase())
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
    /// A row of Sources: a listing to open.
    Entry(Entry),
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
            Item::Remote(_) | Item::Entry(_) => None,
        }
    }

    /// What names it to the daemon; `None` for a Sources entry, which is the client's own.
    #[must_use]
    pub fn item_ref(&self) -> Option<ItemRef> {
        match self {
            Item::Remote(remote) => Some(remote.item.clone()),
            Item::Entry(_) => None,
            _ => self.id().map(|entity| ItemRef::Entity { entity }),
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
            Item::Entry(e) => e.label(),
        }
    }

    /// Whether `needle` (lower-case) is in its name or who it's by, ignoring case: what `/`
    /// finds in a list.
    #[must_use]
    pub fn matches(&self, needle: &str) -> bool {
        let by = match self {
            Item::Track(t) => t
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            Item::Album(a) => vec![a.credit.as_str()],
            Item::Entry(e) => vec![e.group()],
            Item::Artist(_) | Item::Playlist(_) | Item::Remote(_) => Vec::new(),
        };
        std::iter::once(self.name())
            .chain(by)
            .any(|text| text.to_lowercase().contains(needle))
    }

    /// Whether it is in the library; `None` for what can't be saved (a playlist is canon's own,
    /// and a service's playlist or mix is never ingested at all).
    #[must_use]
    pub fn saved(&self) -> Option<bool> {
        match self {
            Item::Track(t) => Some(t.saved),
            Item::Album(a) => Some(a.saved),
            Item::Artist(a) => Some(a.saved),
            Item::Playlist(_) | Item::Remote(_) | Item::Entry(_) => None,
        }
    }

    pub(crate) fn set_saved(&mut self, saved: bool) {
        match self {
            Item::Track(t) => t.saved = saved,
            Item::Album(a) => a.saved = saved,
            Item::Artist(a) => a.saved = saved,
            Item::Playlist(_) | Item::Remote(_) | Item::Entry(_) => {}
        }
    }

    /// This item as a list of tracks to copy or merge somewhere; `None` for what isn't a list
    /// (a track, an artist).
    #[must_use]
    pub fn list(&self) -> Option<Listed> {
        match self {
            Item::Album(_) | Item::Playlist(_) | Item::Remote(_) => Some(Listed {
                item: self.item_ref()?,
                name: self.name().to_owned(),
                mine: matches!(self, Item::Playlist(_)),
            }),
            Item::Track(_) | Item::Artist(_) | Item::Entry(_) => None,
        }
    }
}

/// A list to copy or merge somewhere: what names it to the daemon, what to call it, and whether
/// it is one of canon's own playlists.
///
/// `mine` is the one thing an [`ItemRef`] can't say for itself — a local playlist and a local
/// album are both `ItemRef::Entity` — and it is what tells a *duplicate* from a *copy*: two of
/// canon's own playlists with the same name can't be told apart, so copying one has to ask for
/// a new name (canon-ef86), while copying a service's keeps the name it has upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub item: ItemRef,
    pub name: String,
    pub mine: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Heading(String),
    Item(Item),
}

/// Where a page's rows come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// The saved library, of one kind, paged, and narrowed to what holds the query if there is
    /// one (Library's `/`, a filter on the server).
    Library(EntityKind, Option<String>),
    /// Sources' own list: its entries, grouped by service. Built by the client; never asked for.
    Sources,
    /// One track's detail: what it is, where it is, and the albums and artists it links to.
    Track(EntityId),
    Playlist(EntityId),
    Album(EntityId),
    Artist(EntityId),
    /// A search: of one service's catalog, or (`None`) of every connected one.
    Search(Option<Service>, String),
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
            Item::Track(t) => Some(Source::Track(t.id)),
            Item::Entry(Entry::Search(_)) => None,
            Item::Entry(Entry::ServicePlaylists(service)) => {
                Some(Source::ServicePlaylists(*service))
            }
            Item::Entry(Entry::ServiceMixes(service)) => Some(Source::ServiceMixes(*service)),
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

    /// This page's own contents as a list to copy or merge somewhere, and whether that list is
    /// one of canon's own playlists; `None` for what isn't a list (a library listing, a search,
    /// an artist). The page supplies the name, which is its title.
    #[must_use]
    pub fn list(&self) -> Option<(ItemRef, bool)> {
        match self {
            Source::Playlist(id) => Some((ItemRef::Entity { entity: *id }, true)),
            Source::Album(id) => Some((ItemRef::Entity { entity: *id }, false)),
            Source::ServicePlaylist(service, id) => Some((
                ItemRef::Service {
                    service: *service,
                    id: id.clone(),
                    kind: EntityKind::Playlist,
                },
                false,
            )),
            Source::Mix(service, mix) => Some((
                ItemRef::Mix {
                    service: *service,
                    mix: mix.clone(),
                },
                false,
            )),
            _ => None,
        }
    }

    /// Whether a change to canon's own playlists changes what this page shows, and so whether
    /// reading it again after a create, rename or delete is worth a round trip (canon-28ff).
    /// A service's listing is the service's own: nothing canon does to its library moves it.
    #[must_use]
    pub fn follows_playlists(&self) -> bool {
        matches!(
            self,
            Source::Playlist(_) | Source::Library(EntityKind::Playlist, _)
        )
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
    /// Set while a [`Page::reload`] is in flight, so the reply puts the cursor back where the
    /// user left it instead of at the top the way a freshly opened page does.
    reloading: bool,
    /// A search of every service, built up from the partial replies that have come so far; the
    /// final reply replaces it.
    partial: Option<SearchView>,
    /// A track's detail page: the track itself, drawn above its albums and artists.
    pub(crate) track: Option<TrackView>,
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
            reloading: false,
            partial: None,
            track: None,
        }
    }

    /// The list the cursor is on, or — when it is on a track inside one — the list this page is
    /// showing: what `c` copies and `M` merges. A service's playlist, a local playlist, an album
    /// and a mix are all the same kind of source here (canon-5b2b); `mine` is what tells them
    /// apart when it matters.
    pub(crate) fn list(&self) -> Option<Listed> {
        self.selected().and_then(Item::list).or_else(|| {
            self.source.list().map(|(item, mine)| Listed {
                item,
                name: self.title.clone(),
                mine,
            })
        })
    }

    /// Start this page over: its rows go, so its request asks from the top again and the reply
    /// replaces what was there rather than appending the next page to it. The cursor stays put,
    /// to be settled against whatever comes back.
    pub(crate) fn reload(&mut self) -> ClientMessage {
        self.rows.clear();
        self.total = None;
        self.loading = true;
        self.reloading = true;
        self.request()
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
            // Built in place (`App::sources_page`); never asked for. `Services` is a harmless
            // stand-in for the type's sake: nothing sends it for this source.
            Source::Sources => ClientMessage::Services,
            Source::Library(kind, query) => ClientMessage::Library {
                kind: *kind,
                query: query.clone(),
                limit: Some(LIBRARY_PAGE),
                offset: self.loaded(),
            },
            Source::Track(id) => ClientMessage::Track { item: entity(*id) },
            Source::Playlist(id) => ClientMessage::Playlist { playlist: *id },
            Source::Album(id) => ClientMessage::Album { item: entity(*id) },
            Source::Artist(id) => ClientMessage::Artist { item: entity(*id) },
            Source::Search(service, query) => ClientMessage::Search {
                query: query.clone(),
                service: *service,
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
            ReplyData::Track(detail) => {
                self.title = detail.track.title.clone();
                let mut rows = Vec::new();
                section(&mut rows, "Albums", detail.albums, Item::Album);
                section(&mut rows, "Artists", detail.artists, Item::Artist);
                self.rows = rows;
                self.track = Some(detail.track);
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
                self.partial = None;
                self.rows = search_rows(found);
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
        if std::mem::take(&mut self.reloading) {
            self.settle_cursor();
        } else if first {
            self.cursor = self.first_item().unwrap_or(0);
        }
    }

    /// Take in a partial reply (one service's search results, more to come): show what has
    /// arrived so far, merged, and keep waiting for the final reply.
    pub(crate) fn fold_partial(&mut self, data: ReplyData) {
        let ReplyData::Search(found) = data else {
            return;
        };
        let first = self.rows.is_empty();
        let mut so_far = self.partial.take().unwrap_or_default();
        so_far.merge(found);
        self.rows = search_rows(so_far.clone());
        self.partial = Some(so_far);
        if first {
            self.cursor = self.first_item().unwrap_or(0);
        } else {
            self.settle_cursor();
        }
    }

    fn first_item(&self) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, Row::Item(_)))
    }

    /// Put the cursor back on an item after a reload: where it was, or the nearest row above it
    /// when what it was on has gone (a playlist just deleted, the last row of a shorter list).
    fn settle_cursor(&mut self) {
        let at = self.cursor;
        self.cursor = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| matches!(row, Row::Item(_)).then_some(i))
            .take_while(|i| *i <= at)
            .last()
            .or_else(|| self.first_item())
            .unwrap_or(0);
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
                tracks.extend(item.item_ref());
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
pub(crate) fn service_name(service: Service) -> &'static str {
    match service {
        Service::Local => "Local",
        Service::Tidal => "Tidal",
        Service::Spotify => "Spotify",
    }
}

/// Sources' own list: its entries under a heading per service. Built here, not fetched.
#[must_use]
pub fn sources_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut group = "";
    for entry in Entry::ALL {
        if entry.group() != group {
            group = entry.group();
            rows.push(Row::Heading(group.to_owned()));
        }
        rows.push(Row::Item(Item::Entry(entry)));
    }
    rows
}

/// A search's rows: each kind under its heading.
fn search_rows(found: SearchView) -> Vec<Row> {
    let mut rows = Vec::new();
    section(&mut rows, "Tracks", found.tracks, Item::Track);
    section(&mut rows, "Albums", found.albums, Item::Album);
    section(&mut rows, "Artists", found.artists, Item::Artist);
    section(&mut rows, "Playlists", found.playlists, |view| {
        Item::Remote(Remote::playlist(view))
    });
    rows
}

fn section<T>(rows: &mut Vec<Row>, heading: &str, items: Vec<T>, wrap: fn(T) -> Item) {
    if items.is_empty() {
        return;
    }
    rows.push(Row::Heading(heading.to_owned()));
    rows.extend(items.into_iter().map(|item| Row::Item(wrap(item))));
}
