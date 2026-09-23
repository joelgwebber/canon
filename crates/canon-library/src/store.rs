//! [`Store`]: the library's entities in sqlite, behind plain synchronous calls.
//!
//! Synchronous on purpose: every call is a handful of indexed queries, and keeping them plain
//! functions over one connection keeps them easy to test and to compose in a transaction. The
//! async world reaches them through [`crate::Library::run`], which runs them on the blocking pool.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use canon_core::{
    EntityId, Error, Result, Service, SourceAlbum, SourceArtist, SourceRef, SourceTrack, TrackMeta,
    TrackRef,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::model::{Album, AlbumTrack, Artist, Binding, EntityKind, Provenance, Track};
use crate::schema;

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
            isrc,
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
        let kind = self
            .kind_of(id)?
            .ok_or_else(|| Error::NotFound(format!("entity {id}")))?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO saved (entity, kind, saved_at) VALUES (?1, ?2, ?3)",
                params![text(id), kind.as_str(), now_ms()],
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
            let artists = store.ingest_artists(&described.artists)?;
            let matched = match described.isrc.as_deref() {
                Some(isrc) => store.tracks_with_isrc(isrc)?.into_iter().next(),
                None => None,
            };
            let (track, binding) = match matched {
                Some(track) => (
                    track,
                    Binding {
                        source: described.source.clone(),
                        provenance: Provenance::Isrc,
                        confidence: 1.0,
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
        })
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

    /// The artists a service credits, each found by its binding or created. An artist the
    /// service gives no id for can't be told apart from a namesake, so it is always new.
    fn ingest_artists(&mut self, described: &[SourceArtist]) -> Result<Vec<EntityId>> {
        let mut artists = Vec::with_capacity(described.len());
        for artist in described {
            if let Some(source) = &artist.source
                && let Some(id) = self.bound(EntityKind::Artist, source)?
            {
                artists.push(id);
                continue;
            }
            let id = self.add_artist(&Artist {
                name: artist.name.clone(),
                sort_name: None,
                mbid: None,
            })?;
            if let Some(source) = &artist.source {
                self.bind(EntityKind::Artist, id, &Binding::direct(source.clone()))?;
            }
            artists.push(id);
        }
        Ok(artists)
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
            params![text(id), isrc],
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
}
