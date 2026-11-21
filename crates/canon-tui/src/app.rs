//! The TUI's state and behaviour, with no I/O: server messages and keys go in, requests come
//! out ([`App::take_requests`]), and [`crate::render`] draws it. The live terminal and the
//! headless driver both run this one `App`, so what a script sees is what a person sees.
//!
//! Time is handed in ([`App::tick`]) rather than read, so position interpolation is testable and
//! a headless frame is a pure function of what arrived.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use canon_core::{
    EntityId, Health, LoginFlow, LoginStatus, OutputMode, PlaybackState, PlayerId, PlayerSnapshot,
    Repeat, Settings, SinkId, SinkInfo, TrackRef,
};
use canon_protocol::{
    ClientEnvelope, ClientMessage, EntityKind, ItemRef, PROTOCOL_VERSION, QueueAt, ReplyData,
    ServerMessage, ServiceView,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use toque::image::{ImageView, Picker};

use crate::browse::{Item, Listed, Page, Shelf, Source, Tab};
use crate::keys::{Cmd, Ctx, Keymap};
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

/// Which side of the body has the keys: the view's list, or the pane beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Pane,
}

/// How many locations `o` can step back through; the oldest is dropped past it.
const HISTORY_CAP: usize = 50;

/// A page as history remembers it: what to ask for to get it back, and where the cursor was.
#[derive(Debug, Clone, PartialEq)]
struct Subject {
    title: String,
    source: Source,
    cursor: usize,
}

impl Subject {
    fn of(page: &Page) -> Self {
        Self {
            title: page.title.clone(),
            source: page.source.clone(),
            cursor: page.cursor,
        }
    }
}

/// Somewhere the user has been: the view, which side had the keys, the view's list and the
/// pane's subjects. Rows are not kept, only how to read them again (as yaks re-reads a document
/// on return). The queue's and Settings' cursors are not part of it: they keep their own.
#[derive(Debug, Clone, PartialEq)]
struct Location {
    tab: Tab,
    focus: Focus,
    list: Option<Subject>,
    pane: Vec<Subject>,
}

/// The one history (canon-1c6e): the locations a jump left, and those `o` stepped back from.
/// Not persisted.
#[derive(Default)]
struct History {
    back: Vec<Location>,
    fwd: Vec<Location>,
}

impl History {
    fn push_back(&mut self, location: Location) {
        self.back.push(location);
        if self.back.len() > HISTORY_CAP {
            self.back.remove(0);
        }
    }
}

/// The size art is asked for, in pixels: one size, so the daemon keeps one file per picture, and
/// large enough for the pane over a graphics protocol (half-blocks need far less).
pub(crate) const ART_SIZE: u32 = 320;

/// An entity's album art, as far as this client knows (canon-ed4b).
enum Art {
    /// Asked for; the answer is on its way.
    Asked,
    Ready(Box<ImageView>),
    /// None to be had (no art, or the fetch failed): not asked again this session.
    Missing,
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
    /// The player this connection drives, as the server named it in `hello`.
    pub(crate) player: PlayerId,
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
    /// Where jumps have been: `o` and `i` walk it.
    history: History,
    pub(crate) tab: Tab,
    /// Each browsing view's list (Library, Playlists, Search): what it shows on the left.
    pub(crate) lists: [Option<Page>; 3],
    /// The pane's subjects (canon-e536): an album, artist or playlist opened from a list, and
    /// what was opened from it in turn, the one showing last. Empty: the pane rests on Now
    /// playing. It belongs to no view, so switching views leaves it as it is.
    pub(crate) pane: Vec<Page>,
    /// Which side has the keys.
    pub(crate) focus: Focus,
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
    /// The outputs picker's and the Settings view's cursors and scroll offsets (0 outputs,
    /// 1 settings).
    pub(crate) setup_cursor: [usize; 2],
    pub(crate) setup_scroll: [std::cell::Cell<usize>; 2],
    /// Whether the outputs picker is open over the view (canon-5cb7).
    pub(crate) picking_output: bool,
    /// A sign-in in progress.
    pub(crate) login: Option<Login>,
    next_page: u64,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    outbox: Vec<ClientEnvelope>,
    /// Text to put on the user's clipboard, which only the terminal can do.
    clipboard: Option<String>,
    quit: bool,
    /// The keys in effect: the table, the user's family and overrides.
    pub(crate) keymap: Keymap,
    /// Album art by the entity it was asked for (a track or an album), for this session.
    art: HashMap<EntityId, Art>,
    /// How pictures are drawn: half-blocks until the live terminal says it can do better.
    picker: Picker,
}

impl App {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            link: Link::Connecting,
            player: PlayerId::default(),
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
            history: History::default(),
            tab: Tab::Queue,
            lists: [None, None, None],
            pane: Vec::new(),
            focus: Focus::List,
            library_kind: EntityKind::Track,
            shelf: Shelf::Mine,
            merging: None,
            deleting: None,
            prompt: None,
            services: Vec::new(),
            settings: None,
            setup_cursor: [0, 0],
            setup_scroll: Default::default(),
            picking_output: false,
            login: None,
            next_page: 1,
            next_id: 1,
            art: HashMap::new(),
            picker: Picker::halfblocks(),
            pending: HashMap::new(),
            outbox: Vec::new(),
            clipboard: None,
            quit: false,
            keymap: Keymap::default(),
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

    /// Whether replies are still owed for requests already sent, pictures included.
    #[must_use]
    pub fn awaiting_replies(&self) -> bool {
        !self.pending.is_empty() || self.art.values().any(|art| matches!(art, Art::Asked))
    }

    /// Draw pictures with `picker`: the live terminal's graphics protocol, once it has been asked
    /// which it speaks (before any key is read).
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = picker;
    }

    /// The art the screen wants and nobody has asked for yet, each now marked asked: fetch each
    /// at [`ART_SIZE`] and hand the answer to [`App::art_arrived`]. What is wanted is what could
    /// be drawn: the playing track's and the album open in the pane.
    pub fn take_art_requests(&mut self) -> Vec<EntityId> {
        let mut wanted = Vec::new();
        if let Some(track) = &self.snapshot.track {
            wanted.push(track.id);
        }
        if let Some(Source::Album(album)) = self.pane.last().map(|page| &page.source) {
            wanted.push(*album);
        }
        wanted.retain(|entity| !self.art.contains_key(entity));
        for entity in &wanted {
            self.art.insert(*entity, Art::Asked);
        }
        wanted
    }

    /// The answer to a picture asked for: a JPEG to show, or why there is none.
    pub fn art_arrived(&mut self, entity: EntityId, picture: Result<Vec<u8>, String>) {
        let decoded = picture.and_then(|bytes| {
            image::load_from_memory(&bytes).map_err(|e| format!("not a picture: {e}"))
        });
        let art = match decoded {
            Ok(image) => Art::Ready(Box::new(ImageView::new(image, &self.picker))),
            Err(why) => {
                tracing::debug!(%entity, "no art: {why}");
                Art::Missing
            }
        };
        self.art.insert(entity, art);
    }

    /// `entity`'s picture, when it has arrived.
    pub(crate) fn art(&self, entity: EntityId) -> Option<&ImageView> {
        match self.art.get(&entity) {
            Some(Art::Ready(view)) => Some(view),
            _ => None,
        }
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
        self.output(self.snapshot.sink.as_ref()?)
    }

    /// The known output `id` names, whether it names the output or one protocol endpoint of it.
    fn output(&self, id: &SinkId) -> Option<&SinkInfo> {
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
                " focus={} pane={} rows={}{} cursor={}",
                match self.focus {
                    Focus::List => "list",
                    Focus::Pane => "pane",
                },
                self.pane.len(),
                page.loaded(),
                page.total.map(|t| format!("/{t}")).unwrap_or_default(),
                page.cursor
            )),
            // The view's own cursor; the picker's, when it is open, is `picker=` below.
            None => {
                let cursor = if self.tab == Tab::Settings {
                    self.setup_cursor[1]
                } else {
                    self.cursor
                };
                header.push_str(&format!(" cursor={cursor}"));
            }
        }
        if self.picking_output {
            header.push_str(&format!(" picker=outputs:{}", self.setup_cursor[0]));
        }
        if let Some(login) = &self.login {
            header.push_str(&format!(" login={}", login.method));
        }
        header.push_str(&format!(
            " hist={}/{} pending={}",
            self.history.back.len(),
            self.history.fwd.len(),
            self.pending.len()
        ));
        header
    }

    // --- server messages ---

    /// Fold one server message into the state.
    pub fn apply(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Hello { protocol, player } => {
                self.link = Link::Connected;
                self.player = player;
                if protocol != PROTOCOL_VERSION {
                    self.notice = Some(format!(
                        "the daemon speaks protocol {protocol}; this client, {PROTOCOL_VERSION}"
                    ));
                }
                self.request(ClientMessage::ListSinks, Pending::Sinks);
            }
            ServerMessage::Snapshot { snapshot } => self.snapshot_arrived(snapshot),
            // This client never asks to carry out playback, so it is sent no orders (canon-806d).
            ServerMessage::Executor { .. } => {}
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
                    (Some(Pending::Sinks), Some(ReplyData::Sinks { sinks })) => {
                        self.sinks = sinks;
                        // The picker opened before the outputs were read: start it on the one
                        // playing now, as it would have had they been known.
                        if self.picking_output {
                            self.cursor_on_output();
                        }
                        self.clamp_setup();
                    }
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
        let lost = snapshot
            .sink_lost
            .clone()
            .filter(|lost| self.snapshot.sink_lost.as_ref() != Some(lost));
        let follow = self.cursor == self.snapshot.queue.index;
        self.snapshot = snapshot;
        self.snapshot_at = self.now;
        if queue_moved {
            self.fetch_queue();
        }
        if output_moved && self.selected_output().is_none() && self.snapshot.sink.is_some() {
            self.request(ClientMessage::ListSinks, Pending::Sinks);
        }
        // Playback moved without anyone asking it to: say where from, and why.
        if let Some(lost) = lost {
            let name = self
                .output(&lost.sink)
                .map_or(lost.sink.0.as_str(), |sink| &sink.name);
            self.notice = Some(format!("lost {name}: {}", lost.why));
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
            if self.keymap.lookup(Ctx::Global, None, &key) == Some(Cmd::AppQuit) {
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
        if self.picking_output {
            if let Some(cmd) = self.keymap.lookup(Ctx::Outputs, None, &key) {
                self.run_cmd(Ctx::Outputs, cmd);
            }
            return;
        }
        let ctx = self.key_ctx();
        let cmd = self
            .keymap
            .lookup(Ctx::Global, None, &key)
            .map(|cmd| (Ctx::Global, cmd))
            .or_else(|| self.keymap.lookup(ctx, None, &key).map(|cmd| (ctx, cmd)));
        if let Some((ctx, cmd)) = cmd {
            self.run_cmd(ctx, cmd);
        }
        if ctx == Ctx::Page {
            self.load_more();
        }
    }

    /// Take the user's key family and overrides from their config file; anything in it that
    /// can't be used is said once, as a notice.
    pub fn load_user_keys(&mut self) {
        let (keymap, notice) = crate::keys::load_user();
        self.keymap = keymap;
        if notice.is_some() {
            self.notice = notice;
        }
    }

    /// The surface the tab on screen is, for the key table.
    pub(crate) fn key_ctx(&self) -> Ctx {
        if self.picking_output {
            return Ctx::Outputs;
        }
        match self.tab {
            Tab::Queue => Ctx::Queue,
            Tab::Settings => Ctx::Setup,
            Tab::Library | Tab::Playlists | Tab::Search => Ctx::Page,
        }
    }

    /// Run `cmd`, bound in `ctx`: the one place a key's command becomes an action.
    #[allow(clippy::too_many_lines)]
    fn run_cmd(&mut self, ctx: Ctx, cmd: Cmd) {
        use Cmd::*;
        match (ctx, cmd) {
            (_, AppQuit) => self.quit = true,
            (_, AppHelp) => self.help = true,

            // Views.
            (_, ViewNext) => self.go_to(self.tab_at(1)),
            (_, ViewPrev) => self.go_to(self.tab_at(-1)),
            (_, ViewQueue) => self.go_to(Tab::Queue),
            (_, ViewLibrary) => self.go_to(Tab::Library),
            (_, ViewPlaylists) => self.go_to(Tab::Playlists),
            (_, ViewSearch) => self.go_to(Tab::Search),
            (_, ViewSettings) => self.go_to(Tab::Settings),
            (_, NavBack) => self.travel(false),
            (_, NavForward) => self.travel(true),
            (_, OutputsPick) => self.open_outputs(),
            (_, PickerClose) => self.picking_output = false,
            (_, SearchOpen) => {
                self.go_to(Tab::Search);
                self.ask(Asking::Search, String::new());
            }

            // Transport.
            (_, PlayToggle) => self.toggle_play(),
            (_, TrackNext) => self.command(ClientMessage::Next),
            (_, TrackPrev) => self.command(ClientMessage::Previous),
            (_, SeekForward) => self.seek_by(SEEK_STEP, true),
            (_, SeekBack) => self.seek_by(SEEK_STEP, false),
            (_, VolumeUp) => self.volume_by(VOLUME_STEP),
            (_, VolumeDown) => self.volume_by(-VOLUME_STEP),
            (_, MuteToggle) => self.command(ClientMessage::SetMuted {
                muted: !self.snapshot.muted,
            }),
            (_, Shuffle) => self.command(ClientMessage::Shuffle),
            (_, RepeatCycle) => self.command(ClientMessage::Repeat {
                mode: match self.snapshot.queue.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                },
            }),

            (Ctx::Queue, cmd) => self.queue_cmd(cmd),
            (Ctx::Page, cmd) => self.page_cmd(cmd),
            (Ctx::Setup | Ctx::Outputs, cmd) => self.setup_cmd(cmd),
            (Ctx::Global, _) => {}
        }
    }

    fn queue_cmd(&mut self, cmd: Cmd) {
        use Cmd::*;
        match cmd {
            MoveDown => self.move_cursor(1),
            MoveUp => self.move_cursor(-1),
            MovePageDown => self.move_cursor(self.page_rows()),
            MovePageUp => self.move_cursor(-self.page_rows()),
            MoveTop => self.cursor = 0,
            MoveBottom => self.cursor = self.queue.len().saturating_sub(1),
            MovePlaying => self.cursor = self.snapshot.queue.index,
            ItemOpen if !self.queue.is_empty() => {
                self.command(ClientMessage::Jump { index: self.cursor });
            }
            ItemRemove if !self.queue.is_empty() => {
                self.command(ClientMessage::Remove { index: self.cursor });
            }
            ItemMoveDown => self.move_entry(1),
            ItemMoveUp => self.move_entry(-1),
            PageBack => self.pane.clear(),
            _ => {}
        }
    }

    // --- browsing ---

    /// The page with the keys: the pane's subject when the pane has the focus, else the view's
    /// list. Every action on "what is selected" acts on this one.
    pub(crate) fn page(&self) -> Option<&Page> {
        match self.focus {
            Focus::Pane if !self.pane.is_empty() => self.pane.last(),
            _ => self.list_page(),
        }
    }

    fn page_mut(&mut self) -> Option<&mut Page> {
        match self.focus {
            Focus::Pane if !self.pane.is_empty() => self.pane.last_mut(),
            _ => self.lists[self.tab.stack()?].as_mut(),
        }
    }

    /// The view's own list, whichever side has the focus.
    pub(crate) fn list_page(&self) -> Option<&Page> {
        self.lists[self.tab.stack()?].as_ref()
    }

    fn page_by_id(&mut self, id: u64) -> Option<&mut Page> {
        self.lists
            .iter_mut()
            .flatten()
            .chain(self.pane.iter_mut())
            .find(|page| page.id == id)
    }

    fn tab_at(&self, by: isize) -> Tab {
        let at = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        let count = Tab::ALL.len();
        Tab::ALL[(at + count).saturating_add_signed(by) % count]
    }

    /// Where the user is now, as history keeps it.
    fn here(&self) -> Location {
        Location {
            tab: self.tab,
            focus: self.focus,
            list: self.list_page().map(Subject::of),
            pane: self.pane.iter().map(Subject::of).collect(),
        }
    }

    /// Remember where the user is, because a jump is about to leave it: the location goes on the
    /// back side and whatever `o` had stepped back from is gone. Cursor movement, `Esc`, focus
    /// moves and the outputs picker are not jumps and never call this.
    fn jump(&mut self) {
        let here = self.here();
        self.history.push_back(here);
        self.history.fwd.clear();
    }

    /// `o` (back) or `i` (forward): move to the last location on that side, leaving this one on
    /// the other, so the two undo each other.
    fn travel(&mut self, forward: bool) {
        let target = if forward {
            self.history.fwd.pop()
        } else {
            self.history.back.pop()
        };
        let Some(target) = target else {
            self.notice = Some(if forward {
                "nothing forward of here".into()
            } else {
                "nothing to go back to".into()
            });
            return;
        };
        let here = self.here();
        if forward {
            self.history.push_back(here);
        } else {
            self.history.fwd.push(here);
        }
        self.restore(target);
    }

    /// Show `to` again, reading its pages afresh with the cursors where they were: each settles
    /// on the nearest row once the rows arrive, as after a reload. A subject that can no longer
    /// load is an error reply like any other, and says so in the notice.
    fn restore(&mut self, to: Location) {
        self.notice = None;
        let pane: Vec<Page> = to.pane.into_iter().map(|s| self.reopen(s)).collect();
        self.pane = pane;
        if let Some(stack) = to.tab.stack() {
            self.lists[stack] = match to.list {
                Some(list) => {
                    self.listing_of(&list.source);
                    Some(self.reopen(list))
                }
                None => None,
            };
        }
        self.switch_tab(to.tab);
        self.focus = if self.pane.is_empty() {
            Focus::List
        } else {
            to.focus
        };
    }

    /// A page for `subject`, its rows asked for and its cursor held for them.
    fn reopen(&mut self, subject: Subject) -> Page {
        let id = self.next_page;
        self.next_page += 1;
        let mut page = Page::new(id, subject.title, subject.source);
        page.cursor = subject.cursor;
        let request = page.reload();
        self.request(request, Pending::Page(id));
        page
    }

    /// The kind or shelf a view's own listing says it is showing, so the tab's `[` and `]` carry
    /// on from there.
    fn listing_of(&mut self, source: &Source) {
        if let Source::Library(kind) = source {
            self.library_kind = *kind;
        } else if let Some(shelf) = Shelf::ALL.iter().find(|s| s.source() == *source) {
            self.shelf = *shelf;
        }
    }

    /// A jump to `tab`: the view switch the keys make.
    fn go_to(&mut self, tab: Tab) {
        if tab != self.tab {
            self.jump();
        }
        self.switch_tab(tab);
    }

    /// Show `tab`, loading its first page the first time.
    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        // A view opens on its list; the pane keeps its subject beside it.
        self.focus = Focus::List;
        // An armed delete doesn't follow the user to another view and lie in wait there.
        self.deleting = None;
        // What Settings shows changes behind our back (logins lapse): read it afresh each visit.
        if tab == Tab::Settings {
            self.request(ClientMessage::Services, Pending::Services);
            self.request(ClientMessage::Settings, Pending::Settings);
        }
        let Some(stack) = tab.stack() else {
            return;
        };
        if self.lists[stack].is_none() {
            let (title, source) = match tab {
                Tab::Library => (
                    kind_name(self.library_kind),
                    Source::Library(self.library_kind),
                ),
                Tab::Playlists => (self.shelf.name(), self.shelf.source()),
                // Search starts empty: there is nothing to show until something is asked.
                _ => return,
            };
            let page = self.load_page(title, source);
            self.lists[stack] = Some(page);
        }
    }

    /// A new page from `source`, its rows asked for.
    fn load_page(&mut self, title: String, source: Source) -> Page {
        let id = self.next_page;
        self.next_page += 1;
        let mut page = Page::new(id, title, source);
        page.loading = true;
        let request = page.request();
        self.request(request, Pending::Page(id));
        page
    }

    /// Show `source` in the pane and give it the keys. Opened from the list it replaces whatever
    /// the pane showed; opened from the pane, it goes on top, and back comes off again.
    fn open_subject(&mut self, title: String, source: Source) {
        self.jump();
        if self.focus == Focus::List {
            self.pane.clear();
        }
        let page = self.load_page(title, source);
        self.pane.push(page);
        self.focus = Focus::Pane;
    }

    /// Back, one layer at a time (Esc comes out): in the pane, a subject opened from the pane
    /// gives way to the one it was opened from, and the first gives the keys back to the list,
    /// staying on show; from the list, the pane goes back to Now playing.
    fn back(&mut self) {
        match self.focus {
            Focus::Pane if self.pane.len() > 1 => {
                self.pane.pop();
            }
            Focus::Pane => self.focus = Focus::List,
            Focus::List => self.pane.clear(),
        }
    }

    fn page_cmd(&mut self, cmd: Cmd) {
        use Cmd::*;
        let rows = self.page_rows();
        match cmd {
            MoveDown => self.with_page(|page| page.move_by(1)),
            MoveUp => self.with_page(|page| page.move_by(-1)),
            MovePageDown => self.with_page(|page| page.move_by(rows)),
            MovePageUp => self.with_page(|page| page.move_by(-rows)),
            MoveTop => self.with_page(Page::go_top),
            MoveBottom => self.with_page(Page::go_bottom),
            ItemOpen => self.activate(),
            Enqueue => self.enqueue(QueueAt::End),
            EnqueueNext => self.enqueue(QueueAt::Next),
            PlayNow => self.enqueue(QueueAt::Now),
            SaveToggle => self.toggle_saved(),
            ListCopy => self.copy_list(),
            ListMerge => self.start_merge(),
            TrackAddTo => self.start_adding_track(),
            ItemRemove => self.remove_from_playlist(),
            ItemMoveDown => self.move_in_playlist(1),
            ItemMoveUp => self.move_in_playlist(-1),
            PlaylistRename => self.rename_playlist(),
            PlaylistDelete => self.delete_playlist(),
            PlaylistNew if self.tab == Tab::Playlists => {
                self.ask(Asking::Copy { item: None }, String::new());
            }
            // A merge or a delete still waiting on the user gives up before "back" leaves.
            PageBack if self.merging.is_some() => {
                let what = match self.merging.take() {
                    Some(Merging { track: true, .. }) => "add",
                    _ => "merge",
                };
                self.notice = Some(format!("{what} cancelled"));
            }
            PageBack if self.deleting.is_some() => {
                self.deleting = None;
                self.notice = Some("delete cancelled".into());
            }
            PageBack => self.back(),
            ListingNext | ListingPrev => self.switch_listing(cmd == ListingNext),
            _ => {}
        }
    }

    /// `[` / `]` at the root of Library or Playlists: the next or previous kind of saved item,
    /// or shelf.
    fn switch_listing(&mut self, forward: bool) {
        if self.focus == Focus::Pane {
            return;
        }
        // Another listing is another place: `o` comes back to this one, cursor and all.
        if matches!(self.tab, Tab::Library | Tab::Playlists) {
            self.jump();
        }
        if self.tab == Tab::Library {
            self.library_kind = match (self.library_kind, forward) {
                (EntityKind::Track, true) | (EntityKind::Artist, false) => EntityKind::Album,
                (EntityKind::Album, true) | (EntityKind::Track, false) => EntityKind::Artist,
                _ => EntityKind::Track,
            };
            self.lists[0] = None;
            self.switch_tab(Tab::Library);
        } else if self.tab == Tab::Playlists {
            let at = Shelf::ALL
                .iter()
                .position(|s| *s == self.shelf)
                .unwrap_or(0);
            let count = Shelf::ALL.len();
            let by = if forward { 1 } else { count - 1 };
            self.show_shelf(Shelf::ALL[(at + by) % count]);
        }
    }

    /// Show `shelf` on the Playlists tab, from the top.
    fn show_shelf(&mut self, shelf: Shelf) {
        self.shelf = shelf;
        self.lists[1] = None;
        self.switch_tab(Tab::Playlists);
    }

    /// Read the pages on screen again, in place: each with the same request it already knows
    /// how to build, its reply folding back into the page that is there, so the cursor survives
    /// it (canon-28ff). Both sides: a playlist renamed in the pane is also a row in the list.
    ///
    /// Only pages that show canon's own playlists are worth re-reading after a playlist changed;
    /// a service's listing would cost a round trip upstream to come back exactly as it was.
    fn reload_page(&mut self) {
        let list = self.tab.stack().and_then(|s| self.lists[s].as_mut());
        let pane = self.pane.last_mut();
        let requests: Vec<_> = list
            .into_iter()
            .chain(pane)
            .filter(|page| page.source.follows_playlists())
            .map(|page| (page.id, page.reload()))
            .collect();
        for (id, request) in requests {
            self.request(request, Pending::Page(id));
        }
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
        // The target is picked from canon's own playlists, in the list.
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
    fn activate(&mut self) {
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
            Some(source) => self.open_subject(item.name().to_owned(), source),
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
        for page in self.lists.iter_mut().flatten().chain(self.pane.iter_mut()) {
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

    /// Which setup list has the keys: 0, the outputs picker (over any view), or 1, the Settings
    /// view.
    pub(crate) fn setup_tab(&self) -> Option<usize> {
        if self.picking_output {
            return Some(0);
        }
        (self.tab == Tab::Settings).then_some(1)
    }

    /// Open the outputs picker over the view, on the output playing now. What it lists changes
    /// behind our back (speakers come and go), so it is read afresh each time.
    fn open_outputs(&mut self) {
        self.picking_output = true;
        self.request(ClientMessage::ListSinks, Pending::Sinks);
        self.request(ClientMessage::Settings, Pending::Settings);
        self.cursor_on_output();
    }

    /// Put the picker's cursor on the output playing now: the endpoint, when one protocol of a
    /// speaker is pinned, since that row is the one marked.
    fn cursor_on_output(&mut self) {
        let rows = setup::output_rows(&self.sinks);
        let at = rows
            .iter()
            .rposition(|row| match row {
                SetupRow::Output(sink) => self.is_output(&sink.id),
                SetupRow::Endpoint { endpoint, .. } => self.is_output(&endpoint.id),
                _ => false,
            })
            .unwrap_or(0);
        self.setup_cursor[0] = at;
        self.clamp_setup();
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
    pub(crate) fn is_output(&self, id: &SinkId) -> bool {
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

    fn setup_cmd(&mut self, cmd: Cmd) {
        use Cmd::*;
        let Some(tab) = self.setup_tab() else {
            return;
        };
        let rows = self.setup_rows();
        let cursor = self.setup_cursor[tab];
        match cmd {
            MoveDown => self.setup_cursor[tab] = step(&rows, cursor, 1),
            MoveUp => self.setup_cursor[tab] = step(&rows, cursor, -1),
            MoveTop => {
                self.setup_cursor[tab] = rows.iter().position(SetupRow::selectable).unwrap_or(0);
            }
            MoveBottom => {
                self.setup_cursor[tab] = rows.iter().rposition(SetupRow::selectable).unwrap_or(0);
            }
            ItemOpen => self.setup_enter(rows.get(cursor).cloned()),
            OutputMode => self.toggle_mode(rows.get(cursor)),
            ItemMoveDown | ItemMoveUp => {
                if let Some(SetupRow::Order(service)) = rows.get(cursor) {
                    let down = cmd == ItemMoveDown;
                    if self.reorder(*service, down) {
                        self.setup_cursor[tab] = step(&rows, cursor, if down { 1 } else { -1 });
                    }
                }
            }
            SignOut => {
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
            Some(SetupRow::Output(sink)) => {
                self.picking_output = false;
                self.select_output(sink.id.0.clone(), &sink.name);
            }
            Some(SetupRow::Endpoint { name, endpoint }) => {
                self.picking_output = false;
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
                // Results already on show are a place worth coming back to; an empty Search view
                // being filled in for the first time is not.
                if self.lists[2].is_some() {
                    self.jump();
                }
                let page = self.load_page(format!("\"{text}\""), Source::Search(text));
                self.lists[2] = Some(page);
                self.focus = Focus::List;
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
