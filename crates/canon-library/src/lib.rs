//! canon-library — the user's library as the source of truth.
//!
//! Where canon diverges hardest from tideway: canon owns identity and organization;
//! upstream services are interchangeable sources.
//!
//! Responsibilities (see yak canon-4185 and its children):
//! * **Entity model + persistence** (canon-78ea): canon-native entities keyed to
//!   MusicBrainz recording/release MBIDs (the chosen canonical identity), persisted in
//!   sqlite — mapping a [`canon_core::EntityId`] to its MBID and source bindings.
//! * **Source bindings + resolution policy** (canon-880e): each track holds an ordered
//!   set of [`canon_core::SourceRef`] with stored match confidence + provenance; ISRC
//!   (+ MBID) is the first-class join key, fuzzy match the persisted fallback.
//! * **Local index** (canon-5cb2): index local files by content + tags (not a required
//!   service id), embedding the canon id as a durable tag so it rebinds across moves.
//! * **Import/export** (canon-65f7): resolve imported playlists into canon entities
//!   (never a new upstream playlist), and export canon playlists back out.

use canon_core as _; // implemented against next.
