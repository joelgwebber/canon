//! [`Store`]: the library's entities in sqlite, behind plain synchronous calls.
//!
//! Synchronous on purpose: every call is a handful of indexed queries, and keeping them plain
//! functions over one connection keeps them easy to test and to compose in a transaction. The
//! async world reaches them through [`crate::Library::run`], which runs them on the blocking pool.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use canon_core::{
    AlbumListing, EntityId, Error, Result, Service, SourceAlbum, SourceArtist, SourceRef,
    SourceTrack, TrackMeta, TrackRef,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::model::{Album, AlbumTrack, Artist, Binding, EntityKind, Playlist, Provenance, Track};
use crate::schema;
use crate::view::{
    AlbumDetail, AlbumView, ArtistView, ListedTrack, Named, PlaylistDetail, PlaylistView, TrackView,
};

/// What an identity lookup changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identified {
    /// ISRCs the track didn't have before.
    pub new_isrcs: usize,
    /// Tracks found to be the same recording, and merged into this one.
    pub merged: Vec<EntityId>,
    /// Another album already holding the release MBID found: the same edition, held twice.
    /// Albums are not merged (their tracklists would have to be reconciled), only reported.
    pub duplicate: Option<EntityId>,
}

/// The library's sqlite store.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if need be) the library at `path`, migrating it to the current schema.
    ///
    /// # Errors
    /// The file can't be opened, or its schema can't be brought up to date.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        // Write-ahead logging: readers never wait on the writer, and a crash mid-write can't
        // tear the file.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .map_err(db)?;
        Self::init(conn)
    }

    /// A private, empty library that lives as long as the store (for tests and scratch use).
    ///
    /// # Errors
    /// As [`Store::open`].
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(db)?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true).map_err(db)?;
        schema::migrate(&mut conn).map_err(db)?;
        Ok(Self { conn })
    }

    // --- artists ---

    /// Add an artist, returning its new id.
    ///
    /// # Errors
    /// The write failed, or another artist already has this MBID.
    pub fn add_artist(&mut self, artist: &Artist) -> Result<EntityId> {
        let id = EntityId::new();
        self.conn
            .execute(
                "INSERT INTO artists (id, name, sort_name, mbid) VALUES (?1, ?2, ?3, ?4)",
                params![
                    text(id),
                    artist.name,
                    artist.sort_name,
                    artist.mbid.map(|m| m.to_string())
                ],
            )
            .map_err(db)?;
        Ok(id)
    }

    /// # Errors
    /// The read failed.
    pub fn artist(&self, id: EntityId) -> Result<Option<Artist>> {
        self.conn
            .query_row(
                "SELECT name, sort_name, mbid FROM artists WHERE id = ?1",
                [text(id)],
                |row| {
                    Ok(Artist {
                        name: row.get(0)?,
                        sort_name: row.get(1)?,
                        mbid: mbid(row, 2)?,
                    })
                },
            )
            .optional()
            .map_err(db)
    }

    // --- tracks ---

    /// Add a track (a recording), returning its new id.
    ///
    /// # Errors
    /// The write failed: a credited artist doesn't exist, or another track has this MBID.
    pub fn add_track(&mut self, track: &Track) -> Result<EntityId> {
        let id = EntityId::new();
        let tx = self.conn.savepoint().map_err(db)?;
        tx.execute(
            "INSERT INTO tracks (id, title, credit, duration_ms, mbid) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                text(id),
                track.title,
                track.credit,
                millis(track.duration_ms),
                track.mbid.map(|m| m.to_string())
            ],
        )
        .map_err(db)?;
        write_track_links(&tx, id, track)?;
        tx.commit().map_err(db)?;
        Ok(id)
    }

    /// Replace everything known about track `id`.
    ///
    /// # Errors
    /// There is no such track, or the write failed.
    pub fn update_track(&mut self, id: EntityId, track: &Track) -> Result<()> {
        let tx = self.conn.savepoint().map_err(db)?;
        let changed = tx
            .execute(
                "UPDATE tracks SET title = ?2, credit = ?3, duration_ms = ?4, mbid = ?5
                 WHERE id = ?1",
                params![
                    text(id),
                    track.title,
                    track.credit,
                    millis(track.duration_ms),
                    track.mbid.map(|m| m.to_string())
                ],
            )
            .map_err(db)?;
        if changed == 0 {
            return Err(Error::NotFound(format!("track {id}")));
        }
        tx.execute("DELETE FROM track_isrcs WHERE track = ?1", [text(id)])
            .map_err(db)?;
        tx.execute("DELETE FROM credits WHERE entity = ?1", [text(id)])
            .map_err(db)?;
        write_track_links(&tx, id, track)?;
        tx.commit().map_err(db)
    }

    /// # Errors
    /// The read failed.
    pub fn track(&self, id: EntityId) -> Result<Option<Track>> {
        let Some((title, credit, duration_ms, mbid)) = self
            .conn
            .query_row(
                "SELECT title, credit, duration_ms, mbid FROM tracks WHERE id = ?1",
                [text(id)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        mbid(row, 3)?,
                    ))
                },
            )
            .optional()
            .map_err(db)?
        else {
            return Ok(None);
        };
        let isrcs = self.strings(
            "SELECT isrc FROM track_isrcs WHERE track = ?1 ORDER BY rowid",
            &text(id),
        )?;
        Ok(Some(Track {
            title,
            credit,
            artists: self.credits(id)?,
            duration_ms: duration_ms.and_then(|ms| u64::try_from(ms).ok()),
            isrcs,
            mbid,
        }))
    }

    /// Every track carrying `isrc`.
    ///
    /// # Errors
    /// The read failed.
    pub fn tracks_with_isrc(&self, isrc: &str) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT track FROM track_isrcs WHERE isrc = ?1 ORDER BY rowid",
            &isrc.to_ascii_uppercase(),
        )
    }

    /// What the player needs to play track `id`: its display metadata and every binding, most
    /// trusted first. The album shown is the first the track was placed on.
    ///
    /// # Errors
    /// The read failed.
    pub fn track_ref(&self, id: EntityId) -> Result<Option<TrackRef>> {
        let Some(track) = self.track(id)? else {
            return Ok(None);
        };
        let mut artists = Vec::with_capacity(track.artists.len());
        for artist in &track.artists {
            if let Some(artist) = self.artist(*artist)? {
                artists.push(artist.name);
            }
        }
        if artists.is_empty() && !track.credit.is_empty() {
            artists.push(track.credit.clone());
        }
        let album = self
            .conn
            .query_row(
                "SELECT a.title, a.artwork_url FROM album_tracks t JOIN albums a ON a.id = t.album
                 WHERE t.track = ?1 ORDER BY a.rowid LIMIT 1",
                [text(id)],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(db)?;
        let (album, artwork_url) = album.map_or((None, None), |(title, art)| (Some(title), art));
        Ok(Some(TrackRef {
            id,
            meta: TrackMeta {
                title: track.title,
                artists,
                album,
                duration_ms: track.duration_ms,
                artwork_url,
            },
            sources: self
                .bindings(id)?
                .into_iter()
                .map(|binding| binding.source)
                .collect(),
        }))
    }

    /// The tracks entity `id` stands for: a track itself, an album its tracklist.
    ///
    /// # Errors
    /// No such entity, or it is an artist, which is not a list of tracks.
    pub fn expand(&self, id: EntityId) -> Result<Vec<TrackRef>> {
        let tracks = match self.kind_of(id)? {
            Some(EntityKind::Track) => vec![id],
            Some(EntityKind::Album) => self.tracklist(id)?.iter().map(|t| t.track).collect(),
            Some(EntityKind::Playlist) => self.playlist_tracks(id)?,
            Some(EntityKind::Artist) => {
                return Err(Error::Unsupported(
                    "an artist is not a list of tracks: play an album, or their radio".into(),
                ));
            }
            None => return Err(Error::NotFound(format!("entity {id}"))),
        };
        let mut refs = Vec::with_capacity(tracks.len());
        for track in tracks {
            refs.extend(self.track_ref(track)?);
        }
        Ok(refs)
    }

    // --- albums ---

    /// Add an album (a release), returning its new id.
    ///
    /// # Errors
    /// The write failed: a credited artist doesn't exist, or another album has this MBID.
    pub fn add_album(&mut self, album: &Album) -> Result<EntityId> {
        let id = EntityId::new();
        let tx = self.conn.savepoint().map_err(db)?;
        tx.execute(
            "INSERT INTO albums
                 (id, title, credit, release_date, barcode, mbid, group_mbid, artwork_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                text(id),
                album.title,
                album.credit,
                album.release_date,
                album.barcode,
                album.mbid.map(|m| m.to_string()),
                album.group_mbid.map(|m| m.to_string()),
                album.artwork_url
            ],
        )
        .map_err(db)?;
        write_credits(&tx, id, &album.artists)?;
        tx.commit().map_err(db)?;
        Ok(id)
    }

    /// # Errors
    /// The read failed.
    pub fn album(&self, id: EntityId) -> Result<Option<Album>> {
        let Some(mut album) = self
            .conn
            .query_row(
                "SELECT title, credit, release_date, barcode, mbid, group_mbid, artwork_url
                 FROM albums WHERE id = ?1",
                [text(id)],
                |row| {
                    Ok(Album {
                        title: row.get(0)?,
                        credit: row.get(1)?,
                        artists: Vec::new(),
                        release_date: row.get(2)?,
                        barcode: row.get(3)?,
                        mbid: mbid(row, 4)?,
                        group_mbid: mbid(row, 5)?,
                        artwork_url: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(db)?
        else {
            return Ok(None);
        };
        album.artists = self.credits(id)?;
        Ok(Some(album))
    }

    /// Every album with this barcode (editions are distinct releases, but a barcode can be
    /// reused across territories).
    ///
    /// # Errors
    /// The read failed.
    pub fn albums_with_barcode(&self, barcode: &str) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT id FROM albums WHERE barcode = ?1 ORDER BY rowid",
            barcode,
        )
    }

    /// Replace album `album`'s tracklist.
    ///
    /// # Errors
    /// A listed track doesn't exist, two entries share a slot, or the write failed.
    pub fn set_tracklist(&mut self, album: EntityId, tracks: &[AlbumTrack]) -> Result<()> {
        let tx = self.conn.savepoint().map_err(db)?;
        tx.execute("DELETE FROM album_tracks WHERE album = ?1", [text(album)])
            .map_err(db)?;
        for entry in tracks {
            tx.execute(
                "INSERT INTO album_tracks (album, disc, position, track) VALUES (?1, ?2, ?3, ?4)",
                params![text(album), entry.disc, entry.position, text(entry.track)],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)
    }

    /// Replace everything known about album `id` (its tracklist aside).
    ///
    /// # Errors
    /// There is no such album, or the write failed.
    pub fn update_album(&mut self, id: EntityId, album: &Album) -> Result<()> {
        let tx = self.conn.savepoint().map_err(db)?;
        let changed = tx
            .execute(
                "UPDATE albums SET title = ?2, credit = ?3, release_date = ?4, barcode = ?5,
                     mbid = ?6, group_mbid = ?7, artwork_url = ?8
                 WHERE id = ?1",
                params![
                    text(id),
                    album.title,
                    album.credit,
                    album.release_date,
                    album.barcode,
                    album.mbid.map(|m| m.to_string()),
                    album.group_mbid.map(|m| m.to_string()),
                    album.artwork_url
                ],
            )
            .map_err(db)?;
        if changed == 0 {
            return Err(Error::NotFound(format!("album {id}")));
        }
        tx.execute("DELETE FROM credits WHERE entity = ?1", [text(id)])
            .map_err(db)?;
        write_credits(&tx, id, &album.artists)?;
        tx.commit().map_err(db)
    }

    /// Put `entry.track` at its slot on `album`, replacing whatever was there.
    ///
    /// # Errors
    /// The album or track doesn't exist, or the write failed.
    pub fn place(&mut self, album: EntityId, entry: AlbumTrack) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO album_tracks (album, disc, position, track)
                 VALUES (?1, ?2, ?3, ?4)",
                params![text(album), entry.disc, entry.position, text(entry.track)],
            )
            .map_err(db)?;
        Ok(())
    }

    /// Album `album`'s tracklist, in disc and position order.
    ///
    /// # Errors
    /// The read failed.
    pub fn tracklist(&self, album: EntityId) -> Result<Vec<AlbumTrack>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT disc, position, track FROM album_tracks WHERE album = ?1
                 ORDER BY disc, position",
            )
            .map_err(db)?;
        statement
            .query_map([text(album)], |row| {
                Ok(AlbumTrack {
                    disc: row.get(0)?,
                    position: row.get(1)?,
                    track: entity(row, 2)?,
                })
            })
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    /// The albums track `track` appears on, in the order they were added.
    ///
    /// # Errors
    /// The read failed.
    pub fn albums_of(&self, track: EntityId) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT t.album FROM album_tracks t JOIN albums a ON a.id = t.album
             WHERE t.track = ?1 ORDER BY a.rowid",
            &text(track),
        )
    }

    // --- identity ---

    /// The entity of `kind` with this MusicBrainz id.
    ///
    /// # Errors
    /// The read failed.
    pub fn by_mbid(&self, kind: EntityKind, mbid: Uuid) -> Result<Option<EntityId>> {
        let sql = match kind {
            EntityKind::Track => "SELECT id FROM tracks WHERE mbid = ?1",
            EntityKind::Album => "SELECT id FROM albums WHERE mbid = ?1",
            EntityKind::Artist => "SELECT id FROM artists WHERE mbid = ?1",
            EntityKind::Playlist => return Ok(None),
        };
        self.conn
            .query_row(sql, [mbid.to_string()], |row| entity(row, 0))
            .optional()
            .map_err(db)
    }

    /// Which kind of entity `id` is, if it is one.
    ///
    /// # Errors
    /// The read failed.
    pub fn kind_of(&self, id: EntityId) -> Result<Option<EntityKind>> {
        let kind: Option<String> = self
            .conn
            .query_row(
                "SELECT 'track' FROM tracks WHERE id = ?1
                 UNION ALL SELECT 'album' FROM albums WHERE id = ?1
                 UNION ALL SELECT 'artist' FROM artists WHERE id = ?1
                 UNION ALL SELECT 'playlist' FROM playlists WHERE id = ?1
                 LIMIT 1",
                [text(id)],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        Ok(kind.as_deref().and_then(EntityKind::parse))
    }

    // --- bindings ---

    /// Bind `entity` to a service's key for it.
    ///
    /// Binding again what is already bound to the same entity keeps the more certain of the two
    /// provenances (a fuzzy re-match never downgrades a manual one).
    ///
    /// # Errors
    /// The key is already bound to a *different* entity of this kind: a service key names one
    /// entity, and moving it is a correction for someone to make deliberately, not a side effect.
    pub fn bind(&mut self, kind: EntityKind, entity: EntityId, binding: &Binding) -> Result<()> {
        let (service, key) = source_key(&binding.source)?;
        let tx = self.conn.savepoint().map_err(db)?;
        let existing = tx
            .query_row(
                "SELECT entity, provenance FROM bindings
                 WHERE kind = ?1 AND service = ?2 AND key = ?3",
                params![kind.as_str(), service.as_str(), key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(db)?;
        match existing {
            Some((owner, _)) if owner != text(entity) => {
                return Err(Error::Library(format!(
                    "{service} {} {key} is already bound to {owner}",
                    kind.as_str()
                )));
            }
            Some((_, provenance))
                if Provenance::parse(&provenance).is_some_and(|p| p < binding.provenance) => {}
            Some(_) => {
                tx.execute(
                    "UPDATE bindings SET provenance = ?4, confidence = ?5
                     WHERE kind = ?1 AND service = ?2 AND key = ?3",
                    params![
                        kind.as_str(),
                        service.as_str(),
                        key,
                        binding.provenance.as_str(),
                        binding.confidence
                    ],
                )
                .map_err(db)?;
            }
            None => {
                tx.execute(
                    "INSERT INTO bindings
                         (kind, service, key, entity, provenance, confidence, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        kind.as_str(),
                        service.as_str(),
                        key,
                        text(entity),
                        binding.provenance.as_str(),
                        binding.confidence,
                        now_ms()
                    ],
                )
                .map_err(db)?;
            }
        }
        tx.commit().map_err(db)
    }

    /// Forget a binding. Returns whether there was one.
    ///
    /// # Errors
    /// The write failed.
    pub fn unbind(&mut self, kind: EntityKind, source: &SourceRef) -> Result<bool> {
        let (service, key) = source_key(source)?;
        let removed = self
            .conn
            .execute(
                "DELETE FROM bindings WHERE kind = ?1 AND service = ?2 AND key = ?3",
                params![kind.as_str(), service.as_str(), key],
            )
            .map_err(db)?;
        Ok(removed > 0)
    }

    /// The entity of `kind` a service key is bound to.
    ///
    /// # Errors
    /// The read failed.
    pub fn bound(&self, kind: EntityKind, source: &SourceRef) -> Result<Option<EntityId>> {
        let (service, key) = source_key(source)?;
        self.conn
            .query_row(
                "SELECT entity FROM bindings WHERE kind = ?1 AND service = ?2 AND key = ?3",
                params![kind.as_str(), service.as_str(), key],
                |row| entity(row, 0),
            )
            .optional()
            .map_err(db)
    }

    /// `entity`'s bindings, most trusted first: by confidence, then provenance, then age.
    ///
    /// # Errors
    /// The read failed.
    pub fn bindings(&self, entity: EntityId) -> Result<Vec<Binding>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT service, key, provenance, confidence FROM bindings WHERE entity = ?1
                 ORDER BY created_at, rowid",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([text(entity)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f32>(3)?,
                ))
            })
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        // A row this canon can't read (a service it doesn't know, written by a newer one) is
        // skipped rather than failing the whole entity.
        let mut bindings: Vec<Binding> = rows
            .into_iter()
            .filter_map(|(service, key, provenance, confidence)| {
                Some(Binding {
                    source: source_ref(&service, key)?,
                    provenance: Provenance::parse(&provenance)?,
                    confidence,
                })
            })
            .collect();
        bindings.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(a.provenance.cmp(&b.provenance))
        });
        Ok(bindings)
    }

    // --- library membership ---

    /// Add `id` to the user's library.
    ///
    /// # Errors
    /// There is no such entity, or the write failed.
    pub fn save(&mut self, id: EntityId) -> Result<()> {
        self.save_at(id, None)
    }

    /// Add `id` to the user's library as of `at` (milliseconds since the epoch; now if `None`).
    /// Something already saved keeps the date it was first saved.
    ///
    /// # Errors
    /// There is no such entity, it is a playlist, or the write failed.
    pub fn save_at(&mut self, id: EntityId, at: Option<i64>) -> Result<()> {
        let kind = self
            .kind_of(id)?
            .ok_or_else(|| Error::NotFound(format!("entity {id}")))?;
        if kind == EntityKind::Playlist {
            return Err(Error::Unsupported(
                "a playlist is always in your library".into(),
            ));
        }
        self.conn
            .execute(
                "INSERT OR IGNORE INTO saved (entity, kind, saved_at) VALUES (?1, ?2, ?3)",
                params![text(id), kind.as_str(), at.unwrap_or_else(now_ms)],
            )
            .map_err(db)?;
        Ok(())
    }

    /// Take `id` out of the user's library. Returns whether it was in it.
    ///
    /// # Errors
    /// The write failed.
    pub fn unsave(&mut self, id: EntityId) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM saved WHERE entity = ?1", [text(id)])
            .map_err(db)?;
        Ok(removed > 0)
    }

    /// A page of the saved entities of `kind`, most recently saved first, keeping those whose
    /// title, name or credit contains `query` (case-insensitively), and how many there are in all.
    ///
    /// # Errors
    /// The read failed.
    pub fn saved_page(
        &self,
        kind: EntityKind,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<EntityId>, usize)> {
        let (table, text_columns) = match kind {
            EntityKind::Track => ("tracks", "e.title || ' ' || e.credit"),
            EntityKind::Album => ("albums", "e.title || ' ' || e.credit"),
            EntityKind::Artist => ("artists", "e.name"),
            EntityKind::Playlist => ("playlists", "e.name"),
        };
        // Every playlist is the user's; the others are theirs once saved.
        let (from, order) = if kind == EntityKind::Playlist {
            (
                "FROM playlists e WHERE ?1 = ?1 AND (?2 IS NULL OR e.name LIKE ?2 ESCAPE '\\')"
                    .to_string(),
                "e.updated_at DESC, e.rowid DESC",
            )
        } else {
            (
                format!(
                    "FROM saved s JOIN {table} e ON e.id = s.entity
                     WHERE s.kind = ?1 AND (?2 IS NULL OR {text_columns} LIKE ?2 ESCAPE '\\')"
                ),
                "s.saved_at DESC, s.rowid DESC",
            )
        };
        let pattern = query.map(|q| {
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        });
        let total: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) {from}"),
                params![kind.as_str(), pattern],
                |row| row.get(0),
            )
            .map_err(db)?;
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT e.id {from} ORDER BY {order} LIMIT ?3 OFFSET ?4"
            ))
            .map_err(db)?;
        let ids = statement
            .query_map(
                params![
                    kind.as_str(),
                    pattern,
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX)
                ],
                |row| entity(row, 0),
            )
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)?;
        Ok((ids, usize::try_from(total).unwrap_or(0)))
    }

    /// The saved entities of `kind`, most recently saved first.
    ///
    /// # Errors
    /// The read failed.
    pub fn saved(&self, kind: EntityKind) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT entity FROM saved WHERE kind = ?1 ORDER BY saved_at DESC, rowid DESC",
            kind.as_str(),
        )
    }

    // --- matching ---

    /// Remember that `service` had no copy of `track` when asked just now.
    ///
    /// # Errors
    /// The write failed.
    pub fn mark_unmatched(&mut self, track: EntityId, service: Service) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO unmatched (track, service, checked_at) VALUES (?1, ?2, ?3)",
                params![text(track), service.as_str(), now_ms()],
            )
            .map_err(db)?;
        Ok(())
    }

    /// Whether `service` was found not to have `track` within the last `within_ms`.
    ///
    /// # Errors
    /// The read failed.
    pub fn unmatched_within(
        &self,
        track: EntityId,
        service: Service,
        within_ms: i64,
    ) -> Result<bool> {
        let checked: Option<i64> = self
            .conn
            .query_row(
                "SELECT checked_at FROM unmatched WHERE track = ?1 AND service = ?2",
                params![text(track), service.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        Ok(checked.is_some_and(|at| now_ms().saturating_sub(at) < within_ms))
    }

    // --- identification ---

    /// Tracks worth looking up in MusicBrainz: no MBID yet, an ISRC to look up by, and not
    /// looked up within the last `within_ms`. At most `limit`, those some service was found not
    /// to have first (learning more of their ISRCs is what gets them matched), then oldest.
    ///
    /// # Errors
    /// The read failed.
    pub fn unidentified_tracks(&self, limit: usize, within_ms: i64) -> Result<Vec<EntityId>> {
        let since = now_ms().saturating_sub(within_ms);
        let mut statement = self
            .conn
            .prepare(
                "SELECT t.id FROM tracks t
                 WHERE t.mbid IS NULL
                   AND EXISTS (SELECT 1 FROM track_isrcs i WHERE i.track = t.id)
                   AND NOT EXISTS
                       (SELECT 1 FROM identified d WHERE d.entity = t.id AND d.checked_at > ?1)
                 ORDER BY EXISTS (SELECT 1 FROM unmatched u WHERE u.track = t.id) DESC, t.rowid
                 LIMIT ?2",
            )
            .map_err(db)?;
        statement
            .query_map(params![since, limit], |row| entity(row, 0))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    /// Albums worth looking up in MusicBrainz: no MBID yet, a barcode to look up by, and not
    /// looked up within the last `within_ms`. At most `limit`, oldest first.
    ///
    /// # Errors
    /// The read failed.
    pub fn unidentified_albums(&self, limit: usize, within_ms: i64) -> Result<Vec<EntityId>> {
        let since = now_ms().saturating_sub(within_ms);
        let mut statement = self
            .conn
            .prepare(
                "SELECT a.id FROM albums a
                 WHERE a.mbid IS NULL AND a.barcode IS NOT NULL
                   AND NOT EXISTS
                       (SELECT 1 FROM identified d WHERE d.entity = a.id AND d.checked_at > ?1)
                 ORDER BY a.rowid
                 LIMIT ?2",
            )
            .map_err(db)?;
        statement
            .query_map(params![since, limit], |row| entity(row, 0))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    /// Record what MusicBrainz says about track `id`: the recording it is (`mbid`, if one was
    /// told apart) and every ISRC that recording is released under. ISRCs are only ever added.
    /// Learning one clears the track's no-match markers, since a service that lacked the track
    /// under the old ISRCs may have it under a new one. The track counts as looked up either way.
    ///
    /// Any other track holding the MBID, or one of the ISRCs, is the same recording held twice,
    /// and is merged into this one ([`Store::merge_tracks`]).
    ///
    /// # Errors
    /// There is no such track, or the write failed.
    pub fn identify_track(
        &mut self,
        id: EntityId,
        mbid: Option<Uuid>,
        isrcs: &[String],
    ) -> Result<Identified> {
        self.atomically(|store| {
            let before = store
                .track(id)?
                .ok_or_else(|| Error::NotFound(format!("track {id}")))?
                .isrcs
                .len();
            let mut twins = Vec::new();
            if let Some(mbid) = mbid {
                twins.extend(store.by_mbid(EntityKind::Track, mbid)?);
            }
            for isrc in isrcs {
                twins.extend(store.tracks_with_isrc(isrc)?);
            }
            twins.retain(|twin| *twin != id);
            twins.sort_unstable_by_key(|twin| twin.0);
            twins.dedup();
            for twin in &twins {
                store.merge_tracks(id, *twin)?;
            }
            if let Some(mbid) = mbid {
                store
                    .conn
                    .execute(
                        "UPDATE tracks SET mbid = ?2 WHERE id = ?1",
                        params![text(id), mbid.to_string()],
                    )
                    .map_err(db)?;
            }
            for isrc in isrcs {
                store
                    .conn
                    .execute(
                        "INSERT OR IGNORE INTO track_isrcs (track, isrc) VALUES (?1, ?2)",
                        params![text(id), isrc.to_ascii_uppercase()],
                    )
                    .map_err(db)?;
            }
            let after = store.track(id)?.map_or(before, |track| track.isrcs.len());
            if after > before {
                store
                    .conn
                    .execute("DELETE FROM unmatched WHERE track = ?1", [text(id)])
                    .map_err(db)?;
            }
            store.mark_identified(id)?;
            Ok(Identified {
                new_isrcs: after.saturating_sub(before),
                merged: twins,
                duplicate: None,
            })
        })
    }

    /// Record what MusicBrainz says about album `id`: the release it is and that release's
    /// group (every edition of the album), when one was told apart. A release MBID already on
    /// another album is left off this one, as with [`Store::identify_track`]; the group is
    /// shared by design. The album counts as looked up either way.
    ///
    /// # Errors
    /// There is no such album, or the write failed.
    pub fn identify_album(
        &mut self,
        id: EntityId,
        release: Option<(Uuid, Option<Uuid>)>,
    ) -> Result<Identified> {
        self.atomically(|store| {
            let mut identified = Identified::default();
            if let Some((mbid, group)) = release {
                match store.by_mbid(EntityKind::Album, mbid)? {
                    Some(other) if other != id => identified.duplicate = Some(other),
                    _ => {
                        let changed = store
                            .conn
                            .execute(
                                "UPDATE albums SET mbid = ?2, group_mbid = ?3 WHERE id = ?1",
                                params![text(id), mbid.to_string(), group.map(|g| g.to_string())],
                            )
                            .map_err(db)?;
                        if changed == 0 {
                            return Err(Error::NotFound(format!("album {id}")));
                        }
                    }
                }
            }
            store.mark_identified(id)?;
            Ok(identified)
        })
    }

    /// Give artist `id` its MusicBrainz id, unless it has one or another artist does.
    /// Returns whether it was set.
    ///
    /// # Errors
    /// The write failed.
    pub fn identify_artist(&mut self, id: EntityId, mbid: Uuid) -> Result<bool> {
        if self.by_mbid(EntityKind::Artist, mbid)?.is_some() {
            return Ok(false);
        }
        let changed = self
            .conn
            .execute(
                "UPDATE artists SET mbid = ?2 WHERE id = ?1 AND mbid IS NULL",
                params![text(id), mbid.to_string()],
            )
            .map_err(db)?;
        Ok(changed > 0)
    }

    /// Fold track `gone` into track `keep`: they are one recording, held twice. `keep` takes
    /// `gone`'s bindings, ISRCs, album slots, playlist entries and saved status (the earlier
    /// save), and its MBID and duration if it has none; its own title and credits stand. `gone`
    /// is deleted, and its id kept as an alias of `keep` ([`Store::resolve`]), so a client or a
    /// queue still holding it names the same track. Both tracks' no-match markers are dropped:
    /// what one service lacked under one track's ISRCs it may have under the other's.
    ///
    /// # Errors
    /// Either track doesn't exist, or the write failed.
    pub fn merge_tracks(&mut self, keep: EntityId, gone: EntityId) -> Result<()> {
        if keep == gone {
            return Ok(());
        }
        self.atomically(|store| {
            let kept = store
                .track(keep)?
                .ok_or_else(|| Error::NotFound(format!("track {keep}")))?;
            let merged = store
                .track(gone)?
                .ok_or_else(|| Error::NotFound(format!("track {gone}")))?;
            let (k, g) = (text(keep), text(gone));
            let conn = &store.conn;
            let moves = [
                "UPDATE bindings SET entity = ?1 WHERE entity = ?2",
                "INSERT OR IGNORE INTO track_isrcs (track, isrc)
                     SELECT ?1, isrc FROM track_isrcs WHERE track = ?2 ORDER BY rowid",
                // A slot both hold stays `keep`'s.
                "UPDATE OR IGNORE album_tracks SET track = ?1 WHERE track = ?2",
                "UPDATE playlist_tracks SET track = ?1 WHERE track = ?2",
                "INSERT INTO saved (entity, kind, saved_at)
                     SELECT ?1, kind, saved_at FROM saved WHERE entity = ?2
                     ON CONFLICT (entity) DO UPDATE SET saved_at = min(saved_at, excluded.saved_at)",
                "DELETE FROM unmatched WHERE track IN (?1, ?2)",
                "UPDATE merged SET into_entity = ?1 WHERE into_entity = ?2",
                "INSERT OR REPLACE INTO merged (entity, into_entity) VALUES (?2, ?1)",
            ];
            for sql in moves {
                conn.execute(sql, [&k, &g]).map_err(db)?;
            }
            for sql in [
                "DELETE FROM album_tracks WHERE track = ?1",
                "DELETE FROM saved WHERE entity = ?1",
                "DELETE FROM identified WHERE entity = ?1",
                "DELETE FROM credits WHERE entity = ?1",
                "DELETE FROM tracks WHERE id = ?1",
            ] {
                conn.execute(sql, [&g]).map_err(db)?;
            }
            conn.execute(
                "UPDATE tracks SET mbid = coalesce(mbid, ?2),
                                   duration_ms = coalesce(duration_ms, ?3)
                 WHERE id = ?1",
                params![
                    k,
                    merged.mbid.map(|m| m.to_string()),
                    millis(merged.duration_ms)
                ],
            )
            .map_err(db)?;
            tracing::info!(
                "merged track {gone} into {keep}: \"{}\" held twice",
                kept.title
            );
            Ok(())
        })
    }

    /// Merge every pair of tracks sharing an ISRC, the later into the earlier, as ingestion
    /// would have had it seen them together. Returns how many were merged.
    ///
    /// # Errors
    /// The store failed.
    pub fn merge_isrc_twins(&mut self) -> Result<usize> {
        let mut merged = 0;
        loop {
            let pair = self
                .conn
                .query_row(
                    "SELECT a.track, b.track FROM track_isrcs a
                     JOIN track_isrcs b ON b.isrc = a.isrc
                     JOIN tracks ta ON ta.id = a.track
                     JOIN tracks tb ON tb.id = b.track
                     WHERE ta.rowid < tb.rowid
                     LIMIT 1",
                    [],
                    |row| Ok((entity(row, 0)?, entity(row, 1)?)),
                )
                .optional()
                .map_err(db)?;
            let Some((keep, gone)) = pair else {
                return Ok(merged);
            };
            self.merge_tracks(keep, gone)?;
            merged += 1;
        }
    }

    /// The entity `id` names now: itself, or the track it was merged into.
    ///
    /// # Errors
    /// The read failed.
    pub fn resolve(&self, id: EntityId) -> Result<EntityId> {
        Ok(self
            .conn
            .query_row(
                "SELECT into_entity FROM merged WHERE entity = ?1",
                [text(id)],
                |row| entity(row, 0),
            )
            .optional()
            .map_err(db)?
            .unwrap_or(id))
    }

    fn mark_identified(&mut self, id: EntityId) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO identified (entity, checked_at) VALUES (?1, ?2)",
                params![text(id), now_ms()],
            )
            .map_err(db)?;
        Ok(())
    }

    // --- playlists ---

    /// A new playlist holding `tracks`, in order.
    ///
    /// # Errors
    /// A track doesn't exist, or the write failed.
    pub fn create_playlist(&mut self, name: &str, tracks: &[EntityId]) -> Result<EntityId> {
        let id = EntityId::new();
        let now = now_ms();
        self.atomically(|store| {
            store
                .conn
                .execute(
                    "INSERT INTO playlists (id, name, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![text(id), name, now],
                )
                .map_err(db)?;
            store.write_playlist(id, tracks)
        })?;
        Ok(id)
    }

    /// # Errors
    /// The read failed.
    pub fn playlist(&self, id: EntityId) -> Result<Option<Playlist>> {
        self.conn
            .query_row(
                "SELECT name, created_at, updated_at FROM playlists WHERE id = ?1",
                [text(id)],
                |row| {
                    Ok(Playlist {
                        name: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(db)
    }

    /// Playlist `id`'s tracks, in order.
    ///
    /// # Errors
    /// The read failed.
    pub fn playlist_tracks(&self, id: EntityId) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT track FROM playlist_tracks WHERE playlist = ?1 ORDER BY position",
            &text(id),
        )
    }

    /// # Errors
    /// There is no such playlist, or the write failed.
    pub fn rename_playlist(&mut self, id: EntityId, name: &str) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE playlists SET name = ?2, updated_at = ?3 WHERE id = ?1",
                params![text(id), name, now_ms()],
            )
            .map_err(db)?;
        if changed == 0 {
            return Err(Error::NotFound(format!("playlist {id}")));
        }
        Ok(())
    }

    /// Delete playlist `id` (never the tracks on it). Returns whether there was one.
    ///
    /// # Errors
    /// The write failed.
    pub fn delete_playlist(&mut self, id: EntityId) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM playlists WHERE id = ?1", [text(id)])
            .map_err(db)?;
        Ok(removed > 0)
    }

    /// Change playlist `id`'s tracks with `edit`, which sees them in order and may refuse.
    ///
    /// # Errors
    /// There is no such playlist, `edit` refused, a track doesn't exist, or the write failed.
    pub fn edit_playlist(
        &mut self,
        id: EntityId,
        edit: impl FnOnce(&mut Vec<EntityId>) -> Result<()>,
    ) -> Result<()> {
        if self.playlist(id)?.is_none() {
            return Err(Error::NotFound(format!("playlist {id}")));
        }
        let mut tracks = self.playlist_tracks(id)?;
        edit(&mut tracks)?;
        self.atomically(|store| {
            store
                .conn
                .execute(
                    "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
                    params![text(id), now_ms()],
                )
                .map_err(db)?;
            store.write_playlist(id, &tracks)
        })
    }

    /// Replace playlist `id`'s rows with `tracks`, positions dense from 0.
    fn write_playlist(&mut self, id: EntityId, tracks: &[EntityId]) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist = ?1",
                [text(id)],
            )
            .map_err(db)?;
        for (position, track) in tracks.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO playlist_tracks (playlist, position, track) VALUES (?1, ?2, ?3)",
                    params![text(id), position, text(*track)],
                )
                .map_err(db)?;
        }
        Ok(())
    }

    /// # Errors
    /// The read failed.
    pub fn playlist_view(&self, id: EntityId) -> Result<Option<PlaylistView>> {
        let Some(playlist) = self.playlist(id)? else {
            return Ok(None);
        };
        let tracks: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM playlist_tracks WHERE playlist = ?1",
                [text(id)],
                |row| row.get(0),
            )
            .map_err(db)?;
        Ok(Some(PlaylistView {
            id,
            name: playlist.name,
            track_count: usize::try_from(tracks).unwrap_or(0),
            updated_at: playlist.updated_at,
        }))
    }

    /// Playlist `id` with its tracks.
    ///
    /// # Errors
    /// The read failed.
    pub fn playlist_detail(&self, id: EntityId) -> Result<Option<PlaylistDetail>> {
        let Some(playlist) = self.playlist_view(id)? else {
            return Ok(None);
        };
        let mut tracks = Vec::new();
        for track in self.playlist_tracks(id)? {
            tracks.extend(self.track_view(track, None)?);
        }
        Ok(Some(PlaylistDetail { playlist, tracks }))
    }

    // --- ingestion ---

    /// The track a service's description names, creating what the library doesn't know yet.
    ///
    /// In order: a binding the library already has wins outright; failing that, a track with the
    /// same ISRC *is* this recording, and gains the binding; failing that, the track is new. Its
    /// artists and album are found by their own bindings, or created, and the track is placed on
    /// the album where the service lists it. All of it happens or none of it does.
    ///
    /// # Errors
    /// The store failed.
    pub fn ingest_track(&mut self, described: &SourceTrack) -> Result<EntityId> {
        self.atomically(|store| {
            if let Some(id) = store.bound(EntityKind::Track, &described.source)? {
                return Ok(id);
            }
            let matched = match described.isrc.as_deref() {
                Some(isrc) => store.tracks_with_isrc(isrc)?.into_iter().next(),
                None => None,
            };
            store.ingest_unbound(described, matched.map(|id| (id, Provenance::Isrc, 1.0)))
        })
    }

    /// Bind `track` to `described`, a copy of the same recording on some service found by its
    /// ISRC (provenance `isrc`), and bring in its album and artists as [`Store::ingest_track`]
    /// would. This names the track to bind rather than taking the first with that ISRC, so a
    /// library that holds the recording twice still binds the one asked about.
    ///
    /// Returns whether `track` is now bound to it: `false`, with nothing written, when
    /// `described` doesn't carry one of the track's ISRCs, or its binding already belongs to
    /// another track. A match never moves a binding, nor lands one on a different recording.
    ///
    /// # Errors
    /// There is no such track, or the store failed.
    pub fn bind_isrc_match(&mut self, track: EntityId, described: &SourceTrack) -> Result<bool> {
        self.atomically(|store| {
            let known = store
                .track(track)?
                .ok_or_else(|| Error::NotFound(format!("track {track}")))?;
            let same_recording = described.isrc.as_deref().is_some_and(|isrc| {
                known
                    .isrcs
                    .iter()
                    .any(|have| have.eq_ignore_ascii_case(isrc))
            });
            if !same_recording {
                return Ok(false);
            }
            match store.bound(EntityKind::Track, &described.source)? {
                Some(owner) if owner == track => Ok(true),
                // The copy was ingested as a track of its own: one recording, held twice.
                Some(owner) => {
                    let twin = described.isrc.as_deref().is_some_and(|isrc| {
                        store
                            .tracks_with_isrc(isrc)
                            .is_ok_and(|ids| ids.contains(&owner))
                    });
                    if twin {
                        store.merge_tracks(track, owner)?;
                    }
                    Ok(twin)
                }
                None => {
                    store.ingest_unbound(described, Some((track, Provenance::Isrc, 1.0)))?;
                    Ok(true)
                }
            }
        })
    }

    /// Bind `track` to `described`, a copy on some service judged the same recording by title,
    /// artist and length ([`crate::fuzzy`]), with provenance `fuzzy` and `confidence`, and bring
    /// in its album and artists. Unlike an ISRC match, the copy's ISRC is not taken as the
    /// track's: a judgement is not an identity.
    ///
    /// Returns whether `track` is now bound to it: `false`, with nothing written, when its
    /// binding already belongs to another track.
    ///
    /// # Errors
    /// There is no such track, or the store failed.
    pub fn bind_fuzzy_match(
        &mut self,
        track: EntityId,
        described: &SourceTrack,
        confidence: f32,
    ) -> Result<bool> {
        self.atomically(|store| {
            if store.track(track)?.is_none() {
                return Err(Error::NotFound(format!("track {track}")));
            }
            match store.bound(EntityKind::Track, &described.source)? {
                Some(owner) => Ok(owner == track),
                None => {
                    store
                        .ingest_unbound(described, Some((track, Provenance::Fuzzy, confidence)))?;
                    Ok(true)
                }
            }
        })
    }

    /// Ingest a track whose binding the library doesn't have yet: onto `matched` by ISRC, or as
    /// a new track. Callers run it atomically.
    fn ingest_unbound(
        &mut self,
        described: &SourceTrack,
        matched: Option<(EntityId, Provenance, f32)>,
    ) -> Result<EntityId> {
        let store = self;
        let artists = store.ingest_artists(&described.artists)?;
        let (track, binding) = match matched {
            Some((track, provenance, confidence)) => (
                track,
                Binding {
                    source: described.source.clone(),
                    provenance,
                    confidence,
                },
            ),
            None => {
                let track = store.add_track(&Track {
                    title: described.title.clone(),
                    credit: credit(&described.artists),
                    artists,
                    duration_ms: described.duration_ms,
                    isrcs: described.isrc.iter().cloned().collect(),
                    mbid: None,
                })?;
                (track, Binding::direct(described.source.clone()))
            }
        };
        store.bind(EntityKind::Track, track, &binding)?;

        if let Some(album) = &described.album {
            let album = store.ingest_album(album)?;
            if let (Some(disc), Some(position)) = (described.disc, described.position) {
                store.place(
                    album,
                    AlbumTrack {
                        disc,
                        position,
                        track,
                    },
                )?;
            }
        }
        Ok(track)
    }

    /// The album a service's listing names, created if the library doesn't know it.
    ///
    /// # Errors
    /// The store failed.
    pub fn ingest_album(&mut self, described: &SourceAlbum) -> Result<EntityId> {
        self.atomically(|store| {
            if let Some(source) = &described.source
                && let Some(id) = store.bound(EntityKind::Album, source)?
            {
                store.merge_album(id, described)?;
                return Ok(id);
            }
            let artists = store.ingest_artists(&described.artists)?;
            let album = store.add_album(&Album {
                title: described.title.clone(),
                credit: credit(&described.artists),
                artists,
                release_date: described.release_date.clone(),
                barcode: described.barcode.clone(),
                mbid: None,
                group_mbid: None,
                artwork_url: described.artwork_url.clone(),
            })?;
            if let Some(source) = &described.source {
                store.bind(EntityKind::Album, album, &Binding::direct(source.clone()))?;
            }
            Ok(album)
        })
    }

    /// A known album, told more about by a service: what the description has fills in or
    /// replaces what the library had; what it lacks (a track's abbreviated album has no credits,
    /// date or barcode) is kept.
    fn merge_album(&mut self, id: EntityId, described: &SourceAlbum) -> Result<()> {
        let Some(mut album) = self.album(id)? else {
            return Err(Error::NotFound(format!("album {id}")));
        };
        let before = album.clone();
        if !described.title.is_empty() {
            album.title.clone_from(&described.title);
        }
        if !described.artists.is_empty() {
            album.artists = self.ingest_artists(&described.artists)?;
            album.credit = credit(&described.artists);
        }
        for (field, value) in [
            (&mut album.release_date, &described.release_date),
            (&mut album.barcode, &described.barcode),
            (&mut album.artwork_url, &described.artwork_url),
        ] {
            if value.is_some() {
                field.clone_from(value);
            }
        }
        if album != before {
            self.update_album(id, &album)?;
        }
        Ok(())
    }

    /// An album with its whole tracklist, as a service lists it. The tracklist replaces whatever
    /// the library had pieced together from single tracks.
    ///
    /// # Errors
    /// The store failed.
    pub fn ingest_album_listing(&mut self, listing: &AlbumListing) -> Result<EntityId> {
        self.atomically(|store| {
            let album = store.ingest_album(&listing.album)?;
            let mut tracklist = Vec::with_capacity(listing.tracks.len());
            for (index, described) in listing.tracks.iter().enumerate() {
                let track = store.ingest_track(described)?;
                let fallback = u32::try_from(index + 1).unwrap_or(u32::MAX);
                tracklist.push(AlbumTrack {
                    disc: described.disc.unwrap_or(1),
                    position: described.position.unwrap_or(fallback),
                    track,
                });
            }
            store.set_tracklist(album, &tracklist)?;
            Ok(album)
        })
    }

    /// The artist a service credits, found by its binding or created. An artist the service
    /// gives no id for can't be told apart from a namesake, so it is always new.
    ///
    /// # Errors
    /// The store failed.
    pub fn ingest_artist(&mut self, described: &SourceArtist) -> Result<EntityId> {
        if let Some(source) = &described.source
            && let Some(id) = self.bound(EntityKind::Artist, source)?
        {
            return Ok(id);
        }
        let id = self.add_artist(&Artist {
            name: described.name.clone(),
            sort_name: None,
            mbid: None,
        })?;
        if let Some(source) = &described.source {
            self.bind(EntityKind::Artist, id, &Binding::direct(source.clone()))?;
        }
        Ok(id)
    }

    fn ingest_artists(&mut self, described: &[SourceArtist]) -> Result<Vec<EntityId>> {
        described
            .iter()
            .map(|artist| self.ingest_artist(artist))
            .collect()
    }

    // --- views ---

    /// Whether `id` is in the user's library.
    ///
    /// # Errors
    /// The read failed.
    pub fn is_saved(&self, id: EntityId) -> Result<bool> {
        self.conn
            .query_row("SELECT 1 FROM saved WHERE entity = ?1", [text(id)], |_| {
                Ok(())
            })
            .optional()
            .map(|found| found.is_some())
            .map_err(db)
    }

    fn named_artists(&self, ids: &[EntityId]) -> Result<Vec<Named>> {
        let mut named = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(artist) = self.artist(*id)? {
                named.push(Named {
                    id: *id,
                    name: artist.name,
                });
            }
        }
        Ok(named)
    }

    fn sources(&self, id: EntityId) -> Result<Vec<SourceRef>> {
        Ok(self
            .bindings(id)?
            .into_iter()
            .map(|binding| binding.source)
            .collect())
    }

    /// Track `id` for display, shown on album `on` if given, else the first it was placed on.
    ///
    /// # Errors
    /// The read failed.
    pub fn track_view(&self, id: EntityId, on: Option<EntityId>) -> Result<Option<TrackView>> {
        let Some(track) = self.track(id)? else {
            return Ok(None);
        };
        let album_id = match on {
            Some(album) => Some(album),
            None => self.albums_of(id)?.into_iter().next(),
        };
        let album = match album_id {
            Some(album_id) => self.album(album_id)?.map(|album| (album_id, album)),
            None => None,
        };
        Ok(Some(TrackView {
            id,
            title: track.title,
            artists: self.named_artists(&track.artists)?,
            artwork_url: album.as_ref().and_then(|(_, a)| a.artwork_url.clone()),
            album: album.map(|(id, album)| Named {
                id,
                name: album.title,
            }),
            duration_ms: track.duration_ms,
            saved: self.is_saved(id)?,
            sources: self.sources(id)?,
            plays_from: None,
        }))
    }

    /// # Errors
    /// The read failed.
    pub fn album_view(&self, id: EntityId) -> Result<Option<AlbumView>> {
        let Some(album) = self.album(id)? else {
            return Ok(None);
        };
        Ok(Some(AlbumView {
            id,
            title: album.title,
            credit: album.credit,
            artists: self.named_artists(&album.artists)?,
            release_date: album.release_date,
            artwork_url: album.artwork_url,
            saved: self.is_saved(id)?,
            sources: self.sources(id)?,
        }))
    }

    /// # Errors
    /// The read failed.
    pub fn artist_view(&self, id: EntityId) -> Result<Option<ArtistView>> {
        let Some(artist) = self.artist(id)? else {
            return Ok(None);
        };
        Ok(Some(ArtistView {
            id,
            name: artist.name,
            saved: self.is_saved(id)?,
            sources: self.sources(id)?,
        }))
    }

    /// Album `id` with its tracklist as the library has it.
    ///
    /// # Errors
    /// The read failed.
    pub fn album_detail(&self, id: EntityId) -> Result<Option<AlbumDetail>> {
        let Some(album) = self.album_view(id)? else {
            return Ok(None);
        };
        let mut tracks = Vec::new();
        for entry in self.tracklist(id)? {
            if let Some(track) = self.track_view(entry.track, Some(id))? {
                tracks.push(ListedTrack {
                    disc: entry.disc,
                    position: entry.position,
                    track,
                });
            }
        }
        Ok(Some(AlbumDetail { album, tracks }))
    }

    /// The albums credited to `artist`, newest first.
    ///
    /// # Errors
    /// The read failed.
    pub fn albums_by(&self, artist: EntityId) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT a.id FROM credits c JOIN albums a ON a.id = c.entity WHERE c.artist = ?1
             ORDER BY a.release_date DESC, a.rowid",
            &text(artist),
        )
    }

    /// Run `f` so that all of its writes land, or none do.
    ///
    /// # Errors
    /// Whatever `f` returns (having rolled its writes back), or the store failed.
    pub fn atomically<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.conn
            .execute_batch("SAVEPOINT atomically")
            .map_err(db)?;
        match f(self) {
            Ok(value) => {
                self.conn.execute_batch("RELEASE atomically").map_err(db)?;
                Ok(value)
            }
            Err(e) => {
                // Best effort: the error being returned is the one that matters.
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO atomically; RELEASE atomically");
                Err(e)
            }
        }
    }

    // --- helpers ---

    fn credits(&self, entity: EntityId) -> Result<Vec<EntityId>> {
        self.ids(
            "SELECT artist FROM credits WHERE entity = ?1 ORDER BY position",
            &text(entity),
        )
    }

    fn ids(&self, sql: &str, param: &str) -> Result<Vec<EntityId>> {
        let mut statement = self.conn.prepare(sql).map_err(db)?;
        statement
            .query_map([param], |row| entity(row, 0))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }

    fn strings(&self, sql: &str, param: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(sql).map_err(db)?;
        statement
            .query_map([param], |row| row.get(0))
            .map_err(db)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db)
    }
}

fn write_track_links(tx: &Connection, id: EntityId, track: &Track) -> Result<()> {
    for isrc in &track.isrcs {
        tx.execute(
            "INSERT OR IGNORE INTO track_isrcs (track, isrc) VALUES (?1, ?2)",
            params![text(id), isrc.to_ascii_uppercase()],
        )
        .map_err(db)?;
    }
    write_credits(tx, id, &track.artists)
}

fn write_credits(tx: &Connection, entity: EntityId, artists: &[EntityId]) -> Result<()> {
    for (position, artist) in artists.iter().enumerate() {
        tx.execute(
            "INSERT INTO credits (entity, position, artist) VALUES (?1, ?2, ?3)",
            params![text(entity), position, text(*artist)],
        )
        .map_err(db)?;
    }
    Ok(())
}

/// The credit as displayed, from a service's list of artists.
fn credit(artists: &[SourceArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A binding's service and its key there. Paths are keys too, so they must be text.
fn source_key(source: &SourceRef) -> Result<(Service, String)> {
    match source {
        SourceRef::Tidal { id } | SourceRef::Spotify { id } => Ok((source.service(), id.clone())),
        SourceRef::Local { path } => path
            .to_str()
            .map(|path| (Service::Local, path.to_owned()))
            .ok_or_else(|| Error::Library(format!("not a UTF-8 path: {}", path.display()))),
    }
}

fn source_ref(service: &str, key: String) -> Option<SourceRef> {
    match service {
        "tidal" => Some(SourceRef::Tidal { id: key }),
        "spotify" => Some(SourceRef::Spotify { id: key }),
        "local" => Some(SourceRef::Local { path: key.into() }),
        _ => None,
    }
}

fn text(id: EntityId) -> String {
    id.0.to_string()
}

fn entity(row: &Row<'_>, index: usize) -> rusqlite::Result<EntityId> {
    let value: String = row.get(index)?;
    Uuid::parse_str(&value).map(EntityId).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn mbid(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<Uuid>> {
    let Some(value) = row.get::<_, Option<String>>(index)? else {
        return Ok(None);
    };
    Uuid::parse_str(&value).map(Some).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn millis(duration_ms: Option<u64>) -> Option<i64> {
    duration_ms.and_then(|ms| i64::try_from(ms).ok())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn db(e: rusqlite::Error) -> Error {
    Error::Library(e.to_string())
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn artist(store: &mut Store, name: &str) -> EntityId {
        store
            .add_artist(&Artist {
                name: name.into(),
                sort_name: None,
                mbid: None,
            })
            .unwrap()
    }

    fn track(title: &str, artists: Vec<EntityId>, isrc: &str) -> Track {
        Track {
            title: title.into(),
            credit: "Pink Floyd".into(),
            artists,
            duration_ms: Some(230_000),
            isrcs: vec![isrc.into()],
            mbid: None,
        }
    }

    fn album(title: &str, artists: Vec<EntityId>) -> Album {
        Album {
            title: title.into(),
            credit: "Pink Floyd".into(),
            artists,
            release_date: Some("1973-03-01".into()),
            barcode: Some("5099902987613".into()),
            mbid: None,
            group_mbid: None,
            artwork_url: Some("https://example/cover.jpg".into()),
        }
    }

    fn tidal(id: &str) -> SourceRef {
        SourceRef::Tidal { id: id.into() }
    }

    #[test]
    fn entities_round_trip_with_their_credits_and_isrcs() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let mbid = Uuid::parse_str("83d91898-7763-47d7-b03b-b92132375c47").unwrap();
        let waters = store
            .add_artist(&Artist {
                name: "Roger Waters".into(),
                sort_name: Some("Waters, Roger".into()),
                mbid: Some(mbid),
            })
            .unwrap();

        let brain_damage = track("Brain Damage", vec![floyd, waters], "GBN9Y1100088");
        let id = store.add_track(&brain_damage).unwrap();
        assert_eq!(store.track(id).unwrap().unwrap(), brain_damage);
        assert_eq!(
            store.by_mbid(EntityKind::Artist, mbid).unwrap(),
            Some(waters)
        );
        assert_eq!(store.tracks_with_isrc("GBN9Y1100088").unwrap(), vec![id]);

        let dsotm = album("The Dark Side of the Moon", vec![floyd]);
        let album_id = store.add_album(&dsotm).unwrap();
        assert_eq!(store.album(album_id).unwrap().unwrap(), dsotm);
        assert_eq!(
            store.albums_with_barcode("5099902987613").unwrap(),
            vec![album_id]
        );

        assert_eq!(store.kind_of(id).unwrap(), Some(EntityKind::Track));
        assert_eq!(store.kind_of(album_id).unwrap(), Some(EntityKind::Album));
        assert_eq!(store.kind_of(floyd).unwrap(), Some(EntityKind::Artist));
        assert_eq!(store.kind_of(EntityId::new()).unwrap(), None);
    }

    #[test]
    fn an_update_replaces_isrcs_and_credits() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let gilmour = artist(&mut store, "David Gilmour");
        let id = store
            .add_track(&track("Time", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let mut revised = track("Time", vec![gilmour, floyd], "GBN9Y1100099");
        revised.isrcs.push("GBAYE7300004".into());
        store.update_track(id, &revised).unwrap();
        assert_eq!(store.track(id).unwrap().unwrap(), revised);
        assert!(store.tracks_with_isrc("GBN9Y1100085").unwrap().is_empty());
        assert!(matches!(
            store.update_track(EntityId::new(), &revised),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn a_credit_must_name_a_real_artist() {
        let mut store = store();
        assert!(
            store
                .add_track(&track("Time", vec![EntityId::new()], "X"))
                .is_err()
        );
        assert!(
            store.tracks_with_isrc("X").unwrap().is_empty(),
            "the failed insert left nothing behind"
        );
    }

    /// One recording on two releases is one track, listed on both.
    #[test]
    fn a_recording_appears_on_many_albums() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let money = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100086"))
            .unwrap();
        let time = store
            .add_track(&track("Time", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let dsotm = store
            .add_album(&album("The Dark Side of the Moon", vec![floyd]))
            .unwrap();
        let echoes = store
            .add_album(&album("Echoes: The Best of Pink Floyd", vec![floyd]))
            .unwrap();

        store
            .set_tracklist(
                dsotm,
                &[
                    AlbumTrack {
                        disc: 1,
                        position: 6,
                        track: money,
                    },
                    AlbumTrack {
                        disc: 1,
                        position: 4,
                        track: time,
                    },
                ],
            )
            .unwrap();
        store
            .set_tracklist(
                echoes,
                &[AlbumTrack {
                    disc: 2,
                    position: 3,
                    track: money,
                }],
            )
            .unwrap();

        let listed: Vec<_> = store
            .tracklist(dsotm)
            .unwrap()
            .iter()
            .map(|t| t.track)
            .collect();
        assert_eq!(listed, vec![time, money], "in position order");
        assert_eq!(store.albums_of(money).unwrap(), vec![dsotm, echoes]);
        let shown = store.track_ref(money).unwrap().unwrap();
        assert_eq!(
            shown.meta.album.as_deref(),
            Some("The Dark Side of the Moon")
        );
    }

    #[test]
    fn a_binding_names_one_entity() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let money = store.add_track(&track("Money", vec![floyd], "A")).unwrap();
        let time = store.add_track(&track("Time", vec![floyd], "B")).unwrap();

        store
            .bind(
                EntityKind::Track,
                money,
                &Binding::direct(tidal("55391792")),
            )
            .unwrap();
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391792")).unwrap(),
            Some(money)
        );
        // The same id as an album is a different key.
        assert_eq!(
            store.bound(EntityKind::Album, &tidal("55391792")).unwrap(),
            None
        );

        let error = store
            .bind(EntityKind::Track, time, &Binding::direct(tidal("55391792")))
            .expect_err("already bound elsewhere");
        assert!(error.to_string().contains("already bound"), "{error}");

        assert!(store.unbind(EntityKind::Track, &tidal("55391792")).unwrap());
        store
            .bind(EntityKind::Track, time, &Binding::direct(tidal("55391792")))
            .unwrap();
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391792")).unwrap(),
            Some(time)
        );
    }

    #[test]
    fn a_weaker_rebinding_never_downgrades_a_stronger_one() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let money = store.add_track(&track("Money", vec![floyd], "A")).unwrap();
        let manual = Binding {
            source: tidal("1"),
            provenance: Provenance::Manual,
            confidence: 1.0,
        };
        store.bind(EntityKind::Track, money, &manual).unwrap();
        store
            .bind(
                EntityKind::Track,
                money,
                &Binding {
                    source: tidal("1"),
                    provenance: Provenance::Fuzzy,
                    confidence: 0.7,
                },
            )
            .unwrap();
        assert_eq!(store.bindings(money).unwrap(), vec![manual]);

        let fuzzy = Binding {
            source: tidal("2"),
            provenance: Provenance::Fuzzy,
            confidence: 0.7,
        };
        store.bind(EntityKind::Track, money, &fuzzy).unwrap();
        let upgraded = Binding {
            source: tidal("2"),
            provenance: Provenance::Isrc,
            confidence: 1.0,
        };
        store.bind(EntityKind::Track, money, &upgraded).unwrap();
        assert_eq!(store.bindings(money).unwrap()[1], upgraded);
    }

    #[test]
    fn a_track_ref_lists_bindings_most_trusted_first() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let money = store.add_track(&track("Money", vec![floyd], "A")).unwrap();
        let local = SourceRef::Local {
            path: "/music/Pink Floyd/Money.flac".into(),
        };
        store
            .bind(
                EntityKind::Track,
                money,
                &Binding {
                    source: tidal("2"),
                    provenance: Provenance::Fuzzy,
                    confidence: 0.8,
                },
            )
            .unwrap();
        store
            .bind(EntityKind::Track, money, &Binding::direct(local.clone()))
            .unwrap();
        store
            .bind(
                EntityKind::Track,
                money,
                &Binding {
                    source: tidal("1"),
                    provenance: Provenance::Isrc,
                    confidence: 1.0,
                },
            )
            .unwrap();

        let shown = store.track_ref(money).unwrap().unwrap();
        assert_eq!(shown.id, money);
        assert_eq!(shown.meta.title, "Money");
        assert_eq!(shown.meta.artists, vec!["Pink Floyd".to_string()]);
        assert_eq!(shown.meta.duration_ms, Some(230_000));
        assert_eq!(shown.meta.album, None, "on no album yet");
        assert_eq!(shown.sources, vec![local, tidal("1"), tidal("2")]);
        assert_eq!(store.track_ref(EntityId::new()).unwrap(), None);
    }

    #[test]
    fn saved_entities_list_newest_first_by_kind() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let money = store.add_track(&track("Money", vec![floyd], "A")).unwrap();
        let time = store.add_track(&track("Time", vec![floyd], "B")).unwrap();
        let dsotm = store
            .add_album(&album("The Dark Side of the Moon", vec![floyd]))
            .unwrap();

        store.save(money).unwrap();
        store.save(dsotm).unwrap();
        store.save(time).unwrap();
        store.save(money).unwrap(); // already saved: keeps its place
        assert_eq!(store.saved(EntityKind::Track).unwrap(), vec![time, money]);
        assert_eq!(store.saved(EntityKind::Album).unwrap(), vec![dsotm]);

        assert!(store.unsave(time).unwrap());
        assert!(!store.unsave(time).unwrap());
        assert_eq!(store.saved(EntityKind::Track).unwrap(), vec![money]);
        assert!(matches!(
            store.save(EntityId::new()),
            Err(Error::NotFound(_))
        ));
    }

    fn described(id: &str, title: &str, isrc: Option<&str>, position: u32) -> SourceTrack {
        SourceTrack {
            source: tidal(id),
            title: title.into(),
            artists: vec![SourceArtist {
                source: Some(tidal("9706")),
                name: "Pink Floyd".into(),
            }],
            album: Some(SourceAlbum {
                source: Some(tidal("55391786")),
                title: "The Dark Side of the Moon".into(),
                artwork_url: Some("https://example/cover.jpg".into()),
                ..SourceAlbum::default()
            }),
            disc: Some(1),
            position: Some(position),
            duration_ms: Some(382_000),
            isrc: isrc.map(str::to_owned),
        }
    }

    #[test]
    fn ingesting_a_binding_twice_is_one_track() {
        let mut store = store();
        let money = described("55391792", "Money", Some("GBN9Y1100086"), 6);
        let id = store.ingest_track(&money).unwrap();
        assert_eq!(store.ingest_track(&money).unwrap(), id);

        let shown = store.track_ref(id).unwrap().unwrap();
        assert_eq!(shown.meta.title, "Money");
        assert_eq!(shown.meta.artists, vec!["Pink Floyd".to_string()]);
        assert_eq!(
            shown.meta.album.as_deref(),
            Some("The Dark Side of the Moon")
        );
        assert_eq!(
            shown.meta.artwork_url.as_deref(),
            Some("https://example/cover.jpg")
        );
        assert_eq!(shown.sources, vec![tidal("55391792")]);
    }

    /// Two tracks off one album share the album and the artist, and fill in its tracklist.
    #[test]
    fn tracks_off_one_album_share_it_and_its_artist() {
        let mut store = store();
        let money = store
            .ingest_track(&described("55391792", "Money", None, 6))
            .unwrap();
        let time = store
            .ingest_track(&described("55391790", "Time", None, 4))
            .unwrap();
        assert_ne!(money, time);

        let album = store
            .bound(EntityKind::Album, &tidal("55391786"))
            .unwrap()
            .expect("album bound");
        let listed: Vec<_> = store
            .tracklist(album)
            .unwrap()
            .iter()
            .map(|t| (t.position, t.track))
            .collect();
        assert_eq!(listed, vec![(4, time), (6, money)]);
        let floyd = store.bound(EntityKind::Artist, &tidal("9706")).unwrap();
        assert_eq!(
            store.track(money).unwrap().unwrap().artists,
            vec![floyd.unwrap()]
        );
        // A track listing doesn't say who the album is by, and guessing from the track would be
        // wrong for every compilation.
        assert!(store.album(album).unwrap().unwrap().artists.is_empty());
    }

    /// The same recording under another id (a compilation, a reissue) is found by its ISRC and
    /// gains the new binding instead of becoming a second track.
    #[test]
    fn the_same_isrc_under_another_id_is_the_same_recording() {
        let mut store = store();
        let original = store
            .ingest_track(&described("55391792", "Money", Some("GBN9Y1100086"), 6))
            .unwrap();
        let mut on_a_compilation = described("77000001", "Money", Some("GBN9Y1100086"), 3);
        on_a_compilation.album = Some(SourceAlbum {
            source: Some(tidal("77000000")),
            title: "Echoes: The Best of Pink Floyd".into(),
            ..SourceAlbum::default()
        });
        let matched = store.ingest_track(&on_a_compilation).unwrap();
        assert_eq!(matched, original);

        let bindings = store.bindings(original).unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[1].provenance, Provenance::Isrc);
        assert_eq!(store.albums_of(original).unwrap().len(), 2);
    }

    /// An ISRC match binds the track it names, even when another holds the same ISRC, and never
    /// binds a copy of a different recording or takes a binding another track owns.
    #[test]
    fn an_isrc_match_binds_only_the_named_recording() {
        let mut store = store();
        let mut from_spotify = described("x", "Money", Some("GBN9Y1100081"), 6);
        from_spotify.source = SourceRef::Spotify { id: "4KW1".into() };
        from_spotify.album = None;
        let first = store.ingest_track(&from_spotify).unwrap();
        // A second library track carrying the same ISRC (added by hand, not ingested).
        let twin = store
            .add_track(&track("Money", Vec::new(), "GBN9Y1100081"))
            .unwrap();

        let other_recording = described("55391790", "Time", Some("GBN9Y1100079"), 4);
        assert!(!store.bind_isrc_match(twin, &other_recording).unwrap());
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391790")).unwrap(),
            None,
            "a refused match writes nothing"
        );

        let on_tidal = described("55391792", "Money", Some("gbn9y1100081"), 6);
        assert!(store.bind_isrc_match(twin, &on_tidal).unwrap());
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391792")).unwrap(),
            Some(twin),
            "the named track, not the first with the ISRC"
        );
        let bindings = store.bindings(twin).unwrap();
        assert_eq!(bindings[0].provenance, Provenance::Isrc);
        assert_eq!(store.albums_of(twin).unwrap().len(), 1);

        assert!(
            store.bind_isrc_match(twin, &on_tidal).unwrap(),
            "idempotent"
        );

        let mut unrelated = described("55391793", "Money", None, 6);
        unrelated.album = None;
        let owner = store.ingest_track(&unrelated).unwrap();
        unrelated.isrc = Some("GBN9Y1100081".into());
        assert!(
            !store.bind_isrc_match(first, &unrelated).unwrap(),
            "a binding owned by a track not known to be this recording is not moved"
        );
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391793")).unwrap(),
            Some(owner)
        );
    }

    /// A match that finds its copy already ingested as a track of its own, with the same ISRC,
    /// has found the recording held twice: the two become one.
    #[test]
    fn an_isrc_match_onto_a_twin_merges_them() {
        let mut store = store();
        let mut from_spotify = described("x", "Money", Some("GBN9Y1100081"), 6);
        from_spotify.source = SourceRef::Spotify { id: "4KW1".into() };
        let asked = store.ingest_track(&from_spotify).unwrap();
        let on_tidal = described("55391792", "Money", Some("GBN9Y1100081"), 6);
        let twin = store
            .add_track(&track("Money", Vec::new(), "GBN9Y1100081"))
            .unwrap();
        store
            .bind(EntityKind::Track, twin, &Binding::direct(tidal("55391792")))
            .unwrap();

        assert!(store.bind_isrc_match(asked, &on_tidal).unwrap());

        assert_eq!(
            store.bound(EntityKind::Track, &tidal("55391792")).unwrap(),
            Some(asked)
        );
        assert_eq!(store.resolve(twin).unwrap(), asked);
        assert!(store.track(twin).unwrap().is_none());
    }

    #[test]
    fn a_merge_carries_everything_that_named_the_twin() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let keep = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let mut held_twice = track("Money", vec![floyd], "USEE10301026");
        held_twice.mbid = Some(Uuid::from_u128(7));
        let gone = store.add_track(&held_twice).unwrap();
        let earlier = store
            .add_track(&track("Time", vec![floyd], "GBN9Y1100086"))
            .unwrap();
        store
            .bind(EntityKind::Track, gone, &Binding::direct(tidal("1")))
            .unwrap();
        let dsotm = store.add_album(&album("DSOTM", vec![floyd])).unwrap();
        store
            .place(
                dsotm,
                AlbumTrack {
                    disc: 1,
                    position: 6,
                    track: gone,
                },
            )
            .unwrap();
        let list = store.create_playlist("mine", &[gone, keep]).unwrap();
        store.save_at(gone, Some(10)).unwrap();
        store.save_at(keep, Some(20)).unwrap();
        store.mark_unmatched(keep, Service::Tidal).unwrap();
        // An alias made earlier, pointing at the track about to go.
        store.merge_tracks(gone, earlier).unwrap();

        store.merge_tracks(keep, gone).unwrap();

        let kept = store.track(keep).unwrap().unwrap();
        assert_eq!(kept.isrcs, ["GBN9Y1100085", "USEE10301026", "GBN9Y1100086"]);
        assert_eq!(kept.mbid, Some(Uuid::from_u128(7)));
        assert_eq!(
            store.bound(EntityKind::Track, &tidal("1")).unwrap(),
            Some(keep)
        );
        assert_eq!(store.tracklist(dsotm).unwrap()[0].track, keep);
        assert_eq!(store.playlist_tracks(list).unwrap(), vec![keep, keep]);
        assert_eq!(store.saved(EntityKind::Track).unwrap(), vec![keep]);
        assert!(
            !store
                .unmatched_within(keep, Service::Tidal, i64::MAX)
                .unwrap()
        );
        assert_eq!(store.resolve(gone).unwrap(), keep);
        assert_eq!(store.resolve(earlier).unwrap(), keep, "aliases are one hop");
        assert_eq!(store.kind_of(gone).unwrap(), None);
    }

    #[test]
    fn tracks_sharing_an_isrc_are_merged_into_the_earliest() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let first = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let second = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let third = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let other = store
            .add_track(&track("Time", vec![floyd], "GBN9Y1100086"))
            .unwrap();

        assert_eq!(store.merge_isrc_twins().unwrap(), 2);

        assert_eq!(store.resolve(second).unwrap(), first);
        assert_eq!(store.resolve(third).unwrap(), first);
        assert_eq!(store.resolve(other).unwrap(), other);
        assert_eq!(store.merge_isrc_twins().unwrap(), 0);
    }

    #[test]
    fn a_failed_ingest_leaves_nothing_behind() {
        let mut store = store();
        let mut broken = described("1", "Broken", None, 1);
        broken.album = Some(SourceAlbum {
            source: Some(SourceRef::Local {
                path: std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/\xff")),
            }),
            title: "Unkeyable".into(),
            ..SourceAlbum::default()
        });
        assert!(store.ingest_track(&broken).is_err());
        assert_eq!(store.bound(EntityKind::Track, &tidal("1")).unwrap(), None);
        assert_eq!(
            store.bound(EntityKind::Artist, &tidal("9706")).unwrap(),
            None
        );
    }

    #[test]
    fn a_library_file_survives_reopening_and_refuses_a_newer_schema() {
        let dir = std::env::temp_dir().join(format!("canon-library-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.sqlite");

        let id = {
            let mut store = Store::open(&path).unwrap();
            let floyd = artist(&mut store, "Pink Floyd");
            store.save(floyd).unwrap();
            floyd
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.artist(id).unwrap().unwrap().name, "Pink Floyd");
        assert_eq!(store.saved(EntityKind::Artist).unwrap(), vec![id]);
        drop(store);

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        let error = Store::open(&path).err().expect("refused");
        assert!(
            error.to_string().contains("newer than this canon"),
            "{error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn isrcs_are_one_case_however_a_service_writes_them() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let id = store
            .add_track(&track("Money", vec![floyd], "gbn9y1100085"))
            .unwrap();
        assert_eq!(store.track(id).unwrap().unwrap().isrcs, ["GBN9Y1100085"]);
        assert_eq!(store.tracks_with_isrc("gbn9Y1100085").unwrap(), vec![id]);
    }

    #[test]
    fn identifying_a_track_learns_its_isrcs_and_asks_the_services_again() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let id = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        store.mark_unmatched(id, Service::Tidal).unwrap();
        assert_eq!(store.unidentified_tracks(10, 1_000_000).unwrap(), vec![id]);

        let recording = Uuid::new_v4();
        let identified = store
            .identify_track(
                id,
                Some(recording),
                &["GBN9Y1100085".into(), "gbaye7300004".into()],
            )
            .unwrap();

        assert_eq!(identified.new_isrcs, 1);
        let known = store.track(id).unwrap().unwrap();
        assert_eq!(known.mbid, Some(recording));
        assert_eq!(known.isrcs, ["GBN9Y1100085", "GBAYE7300004"]);
        assert_eq!(
            store.by_mbid(EntityKind::Track, recording).unwrap(),
            Some(id)
        );
        assert!(
            !store
                .unmatched_within(id, Service::Tidal, i64::MAX)
                .unwrap(),
            "a new ISRC is worth asking about"
        );
        assert!(store.unidentified_tracks(10, 1_000_000).unwrap().is_empty());
    }

    #[test]
    fn an_unidentified_track_is_not_asked_about_again_until_the_window_passes() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let first = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let unmatched = store
            .add_track(&track("Time", vec![floyd], "GBN9Y1100086"))
            .unwrap();
        store.mark_unmatched(unmatched, Service::Tidal).unwrap();
        assert_eq!(
            store.unidentified_tracks(10, 1_000_000).unwrap(),
            vec![unmatched, first],
            "tracks a service lacks come first"
        );

        let nothing = store.identify_track(first, None, &[]).unwrap();
        assert_eq!(nothing, Identified::default());
        assert_eq!(
            store.unidentified_tracks(10, 1_000_000).unwrap(),
            vec![unmatched]
        );
        assert!(
            store
                .unmatched_within(unmatched, Service::Tidal, i64::MAX)
                .unwrap(),
            "nothing learned, nothing cleared"
        );
        assert_eq!(
            store.unidentified_tracks(10, -1).unwrap(),
            vec![unmatched, first],
            "asked again once the window has passed"
        );
    }

    #[test]
    fn identifying_a_track_merges_the_recording_held_twice() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let one = store
            .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
            .unwrap();
        let two = store
            .add_track(&track("Money", vec![floyd], "GBAYE7300004"))
            .unwrap();
        let three = store
            .add_track(&track("Money", vec![floyd], "USEE10301026"))
            .unwrap();
        let recording = Uuid::new_v4();
        store.identify_track(one, Some(recording), &[]).unwrap();

        let identified = store
            .identify_track(two, Some(recording), &["USEE10301026".into()])
            .unwrap();

        assert_eq!(identified.merged.len(), 2);
        assert!(identified.merged.contains(&one) && identified.merged.contains(&three));
        let known = store.track(two).unwrap().unwrap();
        assert_eq!(known.mbid, Some(recording));
        assert_eq!(known.isrcs.len(), 3);
        assert_eq!(store.resolve(one).unwrap(), two);
    }

    #[test]
    fn identifying_an_album_fills_its_release_and_group() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let id = store
            .add_album(&album("The Dark Side of the Moon", vec![floyd]))
            .unwrap();
        let mut meddle = album("Meddle", vec![floyd]);
        meddle.barcode = None;
        let bare = store.add_album(&meddle).unwrap();
        assert_eq!(store.unidentified_albums(10, 1_000_000).unwrap(), vec![id]);

        let (release, group) = (Uuid::new_v4(), Uuid::new_v4());
        store
            .identify_album(id, Some((release, Some(group))))
            .unwrap();

        let known = store.album(id).unwrap().unwrap();
        assert_eq!((known.mbid, known.group_mbid), (Some(release), Some(group)));
        assert!(store.unidentified_albums(10, 1_000_000).unwrap().is_empty());
        assert_eq!(
            store
                .identify_album(bare, Some((release, None)))
                .unwrap()
                .duplicate,
            Some(id)
        );
    }

    #[test]
    fn an_artist_takes_an_mbid_only_once_and_only_one_artist_does() {
        let mut store = store();
        let floyd = artist(&mut store, "Pink Floyd");
        let tribute = artist(&mut store, "Pink Floyd");
        let (mbid, other) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(store.identify_artist(floyd, mbid).unwrap());
        assert!(!store.identify_artist(floyd, other).unwrap());
        assert!(!store.identify_artist(tribute, mbid).unwrap());
        assert_eq!(store.artist(floyd).unwrap().unwrap().mbid, Some(mbid));
    }

    #[test]
    fn upgrading_folds_isrcs_into_one_case() {
        let dir = std::env::temp_dir().join(format!("canon-library-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.sqlite");
        let id = {
            let mut store = Store::open(&path).unwrap();
            let floyd = artist(&mut store, "Pink Floyd");
            store
                .add_track(&track("Money", vec![floyd], "GBN9Y1100085"))
                .unwrap()
        };
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO track_isrcs (track, isrc)
                 SELECT track, lower(isrc) FROM track_isrcs;
             INSERT INTO track_isrcs (track, isrc)
                 SELECT track, 'usee10301026' FROM track_isrcs LIMIT 1;
             DROP TABLE identified;
             DROP TABLE merged;
             PRAGMA user_version = 3;",
        )
        .unwrap();
        drop(conn);

        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.track(id).unwrap().unwrap().isrcs,
            ["GBN9Y1100085", "USEE10301026"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
