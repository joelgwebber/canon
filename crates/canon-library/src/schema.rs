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
