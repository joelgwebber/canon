//! canon-protocol — the wire contract between canon's server and its clients (yak canon-e329).
//!
//! Every type that crosses the WebSocket lives here: the client and server messages
//! ([`message`]), the library views a client is shown ([`view`]), and how a client names
//! something to act on ([`item`]). Nothing here stores, resolves or plays anything, so a client
//! (the TUI, a player, a native app) links this and `canon-core`'s plain types, not the library's
//! sqlite. The JSON these types define is what a non-Rust client implements against.

pub mod item;
pub mod message;
pub mod view;

pub use item::{EntityKind, ItemRef};
pub use message::{
    ClientEnvelope, ClientMessage, PROTOCOL_VERSION, QueueAt, ReplyData, ServerMessage, ServiceView,
};
pub use view::{
    AlbumDetail, AlbumView, ArtistDetail, ArtistView, ExportReport, ImportReport, LibraryPage,
    ListedTrack, MixView, Named, PlaylistDetail, PlaylistVersion, PlaylistView, SearchView,
    ServicePlaylistView, TrackView,
};
