//! The TUI's state and behaviour, with no I/O: server messages and keys go in, requests come
//! out ([`App::take_requests`]), and [`crate::render`] draws it. The live terminal and the
//! headless driver both run this one `App`, so what a script sees is what a person sees.
//!
//! Time is handed in ([`App::tick`]) rather than read, so position interpolation is testable and
//! a headless frame is a pure function of what arrived.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use canon_api::protocol::QueueAt;
use canon_api::{ClientEnvelope, ClientMessage, PROTOCOL_VERSION, ReplyData, ServerMessage};
use canon_core::{EntityId, PlaybackState, PlayerSnapshot, Repeat, SinkInfo, TrackRef};
use canon_library::EntityKind;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::browse::{Page, Source, Tab};

/// How far a seek key moves.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// How far a volume key moves, as a fraction of full.
pub const VOLUME_STEP: f32 = 0.05;

/// Where the connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    /// Gone, and why. The app stays up to show it.
    Lost(String),
}

/// What an outstanding request was for, so its reply lands in the right place.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Queue,
    Sinks,
    /// A command whose only interesting reply is an error.
    Command,
    /// A command that says, once done, what it did.
    Done(String),
    /// Rows for the page with this id.
    Page(u64),
    /// Saving (or unsaving) an entity, shown as such once the library agrees.
    Save {
        entity: EntityId,
        saved: bool,
    },
}

pub struct App {
    pub(crate) link: Link,
    pub(crate) snapshot: PlayerSnapshot,
    /// When `snapshot` arrived, for interpolating position between snapshots.
    snapshot_at: Instant,
    pub(crate) now: Instant,
    /// The queue's entries as of `queue_revision`.
    pub(crate) queue: Vec<TrackRef>,
    queue_revision: Option<u64>,
    /// The known outputs, for naming the selected one.
    pub(crate) sinks: Vec<SinkInfo>,
    pub(crate) cursor: usize,
    /// First visible queue row, kept across frames so moving the cursor scrolls only as far as
    /// it must.
    pub(crate) scroll: std::cell::Cell<usize>,
    /// Visible queue rows at the last draw, for paging.
    pub(crate) page: usize,
    /// A one-line message: the last error, or what just happened.
    pub(crate) notice: Option<String>,
    pub(crate) help: bool,
    pub(crate) tab: Tab,
    /// The browsing tabs' page stacks (Library, Playlists, Search), root first.
    pub(crate) stacks: [Vec<Page>; 3],
    /// Which kind of saved item the Library tab lists.
    pub(crate) library_kind: EntityKind,
    /// The search being typed, while the search box has the keys.
    pub(crate) input: Option<String>,
    next_page: u64,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    outbox: Vec<ClientEnvelope>,
    quit: bool,
}

impl App {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            link: Link::Connecting,
            snapshot: PlayerSnapshot::idle(),
            snapshot_at: now,
            now,
            queue: Vec::new(),
            queue_revision: None,
            sinks: Vec::new(),
            cursor: 0,
            scroll: std::cell::Cell::new(0),
            page: 10,
            notice: None,
            help: false,
            tab: Tab::Queue,
            stacks: [Vec::new(), Vec::new(), Vec::new()],
            library_kind: EntityKind::Track,
            input: None,
            next_page: 1,
            next_id: 1,
            pending: HashMap::new(),
            outbox: Vec::new(),
            quit: false,
        }
    }

    /// The requests to send, in order. Draining them is the caller's side of the bargain.
    pub fn take_requests(&mut self) -> Vec<ClientEnvelope> {
        std::mem::take(&mut self.outbox)
    }

    /// Whether replies are still owed for requests already sent.
    #[must_use]
    pub fn awaiting_replies(&self) -> bool {
        !self.pending.is_empty()
    }

    #[must_use]
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The terminal is now `height` rows tall: the queue gets what the bars leave.
    pub fn resize(&mut self, height: u16) {
        self.page = usize::from(height.saturating_sub(5)).max(1);
    }

    /// Advance the clock the app draws against.
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
    }

    /// The connection is gone.
    pub fn lost(&mut self, why: String) {
        self.link = Link::Lost(why);
        self.pending.clear();
    }

    /// Something arrived that this client can't read.
    pub fn unreadable(&mut self, what: &str) {
        self.notice = Some(format!("unreadable message from the daemon: {what}"));
    }

    /// Where playback is now: the snapshot's position, carried forward at its rate since it
    /// arrived, and held at the end of the track.
    #[must_use]
    pub fn position(&self) -> Duration {
        let reported = Duration::from_millis(self.snapshot.position_ms);
        let moving = self.snapshot.state == PlaybackState::Playing && self.snapshot.rate > 0.0;
        let position = if moving {
            let since = self.now.saturating_duration_since(self.snapshot_at);
            reported + since.mul_f32(self.snapshot.rate)
        } else {
            reported
        };
        match self.snapshot.duration_ms {
            Some(duration) => position.min(Duration::from_millis(duration)),
            None => position,
        }
    }

    /// The selected output, if it is one of those known.
    fn selected_output(&self) -> Option<&SinkInfo> {
        let id = self.snapshot.sink.as_ref()?;
        self.sinks
            .iter()
            .find(|sink| sink.id == *id || sink.protocols.iter().any(|endpoint| endpoint.id == *id))
    }

    /// The display name of the selected output: its name when known, else its id; the local
    /// output when no renderer is selected.
    #[must_use]
    pub fn output_name(&self) -> &str {
        match (self.selected_output(), &self.snapshot.sink) {
            (Some(sink), _) => &sink.name,
            (None, Some(id)) => &id.0,
            (None, None) => "local",
        }
    }

    /// A one-line digest for headless frames: what the app believes, to check a frame against.
    #[must_use]
    pub fn state_header(&self) -> String {
        let link = match &self.link {
            Link::Connecting => "connecting",
            Link::Connected => "connected",
            Link::Lost(_) => "lost",
        };
        let mut header = format!(
            "link={link} state={} seq={} queue={}/{} tab={}",
            state_name(self.snapshot.state),
            self.snapshot.seq,
            self.snapshot.queue.index,
            self.queue.len(),
            self.tab.name().to_lowercase(),
        );
        match self.page() {
            Some(page) => header.push_str(&format!(
                " depth={} rows={}{} cursor={}",
                self.tab.stack().map_or(0, |s| self.stacks[s].len()),
                page.loaded(),
                page.total.map(|t| format!("/{t}")).unwrap_or_default(),
                page.cursor
            )),
            None => header.push_str(&format!(" cursor={}", self.cursor)),
        }
        header.push_str(&format!(" pending={}", self.pending.len()));
        header
    }

    // --- server messages ---

    /// Fold one server message into the state.
    pub fn apply(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Hello { protocol } => {
                self.link = Link::Connected;
                if protocol != PROTOCOL_VERSION {
                    self.notice = Some(format!(
                        "the daemon speaks protocol {protocol}; this client, {PROTOCOL_VERSION}"
                    ));
                }
                self.request(ClientMessage::ListSinks, Pending::Sinks);
            }
            ServerMessage::Snapshot { snapshot } => self.snapshot_arrived(snapshot),
            ServerMessage::Reply {
                id,
                ok,
                result,
                error,
            } => {
                let pending = id.and_then(|id| self.pending.remove(&id));
                if !ok {
                    if let Some(Pending::Page(page)) = pending
                        && let Some(page) = self.page_by_id(page)
                    {
                        page.loading = false;
                    }
                    self.notice = Some(error.unwrap_or_else(|| "request failed".into()));
                    return;
                }
                match (pending, result) {
                    (Some(Pending::Page(page)), Some(data)) => {
                        if let Some(page) = self.page_by_id(page) {
                            page.fold(data);
                        }
                    }
                    (Some(Pending::Done(what)), _) => self.notice = Some(what),
                    (Some(Pending::Save { entity, saved }), _) => self.saved(entity, saved),
                    (
                        Some(Pending::Queue),
                        Some(ReplyData::Queue {
                            revision, tracks, ..
                        }),
                    ) => {
                        let first = self.queue_revision.is_none();
                        self.queue = tracks;
                        self.queue_revision = Some(revision);
                        if first {
                            self.cursor = self.snapshot.queue.index;
                        }
                        // Changed again while the reply was on its way.
                        if revision < self.snapshot.queue.revision {
                            self.fetch_queue();
                        }
                        self.clamp_cursor();
                    }
                    (Some(Pending::Sinks), Some(ReplyData::Sinks { sinks })) => self.sinks = sinks,
                    _ => {}
                }
            }
        }
    }

    fn snapshot_arrived(&mut self, snapshot: PlayerSnapshot) {
        let queue_moved = self.queue_revision != Some(snapshot.queue.revision);
        let playing_moved = snapshot.queue.index != self.snapshot.queue.index
            || self.snapshot.track.is_none() && snapshot.track.is_some();
        let output_moved = snapshot.sink != self.snapshot.sink;
        let follow = self.cursor == self.snapshot.queue.index;
        self.snapshot = snapshot;
        self.snapshot_at = self.now;
        if queue_moved {
            self.fetch_queue();
        }
        if output_moved && self.selected_output().is_none() && self.snapshot.sink.is_some() {
            self.request(ClientMessage::ListSinks, Pending::Sinks);
        }
        // The cursor rides along with the playing entry unless it was moved away from it.
        if playing_moved && follow {
            self.cursor = self.snapshot.queue.index;
        }
        self.clamp_cursor();
    }

    fn fetch_queue(&mut self) {
        if !self.pending.values().any(|p| p == &Pending::Queue) {
            self.request(ClientMessage::Queue, Pending::Queue);
        }
    }

    fn request(&mut self, message: ClientMessage, pending: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        self.outbox.push(ClientEnvelope {
            id: Some(id),
            message,
        });
    }

    fn command(&mut self, message: ClientMessage) {
        self.notice = None;
        self.request(message, Pending::Command);
    }

    // --- keys ---

    /// Apply one key.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.help {
            // Any key closes help; `q` still quits.
            self.help = false;
            if key.code == KeyCode::Char('q') {
                self.quit = true;
            }
            return;
        }
        if self.input.is_some() {
            self.search_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,

            // Views.
            KeyCode::Tab => self.switch_tab(self.tab_at(1)),
            KeyCode::BackTab => self.switch_tab(self.tab_at(-1)),
            KeyCode::Char(c @ '1'..='4') => {
                self.switch_tab(Tab::ALL[usize::from(c as u8 - b'1')]);
            }
            KeyCode::Char('/') => {
                self.switch_tab(Tab::Search);
                self.input = Some(String::new());
            }

            // Transport.
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Char('n') => self.command(ClientMessage::Next),
            KeyCode::Char('p') => self.command(ClientMessage::Previous),
            KeyCode::Right => self.seek_by(SEEK_STEP, true),
            KeyCode::Left => self.seek_by(SEEK_STEP, false),
            KeyCode::Char('+' | '=') => self.volume_by(VOLUME_STEP),
            KeyCode::Char('-') => self.volume_by(-VOLUME_STEP),
            KeyCode::Char('m') => self.command(ClientMessage::SetMuted {
                muted: !self.snapshot.muted,
            }),
            KeyCode::Char('s') => self.command(ClientMessage::Shuffle),
            KeyCode::Char('r') => self.command(ClientMessage::Repeat {
                mode: match self.snapshot.queue.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                },
            }),
            _ if self.tab == Tab::Queue => self.queue_key(key),
            _ => self.page_key(key),
        }
    }

    fn queue_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(self.page_rows()),
            KeyCode::PageUp => self.move_cursor(-self.page_rows()),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => self.cursor = self.queue.len().saturating_sub(1),
            KeyCode::Char('.') => self.cursor = self.snapshot.queue.index,
            KeyCode::Enter if !self.queue.is_empty() => {
                self.command(ClientMessage::Jump { index: self.cursor });
            }
            KeyCode::Char('d' | 'x') | KeyCode::Delete if !self.queue.is_empty() => {
                self.command(ClientMessage::Remove { index: self.cursor });
            }
            KeyCode::Char('J') => self.move_entry(1),
            KeyCode::Char('K') => self.move_entry(-1),
            _ => {}
        }
    }

    // --- browsing ---

    /// The page on screen, when the tab browses.
    pub(crate) fn page(&self) -> Option<&Page> {
        self.stacks[self.tab.stack()?].last()
    }

    fn page_mut(&mut self) -> Option<&mut Page> {
        self.stacks[self.tab.stack()?].last_mut()
    }

    fn page_by_id(&mut self, id: u64) -> Option<&mut Page> {
        self.stacks.iter_mut().flatten().find(|page| page.id == id)
    }

    fn tab_at(&self, by: isize) -> Tab {
        let at = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        let count = Tab::ALL.len();
        Tab::ALL[(at + count).saturating_add_signed(by) % count]
    }

    /// Show `tab`, loading its first page the first time.
    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        let Some(stack) = tab.stack() else {
            return;
        };
        if self.stacks[stack].is_empty() {
            let (title, source) = match tab {
                Tab::Library => (
                    kind_name(self.library_kind),
                    Source::Library(self.library_kind),
                ),
                Tab::Playlists => ("Playlists".to_owned(), Source::Playlists),
                // Search starts empty: there is nothing to show until something is asked.
                _ => return,
            };
            self.push_page(stack, title, source);
        }
    }

    /// Open a page on top of `stack`, and ask for its rows.
    fn push_page(&mut self, stack: usize, title: String, source: Source) {
        let id = self.next_page;
        self.next_page += 1;
        let mut page = Page::new(id, title, source);
        page.loading = true;
        let request = page.request();
        self.stacks[stack].push(page);
        self.request(request, Pending::Page(id));
    }

    fn page_key(&mut self, key: KeyEvent) {
        let Some(stack) = self.tab.stack() else {
            return;
        };
        let rows = self.page_rows();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.with_page(|page| page.move_by(1)),
            KeyCode::Char('k') | KeyCode::Up => self.with_page(|page| page.move_by(-1)),
            KeyCode::PageDown => self.with_page(|page| page.move_by(rows)),
            KeyCode::PageUp => self.with_page(|page| page.move_by(-rows)),
            KeyCode::Char('g') | KeyCode::Home => self.with_page(Page::go_top),
            KeyCode::Char('G') | KeyCode::End => self.with_page(Page::go_bottom),
            KeyCode::Enter | KeyCode::Char('l') => self.activate(stack),
            KeyCode::Char('a') => self.enqueue(QueueAt::End),
            KeyCode::Char('A') => self.enqueue(QueueAt::Next),
            KeyCode::Char('P') => self.enqueue(QueueAt::Now),
            KeyCode::Char('*') => self.toggle_saved(),
            KeyCode::Char('h') | KeyCode::Esc | KeyCode::Backspace
                if self.stacks[stack].len() > 1 =>
            {
                self.stacks[stack].pop();
            }
            KeyCode::Char('[' | ']') if self.tab == Tab::Library && self.stacks[0].len() == 1 => {
                self.library_kind = match (self.library_kind, key.code) {
                    (EntityKind::Track, KeyCode::Char(']'))
                    | (EntityKind::Artist, KeyCode::Char('[')) => EntityKind::Album,
                    (EntityKind::Album, KeyCode::Char(']'))
                    | (EntityKind::Track, KeyCode::Char('[')) => EntityKind::Artist,
                    _ => EntityKind::Track,
                };
                self.stacks[0].clear();
                self.switch_tab(Tab::Library);
            }
            _ => {}
        }
        self.load_more();
    }

    fn with_page(&mut self, f: impl FnOnce(&mut Page)) {
        if let Some(page) = self.page_mut() {
            f(page);
        }
    }

    /// Ask for the next page of a paged list once the cursor nears the end of what's loaded.
    fn load_more(&mut self) {
        let Some(page) = self.page_mut().filter(|page| page.wants_more()) else {
            return;
        };
        page.loading = true;
        let (id, request) = (page.id, page.request());
        self.request(request, Pending::Page(id));
    }

    /// Enter: a track plays its section from there; anything else opens.
    fn activate(&mut self, stack: usize) {
        let Some(page) = self.page() else {
            return;
        };
        let Some(item) = page.selected().cloned() else {
            return;
        };
        match Source::of(&item) {
            Some(source) => self.push_page(stack, item.name().to_owned(), source),
            None => {
                let (items, start) = page.section_tracks();
                self.notice = None;
                self.request(
                    ClientMessage::QueueAdd {
                        items,
                        at: QueueAt::Now,
                        start,
                    },
                    Pending::Done(format!("playing \"{}\"", item.name())),
                );
            }
        }
    }

    /// Queue the selected item: a track, or everything an album, artist or playlist holds.
    fn enqueue(&mut self, at: QueueAt) {
        let Some(item) = self.page().and_then(Page::selected).cloned() else {
            return;
        };
        let name = item.name();
        let done = match at {
            QueueAt::End => format!("added \"{name}\" to the queue"),
            QueueAt::Next => format!("\"{name}\" plays next"),
            QueueAt::Now => format!("playing \"{name}\""),
        };
        self.notice = None;
        self.request(
            ClientMessage::QueueAdd {
                items: vec![item.item_ref()],
                at,
                start: 0,
            },
            Pending::Done(done),
        );
    }

    fn toggle_saved(&mut self) {
        let Some(item) = self.page().and_then(Page::selected) else {
            return;
        };
        let Some(saved) = item.saved() else {
            self.notice = Some("playlists are canon's own; there's nothing to save".into());
            return;
        };
        let (entity, target) = (item.id(), item.item_ref());
        let message = if saved {
            ClientMessage::Unsave { item: target }
        } else {
            ClientMessage::Save { item: target }
        };
        self.request(
            message,
            Pending::Save {
                entity,
                saved: !saved,
            },
        );
    }

    /// The library agreed: `entity` is (or isn't) saved now, wherever it is shown.
    fn saved(&mut self, entity: EntityId, saved: bool) {
        let mut name = None;
        for page in self.stacks.iter_mut().flatten() {
            for row in &mut page.rows {
                if let crate::browse::Row::Item(item) = row
                    && item.id() == entity
                {
                    item.set_saved(saved);
                    name.get_or_insert_with(|| item.name().to_owned());
                }
            }
        }
        let name = name.unwrap_or_else(|| "it".into());
        self.notice = Some(if saved {
            format!("saved \"{name}\"")
        } else {
            format!("removed \"{name}\" from the library")
        });
    }

    /// A key while the search box is open.
    fn search_key(&mut self, key: KeyEvent) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) => input.push(c),
            KeyCode::Enter => {
                let query = input.trim().to_owned();
                self.input = None;
                if !query.is_empty() {
                    self.stacks[2].clear();
                    self.push_page(2, format!("\"{query}\""), Source::Search(query));
                }
            }
            _ => {}
        }
    }

    fn toggle_play(&mut self) {
        let message = if self.snapshot.state == PlaybackState::Playing {
            ClientMessage::Pause
        } else {
            ClientMessage::Play
        };
        self.command(message);
    }

    fn seek_by(&mut self, step: Duration, forward: bool) {
        if self.snapshot.track.is_none() {
            return;
        }
        let at = self.position();
        let to = if forward {
            at + step
        } else {
            at.saturating_sub(step)
        };
        let to = match self.snapshot.duration_ms {
            Some(duration) => to.min(Duration::from_millis(duration)),
            None => to,
        };
        #[allow(clippy::cast_possible_truncation)]
        self.command(ClientMessage::Seek {
            position_ms: to.as_millis() as u64,
        });
    }

    fn volume_by(&mut self, step: f32) {
        let volume = (self.snapshot.volume + step).clamp(0.0, 1.0);
        // Snap to the step grid, so repeated presses land on round percentages.
        let volume = (volume / VOLUME_STEP).round() * VOLUME_STEP;
        self.command(ClientMessage::SetVolume { volume });
    }

    fn page_rows(&self) -> isize {
        isize::try_from(self.page.max(1)).unwrap_or(1)
    }

    fn move_cursor(&mut self, by: isize) {
        if self.queue.is_empty() {
            return;
        }
        let last = self.queue.len() - 1;
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// Move the entry under the cursor one place, and the cursor with it.
    fn move_entry(&mut self, by: isize) {
        let from = self.cursor;
        let Some(to) = from
            .checked_add_signed(by)
            .filter(|to| *to < self.queue.len())
        else {
            return;
        };
        self.command(ClientMessage::Move { from, to });
        self.cursor = to;
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.queue.len().saturating_sub(1));
    }
}

/// A library kind as a heading.
fn kind_name(kind: EntityKind) -> String {
    match kind {
        EntityKind::Track => "Tracks",
        EntityKind::Album => "Albums",
        EntityKind::Artist => "Artists",
        EntityKind::Playlist => "Playlists",
    }
    .to_owned()
}

/// A playback state as a word.
#[must_use]
pub fn state_name(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Idle => "idle",
        PlaybackState::Loading => "loading",
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Ended => "ended",
        PlaybackState::Error => "error",
    }
}
