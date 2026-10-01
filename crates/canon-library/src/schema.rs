//! The library's schema, as an ordered list of migrations.
//!
//! `PRAGMA user_version` records how many have run. Each migration runs in its own transaction
//! together with the version bump, so a crash leaves the file at a whole version. Migrations are
//! append-only: once one has shipped, a change is a new migration, never an edit.

use rusqlite::Connection;

/// Migration `n` (1-based) is `MIGRATIONS[n - 1]`.
const MIGRATIONS: &[&str] = &[
    // 1: entities, credits, tracklists, bindings, and library membership.
    r"
    CREATE TABLE artists (
        id        TEXT PRIMARY KEY,
        name      TEXT NOT NULL,
        sort_name TEXT,
        mbid      TEXT UNIQUE
    );

    CREATE TABLE albums (
        id           TEXT PRIMARY KEY,
        title        TEXT NOT NULL,
        credit       TEXT NOT NULL,
        release_date TEXT,
        barcode      TEXT,
        mbid         TEXT UNIQUE,
        group_mbid   TEXT,
        artwork_url  TEXT
    );
    CREATE INDEX albums_barcode ON albums (barcode);
    CREATE INDEX albums_group_mbid ON albums (group_mbid);

    CREATE TABLE tracks (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        credit      TEXT NOT NULL,
        duration_ms INTEGER,
        mbid        TEXT UNIQUE
    );

    CREATE TABLE track_isrcs (
        track TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
        isrc  TEXT NOT NULL,
        PRIMARY KEY (track, isrc)
    );
    CREATE INDEX track_isrcs_isrc ON track_isrcs (isrc);

    -- Who is credited on a track or an album, in order. Entity ids share one space, so one
    -- table serves both.
    CREATE TABLE credits (
        entity   TEXT NOT NULL,
        position INTEGER NOT NULL,
        artist   TEXT NOT NULL REFERENCES artists (id),
        PRIMARY KEY (entity, position)
    );
    CREATE INDEX credits_artist ON credits (artist);

    CREATE TABLE album_tracks (
        album    TEXT NOT NULL REFERENCES albums (id) ON DELETE CASCADE,
        disc     INTEGER NOT NULL,
        position INTEGER NOT NULL,
        track    TEXT NOT NULL REFERENCES tracks (id),
        PRIMARY KEY (album, disc, position)
    );
    CREATE INDEX album_tracks_track ON album_tracks (track);

    -- A service key names exactly one entity of a kind; an entity may have many.
    CREATE TABLE bindings (
        kind       TEXT NOT NULL,
        service    TEXT NOT NULL,
        key        TEXT NOT NULL,
        entity     TEXT NOT NULL,
        provenance TEXT NOT NULL,
        confidence REAL NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (kind, service, key)
    );
    CREATE INDEX bindings_entity ON bindings (entity);

    CREATE TABLE saved (
        entity   TEXT PRIMARY KEY,
        kind     TEXT NOT NULL,
        saved_at INTEGER NOT NULL
    );
    ",
    // 2: playlists, canon's own ordered track lists. Positions are dense from 0.
    r"
    CREATE TABLE playlists (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE playlist_tracks (
        playlist TEXT NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        track    TEXT NOT NULL REFERENCES tracks (id),
        PRIMARY KEY (playlist, position)
    );
    CREATE INDEX playlist_tracks_track ON playlist_tracks (track);
    ",
    // 3: services known not to have a track, so a preferred service that lacks it isn't asked
    // again on every queue. Checked again once `checked_at` is old: catalogs change.
    r"
    CREATE TABLE unmatched (
        track      TEXT NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
        service    TEXT NOT NULL,
        checked_at INTEGER NOT NULL,
        PRIMARY KEY (track, service)
    );
    ",
    // 4: entities looked up in MusicBrainz, found or not, so the identifier doesn't ask again
    // until `checked_at` is old. And ISRCs in one case, as ISO 3901 writes them: services
    // disagree (Spotify sends some in lower case), and the join must not.
    r"
    CREATE TABLE identified (
        entity     TEXT PRIMARY KEY,
        checked_at INTEGER NOT NULL
    );

    UPDATE OR IGNORE track_isrcs SET isrc = upper(isrc);
    DELETE FROM track_isrcs WHERE isrc <> upper(isrc);
    ",
    // 5: ids of tracks merged into another (one recording, held twice), so an id a client or a
    // playlist export still holds names the track it became. One hop: merges re-point aliases.
    r"
    CREATE TABLE merged (
        entity      TEXT PRIMARY KEY,
        into_entity TEXT NOT NULL
    );
    ",
    // 6: forget the no-match markers made before matching fell back to title, artist and length:
    // they only say no ISRC found the track.
    r"
    DELETE FROM unmatched;
    ",
    // 7: playlists are canon's own; import no longer binds one to a service playlist and
    // overwrites it on every re-import (canon-f917). A playlist bound from before this drops the
    // binding and becomes an ordinary local playlist -- its name and tracks are untouched.
    r"
    DELETE FROM bindings WHERE kind = 'playlist';
    ",
    // 8: the entity/service pairs an import has already offered as a favorite. `saved` says what
    // is in the library now; this says what import has already had its say about, so unsaving a
    // favorite that is still liked upstream sticks instead of being resurrected on the next
    // import (canon-ae6e). Not backfilled: nothing records which service favorited what was
    // saved before this, and the first import after it only re-saves what is saved already.
    r"
    CREATE TABLE imported_favorites (
        entity      TEXT NOT NULL,
        service     TEXT NOT NULL,
        imported_at INTEGER NOT NULL,
        PRIMARY KEY (entity, service)
    );
    ",
    // 9: a playlist's past track lists, so an edit that went wrong (a bad merge above all) can be
    // undone. One snapshot per change, numbered densely from 1; the newest is what the playlist
    // holds now. The header row carries the timestamp, so a version that empties a playlist is
    // still a version. Positions are dense from 0, as in playlist_tracks.
    r"
    CREATE TABLE playlist_versions (
        playlist   TEXT NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
        version    INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (playlist, version)
    );

    CREATE TABLE playlist_version_tracks (
        playlist TEXT NOT NULL,
        version  INTEGER NOT NULL,
        position INTEGER NOT NULL,
        track    TEXT NOT NULL REFERENCES tracks (id),
        PRIMARY KEY (playlist, version, position),
        FOREIGN KEY (playlist, version) REFERENCES playlist_versions (playlist, version)
            ON DELETE CASCADE
    );
    CREATE INDEX playlist_version_tracks_track ON playlist_version_tracks (track);
    ",
];

/// Bring `conn` up to the newest schema.
///
/// # Errors
/// A migration failed, or the file was written by a newer canon (whose schema this one can't
/// safely read or write).
pub(crate) fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version: usize = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > MIGRATIONS.len() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISMATCH),
            Some(format!(
                "library schema v{version} is newer than this canon (v{})",
                MIGRATIONS.len()
            )),
        ));
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration)?;
        tx.pragma_update(None, "user_version", index + 1)?;
        tx.commit()?;
    }
    Ok(())
}
