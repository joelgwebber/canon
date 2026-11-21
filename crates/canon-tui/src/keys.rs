//! The command table (canon-f1d2): every key of the non-modal surfaces in one place, through
//! toque's `keys` (shared with yaks). A [`Binding`] names a [`Cmd`] (a stable dotted name, which
//! is what a user's override uses), the [`Ctx`] it applies in, its keys and its line in the `?`
//! popup, so a key cannot be bound without being listed.
//!
//! Dispatch is [`App::handle_key`](crate::App::handle_key): the [`Ctx::Global`] keys first, then
//! the tab's own context, then [`App::run_cmd`](crate::App). A [`Cmd`] is one intent, not one
//! handler: `item.remove` removes the queue entry on the queue and the track from a playlist in
//! a list, and `run_cmd` picks the target from the context.
//!
//! The sign-in and the text prompt are modal and take keys of their own; they are not in the
//! table.
//!
//! The user's overrides live in `<config>/canon/tui.yaml` (`~/.config/canon/tui.yaml` on Linux,
//! `~/Library/Application Support/canon/tui.yaml` on macOS), as a `keys:` map from a command's
//! name to its keys; `keys: standard` picks the standard family (no `hjkl` or `g`/`G`).

use ratatui::crossterm::event::KeyCode::{
    BackTab, Backspace, Delete, Down, End, Enter, Esc, F, Home, Left, PageDown, PageUp, Right, Tab,
    Up,
};
use toque::keys::{Command, Context, Family, Hint, Key, c, ctrl, k};

/// One row of the table.
pub(crate) type Binding = toque::keys::Binding<Cmd>;
/// The keys in effect: the table, filtered by the key family, with the user's overrides.
pub(crate) type Keymap = toque::keys::Keymap<Cmd>;
type Section = toque::keys::Section<Cmd>;

/// A surface that takes keys. The tabs are surfaces; [`Ctx::Global`] keys work on all of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ctx {
    /// Transport, volume, views: tried before the tab's own keys.
    Global,
    Queue,
    /// A list of tracks, albums, artists or playlists: Library, Playlists, Search.
    Page,
    /// Settings.
    Setup,
    /// The outputs picker, over any view: modal, so its keys are the only ones.
    Outputs,
}

impl Context for Ctx {
    const ALL: &'static [Ctx] = &[Ctx::Global, Ctx::Queue, Ctx::Page, Ctx::Setup, Ctx::Outputs];

    fn title(self) -> &'static str {
        match self {
            Ctx::Global => "everywhere",
            Ctx::Queue => "queue",
            Ctx::Page => "list",
            Ctx::Setup => "settings",
            Ctx::Outputs => "outputs picker",
        }
    }
}

/// Declares [`Cmd`] with each variant's dotted name, keeping `ALL` and `name` in step with it.
macro_rules! commands {
    ($($variant:ident => $name:literal,)*) => {
        /// One action a key can run.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub(crate) enum Cmd {
            $($variant,)*
        }

        impl Cmd {
            const LIST: &'static [Cmd] = &[$(Cmd::$variant,)*];

            fn dotted(self) -> &'static str {
                match self {
                    $(Cmd::$variant => $name,)*
                }
            }
        }
    };
}

commands! {
    AppQuit => "app.quit",
    AppHelp => "app.help",
    ViewNext => "view.next",
    ViewPrev => "view.prev",
    ViewQueue => "view.queue",
    ViewLibrary => "view.library",
    ViewPlaylists => "view.playlists",
    ViewSearch => "view.search",
    ViewSettings => "view.settings",
    OutputsPick => "outputs.pick",
    PickerClose => "picker.close",
    SearchOpen => "search.open",
    FindNext => "find.next",
    FindPrev => "find.prev",
    NavBack => "nav.back",
    NavForward => "nav.forward",
    PlayToggle => "transport.toggle",
    TrackNext => "transport.next",
    TrackPrev => "transport.prev",
    SeekForward => "transport.seek_forward",
    SeekBack => "transport.seek_back",
    Shuffle => "transport.shuffle",
    RepeatCycle => "transport.repeat",
    VolumeUp => "volume.up",
    VolumeDown => "volume.down",
    MuteToggle => "volume.mute",
    MoveDown => "move.down",
    MoveUp => "move.up",
    MovePageDown => "move.page_down",
    MovePageUp => "move.page_up",
    MoveTop => "move.top",
    MoveBottom => "move.bottom",
    MovePlaying => "move.playing",
    ItemOpen => "item.open",
    PageBack => "page.back",
    ListingNext => "listing.next",
    ListingPrev => "listing.prev",
    Enqueue => "queue.add",
    EnqueueNext => "queue.add_next",
    PlayNow => "queue.play_now",
    SaveToggle => "item.save",
    ItemRemove => "item.remove",
    ItemMoveDown => "item.move_down",
    ItemMoveUp => "item.move_up",
    ListCopy => "list.copy",
    ListMerge => "list.merge",
    TrackAddTo => "track.add_to",
    PlaylistRename => "playlist.rename",
    PlaylistDelete => "playlist.delete",
    PlaylistNew => "playlist.new",
    OutputMode => "output.mode",
    SignOut => "service.sign_out",
}

impl Command for Cmd {
    type Ctx = Ctx;
    /// canon's rows have no kinds a binding needs to tell apart (yet).
    type Kind = ();
    const ALL: &'static [Cmd] = Cmd::LIST;
    const GROUPS: &'static [&'static str] = &["Move", "Play", "Change", "View", "App"];
    /// canon's vim-only keys: `j`/`k` and `g`/`G`. Unlike yaks, `d` removes (it is no
    /// half-page key), so it stays on in standard. `h`/`l` are bound to nothing: they never
    /// move between the list and the pane (doks/design/tui-player.md, Keys).
    const VIM_ONLY: &'static [Key] = &[c('h'), c('j'), c('k'), c('l'), c('g'), c('G')];
    const STANDARD_ONLY: &'static [Key] = &[];

    fn name(self) -> &'static str {
        self.dotted()
    }

    fn group_title(self) -> &'static str {
        use Cmd::*;
        match self {
            MoveDown | MoveUp | MovePageDown | MovePageUp | MoveTop | MoveBottom | MovePlaying
            | ItemOpen | PageBack => "Move",
            PlayToggle | TrackNext | TrackPrev | SeekForward | SeekBack | Shuffle | RepeatCycle
            | VolumeUp | VolumeDown | MuteToggle | Enqueue | EnqueueNext | PlayNow => "Play",
            SaveToggle | ItemRemove | ItemMoveDown | ItemMoveUp | ListCopy | ListMerge
            | TrackAddTo | PlaylistRename | PlaylistDelete | PlaylistNew | OutputMode | SignOut => {
                "Change"
            }
            ViewNext | ViewPrev | ViewQueue | ViewLibrary | ViewPlaylists | ViewSearch
            | ViewSettings | OutputsPick | PickerClose | SearchOpen | FindNext | FindPrev
            | NavBack | NavForward | ListingNext | ListingPrev => "View",
            AppQuit | AppHelp => "App",
        }
    }

    /// Whether it changes something that persists: the library, a playlist, a setting, a
    /// sign-in. (The queue is the session's, so editing it is not.) Exhaustive, so a new command
    /// must say.
    fn mutates(self) -> bool {
        use Cmd::*;
        match self {
            SaveToggle | ItemRemove | ItemMoveDown | ItemMoveUp | ListCopy | ListMerge
            | TrackAddTo | PlaylistRename | PlaylistDelete | PlaylistNew | OutputMode | SignOut => {
                true
            }
            AppQuit | AppHelp | ViewNext | ViewPrev | ViewQueue | ViewLibrary | ViewPlaylists
            | ViewSearch | ViewSettings | OutputsPick | PickerClose | SearchOpen | FindNext
            | FindPrev | NavBack | NavForward | PlayToggle | TrackNext | TrackPrev
            | SeekForward | SeekBack | Shuffle | RepeatCycle | VolumeUp | VolumeDown
            | MuteToggle | MoveDown | MoveUp | MovePageDown | MovePageUp | MoveTop | MoveBottom
            | MovePlaying | ItemOpen | PageBack | ListingNext | ListingPrev | Enqueue
            | EnqueueNext | PlayNow => false,
        }
    }

    fn bindings() -> &'static [Binding] {
        BINDINGS
    }
}

const fn bind(ctx: Ctx, keys: &'static [Key], cmd: Cmd, help: &'static str) -> Binding {
    toque::keys::bind(ctx, keys, cmd, help, Hint::None)
}

use Cmd::*;
use Ctx::{Global, Outputs, Page, Queue, Setup};

/// The table. Within a context, the order is the order of the `?` popup.
#[rustfmt::skip]
static BINDINGS: &[Binding] = &[
    // -- everywhere ----------------------------------------------------------------------------
    bind(Global, &[k(Tab)], ViewNext, "next view (1-4: go to one)"),
    bind(Global, &[k(BackTab)], ViewPrev, "previous view"),
    bind(Global, &[c('1')], ViewQueue, ""),
    bind(Global, &[c('2')], ViewLibrary, ""),
    bind(Global, &[c('3')], ViewPlaylists, ""),
    bind(Global, &[c('4')], ViewSearch, ""),
    bind(Global, &[c(',')], ViewSettings, "settings and sign-in"),
    bind(Global, &[ctrl('o')], OutputsPick, "choose where to play"),
    bind(Global, &[c('/')], SearchOpen, "find here (in Search: search)"),
    bind(Global, &[c('o')], NavBack, "history back"),
    bind(Global, &[c('i')], NavForward, "history forward"),
    bind(Global, &[c(' ')], PlayToggle, "play / pause"),
    bind(Global, &[c('>')], TrackNext, "next track"),
    bind(Global, &[c('<')], TrackPrev, "previous track"),
    bind(Global, &[c('.')], MovePlaying, "to what is playing"),
    bind(Global, &[k(Right)], SeekForward, "seek forward 10s"),
    bind(Global, &[k(Left)], SeekBack, "seek back 10s"),
    bind(Global, &[c('+'), c('=')], VolumeUp, "volume up"),
    bind(Global, &[c('-')], VolumeDown, "volume down"),
    bind(Global, &[c('m')], MuteToggle, "mute"),
    bind(Global, &[c('s')], Shuffle, "shuffle"),
    bind(Global, &[c('r')], RepeatCycle, "repeat: off, all, one"),
    bind(Global, &[c('?'), k(F(1))], AppHelp, "these keys"),
    bind(Global, &[c('q'), ctrl('c')], AppQuit, "quit"),

    // -- the queue -----------------------------------------------------------------------------
    bind(Queue, &[c('j'), k(Down)], MoveDown, "down"),
    bind(Queue, &[c('k'), k(Up)], MoveUp, "up"),
    bind(Queue, &[k(PageDown)], MovePageDown, "page down"),
    bind(Queue, &[k(PageUp)], MovePageUp, "page up"),
    bind(Queue, &[c('g'), k(Home)], MoveTop, "top"),
    bind(Queue, &[c('G'), k(End)], MoveBottom, "bottom"),
    bind(Queue, &[k(Enter)], ItemOpen, "play this entry"),
    bind(Queue, &[c('n')], FindNext, "next match"),
    bind(Queue, &[c('N')], FindPrev, "previous match"),
    bind(Queue, &[c('d'), c('x'), k(Delete)], ItemRemove, "remove from the queue"),
    bind(Queue, &[c('J')], ItemMoveDown, "move the entry down"),
    bind(Queue, &[c('K')], ItemMoveUp, "move the entry up"),
    bind(Queue, &[k(Esc)], PageBack, "back to Now playing"),

    // -- a list: Library, Playlists, Search ----------------------------------------------------
    bind(Page, &[c('j'), k(Down)], MoveDown, "down"),
    bind(Page, &[c('k'), k(Up)], MoveUp, "up"),
    bind(Page, &[k(PageDown)], MovePageDown, "page down"),
    bind(Page, &[k(PageUp)], MovePageUp, "page up"),
    bind(Page, &[c('g'), k(Home)], MoveTop, "top"),
    bind(Page, &[c('G'), k(End)], MoveBottom, "bottom"),
    bind(Page, &[k(Enter)], ItemOpen, "play from here / open in the pane"),
    bind(Page, &[k(Esc), k(Backspace)], PageBack, "back out; calls off a merge or delete"),
    bind(Page, &[c('n')], FindNext, "next match"),
    bind(Page, &[c('N')], FindPrev, "previous match"),
    bind(Page, &[c(']')], ListingNext, "next listing (kinds, services)"),
    bind(Page, &[c('[')], ListingPrev, "previous listing"),
    bind(Page, &[c('a')], Enqueue, "add to the queue"),
    bind(Page, &[c('A')], EnqueueNext, "play next"),
    bind(Page, &[c('P')], PlayNow, "play now"),
    bind(Page, &[c('*')], SaveToggle, "save / unsave"),
    bind(Page, &[c('c')], ListCopy, "copy this list into a new playlist"),
    bind(Page, &[c('M')], ListMerge, "merge it into a playlist…"),
    bind(Page, &[c('L')], TrackAddTo, "add this track to a playlist…"),
    bind(Page, &[c('d'), c('x'), k(Delete)], ItemRemove, "in a playlist: remove the track"),
    bind(Page, &[c('J')], ItemMoveDown, "in a playlist: move the track down"),
    bind(Page, &[c('K')], ItemMoveUp, "in a playlist: move the track up"),
    bind(Page, &[c('R')], PlaylistRename, "rename this playlist"),
    bind(Page, &[c('D')], PlaylistDelete, "delete this playlist (twice to confirm)"),
    bind(Page, &[c('C')], PlaylistNew, "new empty playlist (Playlists)"),

    // -- Settings ------------------------------------------------------------------------------
    bind(Setup, &[c('j'), k(Down)], MoveDown, "down"),
    bind(Setup, &[c('k'), k(Up)], MoveUp, "up"),
    bind(Setup, &[c('g'), k(Home)], MoveTop, "top"),
    bind(Setup, &[c('G'), k(End)], MoveBottom, "bottom"),
    bind(Setup, &[k(Enter)], ItemOpen, "sign in / toggle"),
    bind(Setup, &[c('J')], ItemMoveDown, "streaming order: move down"),
    bind(Setup, &[c('K')], ItemMoveUp, "streaming order: move up"),
    bind(Setup, &[c('X')], SignOut, "sign out"),

    // -- the outputs picker (modal) ------------------------------------------------------------
    bind(Outputs, &[c('j'), k(Down)], MoveDown, "down"),
    bind(Outputs, &[c('k'), k(Up)], MoveUp, "up"),
    bind(Outputs, &[c('g'), k(Home)], MoveTop, "top"),
    bind(Outputs, &[c('G'), k(End)], MoveBottom, "bottom"),
    bind(Outputs, &[k(Enter)], ItemOpen, "play there"),
    bind(Outputs, &[c('f')], OutputMode, "flow or standard mode"),
    bind(Outputs, &[k(Esc), c('q'), ctrl('o')], PickerClose, "close"),
    bind(Outputs, &[c('?'), k(F(1))], AppHelp, "these keys"),
    bind(Outputs, &[ctrl('c')], AppQuit, "quit"),
];

/// The `?` popup's sections: the context's own, then the keys that work everywhere.
pub(crate) fn sections(ctx: Ctx) -> [Section; 2] {
    let own = match ctx {
        Queue => (Queue, "Queue"),
        Setup => (Setup, "Settings"),
        Outputs => (Outputs, "Outputs"),
        Page | Global => (Page, "Library, Playlists, Search"),
    };
    [
        Section {
            ctx: own.0,
            kinds: &[None],
            title: own.1,
        },
        Section {
            ctx: Global,
            kinds: &[None],
            title: "Everywhere",
        },
    ]
}

/// The user's keymap, from `<config>/canon/tui.yaml`: the table's keys in the vim family when
/// there is no file. The notice says, in one line, what in the file could not be used.
pub(crate) fn load_user() -> (Keymap, Option<String>) {
    match directories::ProjectDirs::from("", "", "canon") {
        Some(dirs) => Keymap::load(&dirs.config_dir().join("tui.yaml"), Family::Vim),
        None => (Keymap::default(), None),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use toque::keys::rules;

    use super::*;

    const KINDS: &[Option<()>] = &[None];

    #[test]
    fn the_table_keeps_the_rules() {
        assert_eq!(rules::no_shared_keys::<Cmd>(KINDS), Vec::<String>::new());
        assert_eq!(
            rules::every_command_bound_and_named::<Cmd>(),
            Vec::<String>::new()
        );
        assert_eq!(
            rules::every_key_parses_from_its_label::<Cmd>(),
            Vec::<String>::new()
        );
        assert_eq!(
            rules::every_command_bound_in_both_families::<Cmd>(KINDS),
            Vec::<String>::new()
        );
    }

    /// Lowercase looks, moves and plays; Shift changes what persists. The exceptions say why
    /// (settled in the key pass, canon-2feb).
    #[test]
    fn lowercase_only_looks_bar_the_listed_exceptions() {
        let exceptions = [
            (
                SaveToggle,
                "`*` saves: a mark in the library, as yaks' star",
            ),
            (
                ListCopy,
                "`c` makes a new playlist and changes nothing that exists",
            ),
            (
                ItemRemove,
                "`d`/`x` remove, from a playlist as from the queue: one key, one intent",
            ),
            (
                OutputMode,
                "`f`, in the modal picker, changes how a speaker is driven, not saved data",
            ),
        ];
        assert_eq!(
            rules::lowercase_only_looks::<Cmd>(&exceptions),
            Vec::<String>::new()
        );
    }

    /// The global keys are tried first, so a tab's own binding on one would never run. (The
    /// picker is modal: the global keys are not tried there.)
    #[test]
    fn no_tab_binds_a_global_key() {
        let km = Keymap::default();
        for b in BINDINGS
            .iter()
            .filter(|b| b.ctx != Global && b.ctx != Outputs)
        {
            for key in b.keys {
                let ev = KeyEvent::new(key.code, key.mods);
                assert_eq!(
                    km.lookup(Global, None, &ev),
                    None,
                    "{:?}'s {} is a global key",
                    b.ctx,
                    key.label()
                );
            }
        }
    }

    #[test]
    fn standard_turns_off_hjkl_and_g_but_keeps_d() {
        let std = Keymap::new(Family::Standard);
        let key = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
        for ch in ['h', 'j', 'k', 'l', 'g', 'G'] {
            assert_eq!(std.lookup(Page, None, &key(ch)), None, "{ch}");
        }
        assert_eq!(std.lookup(Queue, None, &key('d')), Some(ItemRemove));
        assert_eq!(
            std.lookup(Page, None, &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(PageBack)
        );
    }
}
