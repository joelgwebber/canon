//! The TUI's state and behaviour, with no I/O: server messages and keys go in, requests come
//! out ([`App::take_requests`]), and [`crate::render`] draws it. The live terminal and the
//! headless driver both run this one `App`, so what a script sees is what a person sees.
//!
//! Time is handed in ([`App::tick`]) rather than read, so position interpolation is testable and
//! a headless frame is a pure function of what arrived.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use canon_api::protocol::{QueueAt, ServiceView};
use canon_api::{ClientEnvelope, ClientMessage, PROTOCOL_VERSION, ReplyData, ServerMessage};
use canon_core::{
    EntityId, Health, LoginFlow, LoginStatus, OutputMode, PlaybackState, PlayerSnapshot, Repeat,
    Settings, SinkInfo, TrackRef,
};
use canon_library::{EntityKind, ItemRef};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::browse::{Item, Listed, Page, Shelf, Source, Tab};
use crate::setup::{self, Login, SetupRow, Toggle};

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
    /// The same, for a command that changes what the page on screen is showing: it is read
    /// again once the daemon has done it, so a copy, rename or delete lands in the listing
    /// without the tab having to be left and re-entered (canon-28ff).
    Reload(String),
    /// Rows for the page with this id.
    Page(u64),
    /// Saving (or unsaving) an entity, shown as such once the library agrees.
    Save {
        entity: EntityId,
        saved: bool,
    },
    Services,
    Settings,
    /// A settings change; the settings are read back once it lands.
    SetSettings(String),
    /// A sign-in starting: the reply says what the user has to do.
    Connect,
    /// A sign-in being completed or polled.
    LoginPoll,
    /// A sign-out; the services are read back once it lands.
    Disconnect(String),
}

/// What a typed string is being collected for. The app has exactly one text-entry mechanism
/// ([`App::prompt`]); this is how it knows what pressing enter means (canon-4c6e).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Asking {
    /// A query to search for.
    Search,
    /// A new name for one of canon's own playlists.
    Rename { playlist: EntityId, was: String },
    /// A name for a new playlist holding `item` — a copy of a list, or (with no item) an
    /// empty one.
    Copy { item: Option<ItemRef> },
}

/// What a merge is holding until a target playlist is picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Merging {
    pub(crate) item: ItemRef,
    pub(crate) name: String,
    /// One track (`L`) rather than a whole list (`M`). The daemon does the same thing with
    /// either — append what the target hasn't got — but what to say about it differs.
    pub(crate) track: bool,
}

/// A line of text being typed, and what it is for.
pub(crate) struct Prompt {
    pub(crate) asking: Asking,
    pub(crate) text: String,
}

impl Prompt {
    /// What to put in front of the text, so the box says what it is asking for.
    pub(crate) fn label(&self) -> &'static str {
        match self.asking {
            Asking::Search => "search",
            Asking::Rename { .. } => "rename to",
            Asking::Copy { item: Some(_) } => "copy as",
            Asking::Copy { item: None } => "new playlist",
        }
    }
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
    /// Which listing the Playlists tab shows: canon's own, or a service's.
    pub(crate) shelf: Shelf,
    /// A merge waiting for its target: a whole list (`M`) or one track (`L`). While it is held,
    /// the Playlists tab's own listing is the picker — enter on a playlist merges into it.
    pub(crate) merging: Option<Merging>,
    /// A delete armed by one press of `D` and waiting for the second: the playlist and its
    /// name. Deleting takes the playlist's version history with it (canon-120d) and is the one
    /// thing here that can't be undone, so it alone asks twice.
    pub(crate) deleting: Option<(EntityId, String)>,
    /// What is being typed, while the prompt has the keys.
    pub(crate) prompt: Option<Prompt>,
    /// Each service's ways in and their state, as last listed.
    pub(crate) services: Vec<ServiceView>,
    /// The settings as last read.
    pub(crate) settings: Option<Settings>,
    /// The Outputs and Settings tabs' cursors and scroll offsets.
    pub(crate) setup_cursor: [usize; 2],
    pub(crate) setup_scroll: [std::cell::Cell<usize>; 2],
    /// A sign-in in progress.
    pub(crate) login: Option<Login>,
    next_page: u64,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    outbox: Vec<ClientEnvelope>,
    /// Text to put on the user's clipboard, which only the terminal can do.
    clipboard: Option<String>,
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
            shelf: Shelf::Mine,
            merging: None,
            deleting: None,
            prompt: None,
            services: Vec::new(),
            settings: None,
            setup_cursor: [0, 0],
            setup_scroll: Default::default(),
            login: None,
            next_page: 1,
            next_id: 1,
            pending: HashMap::new(),
            outbox: Vec::new(),
            clipboard: None,
            quit: false,
        }
    }

    /// Text the app wants on the clipboard, if any since last asked. The terminal does the
    /// copying (see `runtime::copy`); headless, there is no clipboard and it is dropped.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
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
        // A device-code sign-in asks, every so often, whether it has been approved yet.
        let due = self.login.as_ref().is_some_and(|login| {
            matches!(login.flow, LoginFlow::DeviceCode { .. })
                && login.next_poll.is_some_and(|at| at <= now)
        });
        if due
            && !self.pending.values().any(|p| p == &Pending::LoginPoll)
            && let Some(login) = self.login.as_mut()
        {
            login.next_poll = Some(now + login.interval);
            let method = login.method.clone();
            self.request(
                ClientMessage::ConnectComplete {
                    method,
                    redirect: None,
                },
                Pending::LoginPoll,
            );
        }
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
            None => {
                let cursor = self
                    .setup_tab()
                    .map_or(self.cursor, |tab| self.setup_cursor[tab]);
                header.push_str(&format!(" cursor={cursor}"));
            }
        }
        if let Some(login) = &self.login {
            header.push_str(&format!(" login={}", login.method));
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
                    (Some(Pending::Reload(what)), _) => {
                        self.notice = Some(what);
                        self.reload_page();
                    }
                    (Some(Pending::Services), Some(ReplyData::Services { services })) => {
                        self.services = services;
                        self.clamp_setup();
                    }
                    (Some(Pending::Settings), Some(ReplyData::Settings { settings })) => {
                        self.settings = Some(settings);
                        self.clamp_setup();
                    }
                    (Some(Pending::SetSettings(what)), _) => {
                        self.notice = Some(what);
                        self.request(ClientMessage::Settings, Pending::Settings);
                    }
                    (Some(Pending::Connect), Some(ReplyData::Connecting { method, login })) => {
                        self.login_started(method, login);
                    }
                    (Some(Pending::LoginPoll), Some(ReplyData::Login { status })) => {
                        self.login_answered(status);
                    }
                    (Some(Pending::Disconnect(method)), _) => {
                        self.notice = Some(format!("signed out of {method}"));
                        self.request(ClientMessage::Services, Pending::Services);
                    }
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
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        if self.login.is_some() {
            self.login_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,

            // Views.
            KeyCode::Tab => self.switch_tab(self.tab_at(1)),
            KeyCode::BackTab => self.switch_tab(self.tab_at(-1)),
            KeyCode::Char(c @ '1'..='6') => {
                self.switch_tab(Tab::ALL[usize::from(c as u8 - b'1')]);
            }
            KeyCode::Char('/') => {
                self.switch_tab(Tab::Search);
                self.ask(Asking::Search, String::new());
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
            _ if self.setup_tab().is_some() => self.setup_key(key),
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
        // An armed delete doesn't follow the user to another view and lie in wait there.
        self.deleting = None;
        // What these show changes behind our back (speakers come and go, logins lapse): read it
        // afresh on every visit.
        match tab {
            Tab::Outputs => {
                self.request(ClientMessage::ListSinks, Pending::Sinks);
                self.request(ClientMessage::Settings, Pending::Settings);
            }
            Tab::Settings => {
                self.request(ClientMessage::Services, Pending::Services);
                self.request(ClientMessage::Settings, Pending::Settings);
            }
            _ => {}
        }
        let Some(stack) = tab.stack() else {
            return;
        };
        if self.stacks[stack].is_empty() {
            let (title, source) = match tab {
                Tab::Library => (
                    kind_name(self.library_kind),
                    Source::Library(self.library_kind),
                ),
                Tab::Playlists => (self.shelf.name(), self.shelf.source()),
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
            KeyCode::Char('c') => self.copy_list(),
            KeyCode::Char('M') => self.start_merge(),
            KeyCode::Char('L') => self.start_adding_track(),
            KeyCode::Char('d' | 'x') | KeyCode::Delete => self.remove_from_playlist(),
            KeyCode::Char('J') => self.move_in_playlist(1),
            KeyCode::Char('K') => self.move_in_playlist(-1),
            KeyCode::Char('R') => self.rename_playlist(),
            KeyCode::Char('D') => self.delete_playlist(),
            KeyCode::Char('N') if self.tab == Tab::Playlists => {
                self.ask(Asking::Copy { item: None }, String::new());
            }
            // A merge or a delete still waiting on the user gives up before `esc` means "back".
            KeyCode::Esc if self.merging.is_some() => {
                let what = match self.merging.take() {
                    Some(Merging { track: true, .. }) => "add",
                    _ => "merge",
                };
                self.notice = Some(format!("{what} cancelled"));
            }
            KeyCode::Esc if self.deleting.is_some() => {
                self.deleting = None;
                self.notice = Some("delete cancelled".into());
            }
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
            KeyCode::Char('[' | ']') if self.tab == Tab::Playlists && self.stacks[1].len() == 1 => {
                let by = if key.code == KeyCode::Char(']') {
                    1
                } else {
                    -1
                };
                let at = Shelf::ALL
                    .iter()
                    .position(|s| *s == self.shelf)
                    .unwrap_or(0);
                let count = Shelf::ALL.len();
                self.show_shelf(Shelf::ALL[(at + count).saturating_add_signed(by) % count]);
            }
            _ => {}
        }
        self.load_more();
    }

    /// Show `shelf` on the Playlists tab, from the top.
    fn show_shelf(&mut self, shelf: Shelf) {
        self.shelf = shelf;
        self.stacks[1].clear();
        self.switch_tab(Tab::Playlists);
    }

    /// Read the page on screen again, in place: the same request it already knows how to build,
    /// its reply folding back into the page that is there rather than pushing a new one, so the
    /// stack and the cursor survive it (canon-28ff).
    ///
    /// Only pages that show canon's own playlists are worth re-reading after a playlist changed;
    /// a service's listing would cost a round trip upstream to come back exactly as it was.
    fn reload_page(&mut self) {
        let Some(page) = self
            .page_mut()
            .filter(|page| page.source.follows_playlists())
        else {
            return;
        };
        let (id, request) = (page.id, page.reload());
        self.request(request, Pending::Page(id));
    }

    /// The list the cursor is on, or — when it is on a track inside one — the list being shown:
    /// what `c` copies and `M` merges. A page of a service's playlists, a local playlist, an
    /// album and a mix are all the same kind of source here (canon-5b2b).
    fn list_source(&self) -> Option<Listed> {
        self.page()?.list()
    }

    /// Copy the list under the cursor into a brand-new playlist.
    ///
    /// A service's list keeps the name it has upstream: "Jazz practice" copied from Tidal should
    /// land as "Jazz practice". One of canon's own can't — a second "Road trip" beside the first
    /// is indistinguishable — so duplicating one asks what to call the copy (canon-ef86).
    fn copy_list(&mut self) {
        let Some(listed) = self.list_source() else {
            self.notice = Some("there's no list here to copy".into());
            return;
        };
        if listed.mine {
            let suggestion = format!("{} copy", listed.name);
            self.ask(
                Asking::Copy {
                    item: Some(listed.item),
                },
                suggestion,
            );
            return;
        }
        let name = listed.name;
        self.notice = None;
        self.request(
            ClientMessage::PlaylistCreate {
                name: name.clone(),
                items: vec![listed.item],
            },
            Pending::Reload(format!("copied \"{name}\" into a new playlist")),
        );
    }

    /// The local playlist the cursor is on, or the one the page is showing: what `R` renames and
    /// what `D` deletes. Neither is offered for a service's playlist (canon never writes back to
    /// one) or for an album.
    fn local_playlist(&self) -> Option<(EntityId, String)> {
        let page = self.page()?;
        match page.selected() {
            Some(Item::Playlist(playlist)) => Some((playlist.id, playlist.name.clone())),
            // On a track inside one, the playlist being shown — as `c` and `M` already read it.
            _ => match page.source {
                Source::Playlist(id) => Some((id, page.title.clone())),
                _ => None,
            },
        }
    }

    /// Rename the local playlist under the cursor, starting from the name it has.
    fn rename_playlist(&mut self) {
        let Some((playlist, was)) = self.local_playlist() else {
            self.notice = Some("only one of canon's own playlists can be renamed".into());
            return;
        };
        self.ask(
            Asking::Rename {
                playlist,
                was: was.clone(),
            },
            was,
        );
    }

    /// Delete the local playlist under the cursor: armed by the first `D`, done by the second.
    ///
    /// This is the only irreversible thing the TUI can do — the playlist's recorded versions go
    /// with it — so it is the only one that asks. Arming remembers *which* playlist, so moving
    /// the cursor and pressing `D` again arms the new row rather than deleting it outright.
    fn delete_playlist(&mut self) {
        let Some((playlist, name)) = self.local_playlist() else {
            self.notice = Some("only one of canon's own playlists can be deleted".into());
            return;
        };
        if self
            .deleting
            .as_ref()
            .is_some_and(|(id, _)| *id == playlist)
        {
            self.deleting = None;
            self.request(
                ClientMessage::PlaylistDelete { playlist },
                Pending::Reload(format!("deleted \"{name}\"")),
            );
            return;
        }
        self.notice = Some(format!(
            "delete \"{name}\" and its history? D again to confirm, esc to cancel"
        ));
        self.deleting = Some((playlist, name));
    }

    /// Hold the list under the cursor, and show canon's own playlists to pick a target from.
    fn start_merge(&mut self) {
        let Some(Listed { item, name, .. }) = self.list_source() else {
            self.notice = Some("there's no list here to merge".into());
            return;
        };
        self.hold_for_merge(Merging {
            item,
            name,
            track: false,
        });
    }

    /// Hold the track under the cursor, to add to a playlist picked the way `M` picks one.
    ///
    /// This is a merge of one track — the playlist takes it only if it hasn't got it — but it is
    /// its own key rather than `M` on a track, because `M` on a track inside a list already means
    /// "merge the list being shown", as `c` does (canon-028b).
    fn start_adding_track(&mut self) {
        let Some(Item::Track(track)) = self.page().and_then(Page::selected) else {
            self.notice = Some("there's no track here to add; M merges a whole list".into());
            return;
        };
        let held = Merging {
            item: ItemRef::Entity { entity: track.id },
            name: track.title.clone(),
            track: true,
        };
        self.hold_for_merge(held);
    }

    fn hold_for_merge(&mut self, held: Merging) {
        let verb = if held.track { "adding" } else { "merging" };
        self.notice = Some(format!(
            "{verb} \"{}\": enter on a playlist to take it, esc to cancel",
            held.name
        ));
        self.merging = Some(held);
        self.show_shelf(Shelf::Mine);
    }

    /// Finish a held merge into the playlist `target`, which only takes what it hasn't got.
    fn finish_merge(&mut self, target: EntityId, into: &str) {
        let Some(Merging { item, name, track }) = self.merging.take() else {
            return;
        };
        // A track the playlist already had is left as it was, and the reply can't say which
        // happened, so the notice says what is true either way.
        let done = if track {
            format!("\"{name}\" is in \"{into}\"")
        } else {
            format!("merged \"{name}\" into \"{into}\"")
        };
        self.notice = None;
        self.request(
            ClientMessage::PlaylistAdd {
                playlist: target,
                items: vec![item],
                at: None,
                merge: true,
            },
            Pending::Reload(done),
        );
    }

    /// The entry under the cursor in one of canon's own playlists, opened: the playlist, the
    /// entry's index in it, the track's name and the playlist's. `None` anywhere else — a
    /// service's playlist or a mix (canon never writes back to one), an album, and the listing
    /// of playlists itself, where the row is a whole playlist and `D` is what removes it.
    fn playlist_entry(&self) -> Option<(EntityId, usize, String, String)> {
        let page = self.page()?;
        let Source::Playlist(playlist) = page.source else {
            return None;
        };
        let Some(Item::Track(track)) = page.selected() else {
            return None;
        };
        // A playlist's page is its tracks in order with no headings, so the row is the index.
        Some((
            playlist,
            page.cursor,
            track.title.clone(),
            page.title.clone(),
        ))
    }

    /// Take the entry under the cursor out of the local playlist being shown, as `d` does in the
    /// queue. The page is read again once it's done, and the cursor settles on what is there.
    fn remove_from_playlist(&mut self) {
        let Some((playlist, index, track, name)) = self.playlist_entry() else {
            self.notice =
                Some("only a track in one of canon's own playlists can be removed".into());
            return;
        };
        self.notice = None;
        self.request(
            ClientMessage::PlaylistRemove { playlist, index },
            Pending::Reload(format!("removed \"{track}\" from \"{name}\"")),
        );
    }

    /// Move the entry under the cursor one place in the local playlist being shown, and the
    /// cursor with it, as `J`/`K` do in the queue.
    fn move_in_playlist(&mut self, by: isize) {
        let Some((playlist, from, track, _)) = self.playlist_entry() else {
            self.notice = Some("only a track in one of canon's own playlists can be moved".into());
            return;
        };
        let Some(page) = self.page_mut() else {
            return;
        };
        let Some(to) = from.checked_add_signed(by).filter(|to| *to < page.loaded()) else {
            return;
        };
        page.cursor = to;
        self.notice = None;
        self.request(
            ClientMessage::PlaylistMove { playlist, from, to },
            Pending::Reload(format!("moved \"{track}\"")),
        );
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
        // A merge is looking for a target: a playlist takes it instead of opening.
        if let (true, Item::Playlist(target)) = (self.merging.is_some(), &item) {
            let (id, name) = (target.id, target.name.clone());
            self.finish_merge(id, &name);
            return;
        }
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
        let Some((saved, entity)) = item.saved().zip(item.id()) else {
            self.notice = Some("a playlist isn't saved; copy it in with c instead".into());
            return;
        };
        let target = item.item_ref();
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
                    && item.id() == Some(entity)
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

    // --- outputs and settings ---

    /// Which of the setup tabs is showing (0 Outputs, 1 Settings).
    pub(crate) fn setup_tab(&self) -> Option<usize> {
        match self.tab {
            Tab::Outputs => Some(0),
            Tab::Settings => Some(1),
            _ => None,
        }
    }

    /// The rows the setup tab on screen shows.
    pub(crate) fn setup_rows(&self) -> Vec<SetupRow> {
        match self.setup_tab() {
            Some(0) => setup::output_rows(&self.sinks),
            Some(_) => setup::settings_rows(&self.services, self.settings.as_ref()),
            None => Vec::new(),
        }
    }

    /// Whether playback goes to the output (or protocol endpoint) `id`.
    pub(crate) fn is_output(&self, id: &canon_core::SinkId) -> bool {
        match &self.snapshot.sink {
            Some(selected) => selected == id,
            None => SinkInfo::is_local(id),
        }
    }

    /// Keep each setup tab's cursor on a row it can rest on.
    fn clamp_setup(&mut self) {
        for tab in 0..2 {
            let rows = match tab {
                0 => setup::output_rows(&self.sinks),
                _ => setup::settings_rows(&self.services, self.settings.as_ref()),
            };
            let cursor = &mut self.setup_cursor[tab];
            *cursor = (*cursor).min(rows.len().saturating_sub(1));
            if !rows.get(*cursor).is_some_and(SetupRow::selectable) {
                *cursor = rows.iter().position(SetupRow::selectable).unwrap_or(0);
            }
        }
    }

    fn setup_key(&mut self, key: KeyEvent) {
        let Some(tab) = self.setup_tab() else {
            return;
        };
        let rows = self.setup_rows();
        let cursor = self.setup_cursor[tab];
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.setup_cursor[tab] = step(&rows, cursor, 1),
            KeyCode::Char('k') | KeyCode::Up => self.setup_cursor[tab] = step(&rows, cursor, -1),
            KeyCode::Char('g') | KeyCode::Home => {
                self.setup_cursor[tab] = rows.iter().position(SetupRow::selectable).unwrap_or(0);
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.setup_cursor[tab] = rows.iter().rposition(SetupRow::selectable).unwrap_or(0);
            }
            KeyCode::Enter => self.setup_enter(rows.get(cursor).cloned()),
            KeyCode::Char('f') => self.toggle_mode(rows.get(cursor)),
            KeyCode::Char('J' | 'K') => {
                if let Some(SetupRow::Order(service)) = rows.get(cursor) {
                    let down = key.code == KeyCode::Char('J');
                    if self.reorder(*service, down) {
                        self.setup_cursor[tab] = step(&rows, cursor, if down { 1 } else { -1 });
                    }
                }
            }
            KeyCode::Char('X') => {
                if let Some(SetupRow::Connection(connection)) = rows.get(cursor)
                    && connection.health != Health::NeedsLogin
                {
                    let method = connection.id.clone();
                    self.request(
                        ClientMessage::Disconnect {
                            method: method.clone(),
                        },
                        Pending::Disconnect(method),
                    );
                }
            }
            _ => {}
        }
    }

    fn setup_enter(&mut self, row: Option<SetupRow>) {
        match row {
            Some(SetupRow::Output(sink)) => self.select_output(sink.id.0.clone(), &sink.name),
            Some(SetupRow::Endpoint { name, endpoint }) => {
                let via = format!("{name} over {}", setup::protocol(endpoint.kind));
                self.select_output(endpoint.id.0, &via);
            }
            Some(SetupRow::Connection(connection)) if connection.health == Health::Ok => {
                self.notice = Some(format!("{} is signed in; X signs it out", connection.id));
            }
            Some(SetupRow::Connection(connection)) => self.request(
                ClientMessage::Connect {
                    method: connection.id,
                },
                Pending::Connect,
            ),
            Some(SetupRow::Toggle(toggle, on)) => self.change_settings(
                |settings| toggle.set(settings, !on),
                format!(
                    "{} {}",
                    toggle_name(toggle),
                    if on { "turned off" } else { "turned on" }
                ),
            ),
            _ => {}
        }
    }

    /// Flip the output under the cursor between flow and standard mode.
    fn toggle_mode(&mut self, row: Option<&SetupRow>) {
        let output = match row {
            Some(SetupRow::Output(sink)) => Some(sink.clone()),
            Some(SetupRow::Endpoint { name, .. }) => {
                self.sinks.iter().find(|s| &s.name == name).cloned()
            }
            _ => None,
        };
        let Some(output) = output.filter(|o| !SinkInfo::is_local(&o.id)) else {
            return;
        };
        let next = match setup::mode(self.settings.as_ref(), &output).unwrap_or_default() {
            OutputMode::Flow => OutputMode::Standard,
            OutputMode::Standard => OutputMode::Flow,
        };
        let id = output.id.0.clone();
        self.change_settings(
            move |settings| settings.outputs.entry(id).or_default().mode = next,
            format!(
                "{} plays in {} mode from the next track",
                output.name,
                if next == OutputMode::Flow {
                    "flow"
                } else {
                    "standard"
                }
            ),
        );
    }

    /// Move `service` one place down (or up) the streaming order. Returns whether it moved.
    fn reorder(&mut self, service: canon_core::Service, down: bool) -> bool {
        let Some(order) = self.settings.as_ref().map(|s| &s.streaming.order) else {
            return false;
        };
        let Some(at) = order.iter().position(|s| *s == service) else {
            return false;
        };
        let to = if down {
            at.checked_add(1)
        } else {
            at.checked_sub(1)
        };
        let Some(to) = to.filter(|to| *to < order.len()) else {
            return false;
        };
        self.change_settings(
            move |settings| settings.streaming.order.swap(at, to),
            "streaming order changed; it applies from the next track queued".into(),
        );
        true
    }

    fn select_output(&mut self, id: String, name: &str) {
        self.notice = Some(format!("switching to {name}…"));
        self.request(
            ClientMessage::SelectSink { sink: id },
            Pending::Done(format!("playing on {name}")),
        );
    }

    /// Change the settings: read, modify, write back whole, as the protocol has it.
    fn change_settings(&mut self, change: impl FnOnce(&mut Settings), done: String) {
        let Some(mut settings) = self.settings.clone() else {
            self.notice = Some("settings haven't arrived yet".into());
            return;
        };
        change(&mut settings);
        self.request(
            ClientMessage::SetSettings { settings },
            Pending::SetSettings(done),
        );
    }

    fn login_started(&mut self, method: String, flow: LoginFlow) {
        let interval = match &flow {
            LoginFlow::DeviceCode { code } => Duration::from_secs(code.interval.max(1)),
            LoginFlow::Browser { .. } => Duration::ZERO,
        };
        let next_poll = matches!(flow, LoginFlow::DeviceCode { .. }).then(|| self.now + interval);
        // The address has to get to a browser somehow; a long one is a pain to select.
        self.clipboard = Some(login_url(&flow));
        self.login = Some(Login {
            method,
            flow,
            input: String::new(),
            next_poll,
            interval,
        });
    }

    fn login_answered(&mut self, status: LoginStatus) {
        match status {
            LoginStatus::Authorized => {
                let method = self.login.take().map(|l| l.method).unwrap_or_default();
                self.notice = Some(format!("signed in with {method}"));
                self.request(ClientMessage::Services, Pending::Services);
            }
            // Asked to poll less often.
            LoginStatus::SlowDown => {
                if let Some(login) = self.login.as_mut() {
                    login.interval += Duration::from_secs(5);
                }
            }
            LoginStatus::Pending => {}
        }
    }

    /// A key while a sign-in is showing.
    fn login_key(&mut self, key: KeyEvent) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        let browser = matches!(login.flow, LoginFlow::Browser { .. });
        match key.code {
            KeyCode::Esc => {
                self.login = None;
                self.notice = Some("sign-in cancelled".into());
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.clipboard = Some(login_url(&login.flow));
            }
            KeyCode::Char(c) if browser => login.input.push(c),
            KeyCode::Backspace if browser => {
                login.input.pop();
            }
            KeyCode::Enter if browser && !login.input.trim().is_empty() => {
                let (method, redirect) = (login.method.clone(), login.input.trim().to_owned());
                self.request(
                    ClientMessage::ConnectComplete {
                        method,
                        redirect: Some(redirect),
                    },
                    Pending::LoginPoll,
                );
            }
            _ => {}
        }
    }

    /// Open the prompt for `asking`, starting from `text` — the name a rename is changing, say,
    /// so the common case is an edit rather than a retype.
    fn ask(&mut self, asking: Asking, text: String) {
        self.notice = None;
        self.prompt = Some(Prompt { asking, text });
    }

    /// A key while the prompt is open. What enter means is whatever the prompt was opened for.
    fn prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                prompt.text.pop();
            }
            KeyCode::Char(c) => prompt.text.push(c),
            KeyCode::Enter => {
                let Some(prompt) = self.prompt.take() else {
                    return;
                };
                let text = prompt.text.trim().to_owned();
                if text.is_empty() {
                    return;
                }
                self.answered(prompt.asking, text);
            }
            _ => {}
        }
    }

    /// The prompt was answered with `text`: do what it was asked for.
    fn answered(&mut self, asking: Asking, text: String) {
        match asking {
            Asking::Search => {
                self.stacks[2].clear();
                self.push_page(2, format!("\"{text}\""), Source::Search(text));
            }
            Asking::Rename { playlist, was } => self.request(
                ClientMessage::PlaylistRename {
                    playlist,
                    name: text.clone(),
                },
                Pending::Reload(format!("renamed \"{was}\" to \"{text}\"")),
            ),
            Asking::Copy { item } => {
                let done = match &item {
                    Some(_) => format!("copied into \"{text}\""),
                    None => format!("made \"{text}\""),
                };
                self.request(
                    ClientMessage::PlaylistCreate {
                        name: text,
                        items: item.into_iter().collect(),
                    },
                    Pending::Reload(done),
                );
            }
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

/// A toggle, briefly.
fn toggle_name(toggle: Toggle) -> &'static str {
    match toggle {
        Toggle::Autoplay => "autoplay",
        Toggle::Identify => "MusicBrainz lookups",
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

/// The address a sign-in sends the user to.
pub(crate) fn login_url(flow: &LoginFlow) -> String {
    match flow {
        LoginFlow::Browser { url } => url.clone(),
        LoginFlow::DeviceCode { code } => code
            .verification_uri_complete
            .clone()
            .unwrap_or_else(|| code.verification_uri.clone()),
    }
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

/// The row `by` selectable rows from `from`, or `from` if there is none that far.
fn step(rows: &[SetupRow], from: usize, by: isize) -> usize {
    let mut at = from;
    let mut left = by.unsigned_abs();
    while left > 0 {
        let Some(next) = at.checked_add_signed(by.signum()) else {
            break;
        };
        match rows.get(next) {
            Some(row) => {
                at = next;
                if row.selectable() {
                    left -= 1;
                }
            }
            None => break,
        }
    }
    if left == 0 && rows.get(at).is_some_and(SetupRow::selectable) {
        at
    } else {
        from
    }
}
