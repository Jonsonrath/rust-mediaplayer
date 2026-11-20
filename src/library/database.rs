//! SQLite access: tracks, playlists, history and the persisted queue.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::Track;

#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: usize,
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistItem {
    pub item_id: i64,
    pub track: Track,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub id: i64,
    pub played_at: i64,
    pub played_secs: f64,
    pub completed: bool,
    pub track: Track,
}

/// Schema steps; step `n` brings `user_version` to `n + 1`.
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE tracks (
        id INTEGER PRIMARY KEY,
        path TEXT UNIQUE NOT NULL,
        title TEXT NOT NULL,
        artist TEXT, album TEXT, album_artist TEXT,
        track_no INTEGER, genre TEXT, year INTEGER, duration REAL,
        format TEXT,
        size INTEGER NOT NULL DEFAULT 0,
        width INTEGER, height INTEGER,
        is_video INTEGER NOT NULL DEFAULT 0,
        mtime INTEGER NOT NULL DEFAULT 0,
        scanned_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_tracks_title ON tracks(title);
    CREATE INDEX idx_tracks_artist ON tracks(artist);
    CREATE INDEX idx_tracks_album ON tracks(album);
    CREATE TABLE playlists (
        id INTEGER PRIMARY KEY,
        name TEXT UNIQUE NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE playlist_items (
        id INTEGER PRIMARY KEY,
        playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        position INTEGER NOT NULL
    );
    CREATE INDEX idx_items_playlist ON playlist_items(playlist_id, position);
    CREATE TABLE history (
        id INTEGER PRIMARY KEY,
        track_id INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
        path TEXT NOT NULL,
        title TEXT NOT NULL,
        artist TEXT,
        played_at INTEGER NOT NULL,
        played_secs REAL NOT NULL,
        completed INTEGER NOT NULL
    );
    CREATE TABLE queue (
        position INTEGER PRIMARY KEY,
        track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE
    );",
    // Files that vanish are flagged, not deleted, so playlists/queue keep their entries.
    "ALTER TABLE tracks ADD COLUMN missing INTEGER NOT NULL DEFAULT 0;",
];

const TRACK_COLS: &str = "id, path, title, artist, album, album_artist, genre, track_no, year, \
                          duration, format, size, width, height, is_video";

fn track_from_row(r: &Row) -> rusqlite::Result<Track> {
    track_from_row_at(r, 0)
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // Timeout first: switching to WAL can itself hit a lock held by another connection.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        // IMMEDIATE + re-read inside: two connections opening a fresh DB must not both migrate.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let version: i64 = self
                .conn
                .pragma_query_value(None, "user_version", |r| r.get(0))?;
            for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
                self.conn
                    .execute_batch(&format!("{sql};PRAGMA user_version = {};", i + 1))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(self.conn.execute_batch("COMMIT")?),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Start a batch; pair with `commit`. Used by the scanner to group inserts.
    pub fn begin(&self) -> Result<()> {
        Ok(self.conn.execute_batch("BEGIN")?)
    }

    pub fn commit(&self) -> Result<()> {
        Ok(self.conn.execute_batch("COMMIT")?)
    }

    // ---- tracks ----

    /// Insert or update by path; the id stays stable. Returns the id.
    pub fn upsert_track(&self, t: &Track, mtime: i64) -> Result<i64> {
        let id = self.conn.query_row(
            "INSERT INTO tracks (path, title, artist, album, album_artist, genre, track_no, year,
                duration, format, size, width, height, is_video, mtime, scanned_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(path) DO UPDATE SET
                missing=0, title=excluded.title, artist=excluded.artist, album=excluded.album,
                album_artist=excluded.album_artist, genre=excluded.genre,
                track_no=excluded.track_no, year=excluded.year, duration=excluded.duration,
                format=excluded.format, size=excluded.size, width=excluded.width,
                height=excluded.height, is_video=excluded.is_video, mtime=excluded.mtime,
                scanned_at=excluded.scanned_at
             RETURNING id",
            params![
                path_str(&t.path),
                t.title,
                t.artist,
                t.album,
                t.album_artist,
                t.genre,
                t.track_no,
                t.year,
                t.duration,
                t.format,
                t.size as i64,
                t.width,
                t.height,
                t.is_video,
                mtime,
                now()
            ],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn track_by_path(&self, path: &Path) -> Result<Option<Track>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {TRACK_COLS} FROM tracks WHERE path = ?1"),
                [path_str(path)],
                track_from_row,
            )
            .optional()?)
    }

    fn track_by_id(&self, id: i64) -> Result<Option<Track>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1"),
                [id],
                track_from_row,
            )
            .optional()?)
    }

    /// path -> (mtime, size) for every indexed file not already flagged missing.
    pub fn known_files(&self) -> Result<HashMap<PathBuf, (i64, u64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, mtime, size FROM tracks WHERE missing = 0")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                (r.get(1)?, r.get::<_, i64>(2)? as u64),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Drop rows for vanished files. Rows used by a playlist are only flagged missing
    /// (hidden from the library) so the playlist keeps its entries.
    pub fn remove_paths(&self, paths: &[PathBuf]) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<()> {
            let mut del = self.conn.prepare(
                "DELETE FROM tracks WHERE path = ?1
                 AND id NOT IN (SELECT track_id FROM playlist_items)",
            )?;
            let mut flag = self
                .conn
                .prepare("UPDATE tracks SET missing = 1 WHERE path = ?1")?;
            for p in paths {
                let p = path_str(p);
                del.execute([&p])?;
                flag.execute([&p])?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(self.conn.execute_batch("COMMIT")?),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn all_tracks(&self) -> Result<Vec<Track>> {
        self.query_tracks("WHERE missing = 0", "")
    }

    /// Case-insensitive substring match on title/artist/album/path.
    #[cfg(test)]
    pub fn search(&self, query: &str) -> Result<Vec<Track>> {
        let pat = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        self.query_tracks(
            "WHERE missing = 0 AND (title LIKE ?1 ESCAPE '\\' OR artist LIKE ?1 ESCAPE '\\'
                OR album LIKE ?1 ESCAPE '\\' OR path LIKE ?1 ESCAPE '\\')",
            &pat,
        )
    }

    fn query_tracks(&self, filter: &str, pat: &str) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {TRACK_COLS} FROM tracks {filter}
             ORDER BY artist COLLATE NOCASE, album COLLATE NOCASE, track_no, title COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if pat.is_empty() {
            stmt.query_map([], track_from_row)?
                .collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map([pat], track_from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    /// Returns the indexed id, indexing the track first if needed.
    fn ensure_indexed(&self, t: &Track) -> Result<i64> {
        if t.id != 0 && self.track_by_id(t.id)?.is_some() {
            return Ok(t.id);
        }
        // Already indexed by the scanner: keep its metadata and mtime.
        if let Some(existing) = self.track_by_path(&t.path)? {
            return Ok(existing.id);
        }
        self.upsert_track(t, 0)
    }

    // ---- playlists ----

    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, COUNT(i.id), COALESCE(SUM(t.duration), 0)
             FROM playlists p
             LEFT JOIN playlist_items i ON i.playlist_id = p.id
             LEFT JOIN tracks t ON t.id = i.track_id
             GROUP BY p.id ORDER BY p.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                name: r.get(1)?,
                track_count: r.get::<_, i64>(2)? as usize,
                duration: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_playlist(&self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            bail!("playlist name cannot be empty");
        }
        match self.conn.execute(
            "INSERT INTO playlists (name, created_at) VALUES (?1, ?2)",
            params![name, now()],
        ) {
            Ok(_) => Ok(self.conn.last_insert_rowid()),
            Err(e) if is_unique_violation(&e) => bail!("playlist \"{name}\" already exists"),
            Err(e) => Err(e.into()),
        }
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("playlist name cannot be empty");
        }
        match self.conn.execute(
            "UPDATE playlists SET name = ?1 WHERE id = ?2",
            params![name, id],
        ) {
            Ok(_) => Ok(()),
            Err(e) if is_unique_violation(&e) => bail!("playlist \"{name}\" already exists"),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn playlist_items(&self, pid: i64) -> Result<Vec<PlaylistItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT i.id, {} FROM playlist_items i JOIN tracks t ON t.id = i.track_id
             WHERE i.playlist_id = ?1 ORDER BY i.position, i.id",
            TRACK_COLS
                .split(", ")
                .map(|c| format!("t.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let rows = stmt.query_map([pid], |r| {
            Ok(PlaylistItem {
                item_id: r.get(0)?,
                track: track_from_row_at(r, 1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_to_playlist(&self, pid: i64, tracks: &[Track]) -> Result<()> {
        let start: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_items WHERE playlist_id = ?1",
            [pid],
            |r| r.get(0),
        )?;
        for (pos, t) in (start..).zip(tracks) {
            let tid = self.ensure_indexed(t)?;
            self.conn.execute(
                "INSERT INTO playlist_items (playlist_id, track_id, position) VALUES (?1,?2,?3)",
                params![pid, tid, pos],
            )?;
        }
        Ok(())
    }

    pub fn remove_playlist_item(&self, item_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlist_items WHERE id = ?1", [item_id])?;
        Ok(())
    }

    /// Swap with the neighbour above/below. False if already at the edge.
    pub fn move_playlist_item(&self, item_id: i64, up: bool) -> Result<bool> {
        let Some((pid, pos)) = self
            .conn
            .query_row(
                "SELECT playlist_id, position FROM playlist_items WHERE id = ?1",
                [item_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let sql = if up {
            "SELECT id, position FROM playlist_items WHERE playlist_id = ?1
             AND (position < ?2 OR (position = ?2 AND id < ?3)) ORDER BY position DESC, id DESC LIMIT 1"
        } else {
            "SELECT id, position FROM playlist_items WHERE playlist_id = ?1
             AND (position > ?2 OR (position = ?2 AND id > ?3)) ORDER BY position, id LIMIT 1"
        };
        let Some((other, other_pos)) = self
            .conn
            .query_row(sql, params![pid, pos, item_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        else {
            return Ok(false);
        };
        // Equal positions would make the swap a no-op; renumber the pair explicitly.
        let (a, b) = if other_pos == pos {
            if up { (pos - 1, pos) } else { (pos + 1, pos) }
        } else {
            (other_pos, pos)
        };
        self.conn.execute_batch(&format!(
            "BEGIN; UPDATE playlist_items SET position = {a} WHERE id = {item_id};
             UPDATE playlist_items SET position = {b} WHERE id = {other}; COMMIT;"
        ))?;
        Ok(true)
    }

    // ---- history ----

    pub fn add_history(
        &self,
        track: &Track,
        started_at: i64,
        played_secs: f64,
        completed: bool,
    ) -> Result<()> {
        let tid = (track.id != 0 && self.track_by_id(track.id)?.is_some()).then_some(track.id);
        self.conn.execute(
            "INSERT INTO history (track_id, path, title, artist, played_at, played_secs, completed)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                tid,
                path_str(&track.path),
                track.title,
                track.artist,
                started_at,
                played_secs,
                completed
            ],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_id, path, title, artist, played_at, played_secs, completed
             FROM history ORDER BY played_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok((
                    HistoryEntry {
                        id: r.get(0)?,
                        played_at: r.get(5)?,
                        played_secs: r.get(6)?,
                        completed: r.get(7)?,
                        track: Track {
                            id: 0,
                            path: PathBuf::from(r.get::<_, String>(2)?),
                            title: r.get(3)?,
                            artist: r.get(4)?,
                            album: None,
                            album_artist: None,
                            genre: None,
                            track_no: None,
                            year: None,
                            duration: None,
                            format: None,
                            size: 0,
                            width: None,
                            height: None,
                            is_video: false,
                        },
                    },
                    r.get::<_, Option<i64>>(1)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (mut entry, tid) in rows {
            if let Some(full) = tid.map(|id| self.track_by_id(id)).transpose()?.flatten() {
                entry.track = full;
            }
            out.push(entry);
        }
        Ok(out)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    // ---- queue ----

    pub fn save_queue(&self, tracks: &[Track]) -> Result<()> {
        self.conn.execute_batch("BEGIN; DELETE FROM queue;")?;
        let result = (|| -> Result<()> {
            for (pos, t) in tracks.iter().enumerate() {
                let tid = self.ensure_indexed(t)?;
                self.conn.execute(
                    "INSERT INTO queue (position, track_id) VALUES (?1, ?2)",
                    params![pos as i64, tid],
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(self.conn.execute_batch("COMMIT")?),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Persisted queue, skipping files that no longer exist.
    pub fn load_queue(&self) -> Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM queue q JOIN tracks t ON t.id = q.track_id ORDER BY q.position",
            TRACK_COLS
                .split(", ")
                .map(|c| format!("t.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let tracks: Vec<Track> = stmt
            .query_map([], track_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(tracks.into_iter().filter(|t| t.path.exists()).collect())
    }
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// `track_from_row` starting at column `off`.
fn track_from_row_at(r: &Row, off: usize) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(off)?,
        path: PathBuf::from(r.get::<_, String>(off + 1)?),
        title: r.get(off + 2)?,
        artist: r.get(off + 3)?,
        album: r.get(off + 4)?,
        album_artist: r.get(off + 5)?,
        genre: r.get(off + 6)?,
        track_no: r.get(off + 7)?,
        year: r.get(off + 8)?,
        duration: r.get(off + 9)?,
        format: r.get(off + 10)?,
        size: r.get::<_, i64>(off + 11)? as u64,
        width: r.get(off + 12)?,
        height: r.get(off + 13)?,
        is_video: r.get(off + 14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn track(path: &str, title: &str, artist: &str) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(path),
            title: title.into(),
            artist: Some(artist.into()),
            album: Some("Alb".into()),
            album_artist: None,
            genre: None,
            track_no: Some(1),
            year: None,
            duration: Some(10.0),
            format: Some("MP3".into()),
            size: 5,
            width: None,
            height: None,
            is_video: false,
        }
    }

    #[test]
    fn upsert_keeps_id_and_unique_path() {
        let db = Database::open_in_memory().unwrap();
        let a = db.upsert_track(&track("/a.mp3", "A", "X"), 1).unwrap();
        let again = db.upsert_track(&track("/a.mp3", "A2", "X"), 2).unwrap();
        assert_eq!(a, again);
        assert_eq!(db.all_tracks().unwrap().len(), 1);
        let t = db.track_by_path(Path::new("/a.mp3")).unwrap().unwrap();
        assert_eq!((t.id, t.title.as_str()), (a, "A2"));
        assert_eq!(db.known_files().unwrap()[Path::new("/a.mp3")], (2, 5));
        db.remove_paths(&[PathBuf::from("/a.mp3")]).unwrap();
        assert!(db.all_tracks().unwrap().is_empty());
    }

    #[test]
    fn removal_keeps_playlist_rows_hidden_and_revives() {
        let db = Database::open_in_memory().unwrap();
        let used = track("/u.mp3", "U", "A");
        let free = track("/f.mp3", "F", "A");
        db.upsert_track(&free, 1).unwrap();
        let pid = db.create_playlist("P").unwrap();
        db.add_to_playlist(pid, std::slice::from_ref(&used))
            .unwrap();
        db.remove_paths(&[used.path.clone(), free.path.clone()])
            .unwrap();
        assert!(db.all_tracks().unwrap().is_empty());
        assert!(db.search("U").unwrap().is_empty());
        assert!(db.known_files().unwrap().is_empty());
        assert_eq!(db.playlist_items(pid).unwrap().len(), 1);
        assert!(db.track_by_path(&free.path).unwrap().is_none());
        db.upsert_track(&used, 2).unwrap();
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_connections_open_fresh_db_and_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lib.db");
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let db = Database::open(&p).unwrap();
                    db.upsert_track(&track(&format!("/{i}.mp3"), "t", "a"), 1)
                        .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(Database::open(&p).unwrap().all_tracks().unwrap().len(), 4);
    }

    #[test]
    fn ten_thousand_tracks_are_fast() {
        let db = Database::open_in_memory().unwrap();
        db.begin().unwrap();
        for i in 0..10_000 {
            let mut t = track(
                &format!("/m/{i}.mp3"),
                &format!("Song {i}"),
                &format!("Art{}", i % 100),
            );
            t.album = Some(format!("Alb{}", i % 500));
            db.upsert_track(&t, 1).unwrap();
        }
        db.commit().unwrap();
        let t0 = std::time::Instant::now();
        assert_eq!(db.all_tracks().unwrap().len(), 10_000);
        let all = t0.elapsed();
        let t1 = std::time::Instant::now();
        assert_eq!(db.search("song 99").unwrap().len(), 111);
        let search = t1.elapsed();
        eprintln!("all_tracks {all:?}, search {search:?}");
        assert!(all.as_millis() < 100 && search.as_millis() < 100);
    }

    #[test]
    fn migrates_from_zero_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/lib.db");
        {
            let db = Database::open(&p).unwrap();
            let v: i64 = db
                .conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(v as usize, MIGRATIONS.len());
            db.upsert_track(&track("/a.mp3", "A", "X"), 1).unwrap();
        }
        let db = Database::open(&p).unwrap();
        assert_eq!(db.all_tracks().unwrap().len(), 1);
    }

    #[test]
    fn search_escapes_and_ignores_case() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_track(&track("/a.mp3", "100% Real", "Foo"), 1)
            .unwrap();
        db.upsert_track(&track("/b.mp3", "a_b", "Bar"), 1).unwrap();
        db.upsert_track(&track("/c.mp3", "axb", "Bar"), 1).unwrap();
        assert_eq!(db.search("100%").unwrap().len(), 1);
        assert_eq!(db.search("a_b").unwrap().len(), 1);
        assert_eq!(db.search("FOO").unwrap().len(), 1);
        assert_eq!(db.search("%").unwrap().len(), 1);
        assert_eq!(db.search("").unwrap().len(), 3);
    }

    #[test]
    fn playlist_crud_order_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lib.db");
        let (pid, ids) = {
            let db = Database::open(&p).unwrap();
            assert!(db.create_playlist("  ").is_err());
            let pid = db.create_playlist("Mix").unwrap();
            assert!(
                db.create_playlist("Mix")
                    .unwrap_err()
                    .to_string()
                    .contains("exists")
            );
            // unindexed tracks get indexed on add
            db.add_to_playlist(
                pid,
                &[
                    track("/1.mp3", "1", "A"),
                    track("/2.mp3", "2", "A"),
                    track("/3.mp3", "3", "A"),
                ],
            )
            .unwrap();
            let items = db.playlist_items(pid).unwrap();
            let ids: Vec<i64> = items.iter().map(|i| i.item_id).collect();
            assert!(db.move_playlist_item(ids[2], true).unwrap());
            assert!(!db.move_playlist_item(ids[0], true).unwrap());
            let titles: Vec<_> = db
                .playlist_items(pid)
                .unwrap()
                .into_iter()
                .map(|i| i.track.title)
                .collect();
            assert_eq!(titles, ["1", "3", "2"]);
            (pid, ids)
        };
        let db = Database::open(&p).unwrap();
        let titles: Vec<_> = db
            .playlist_items(pid)
            .unwrap()
            .into_iter()
            .map(|i| i.track.title)
            .collect();
        assert_eq!(titles, ["1", "3", "2"]);
        let pl = &db.playlists().unwrap()[0];
        assert_eq!((pl.track_count, pl.duration), (3, 30.0));
        assert!(db.move_playlist_item(ids[0], false).unwrap());
        db.remove_playlist_item(ids[1]).unwrap();
        assert_eq!(db.playlist_items(pid).unwrap().len(), 2);
        db.rename_playlist(pid, "Renamed").unwrap();
        db.delete_playlist(pid).unwrap();
        assert!(db.playlists().unwrap().is_empty());
        assert_eq!(db.all_tracks().unwrap().len(), 3);
    }

    #[test]
    fn history_snapshot_survives_track_removal() {
        let db = Database::open_in_memory().unwrap();
        let mut t = track("/h.mp3", "H", "Z");
        t.id = db.upsert_track(&t, 1).unwrap();
        db.add_history(&t, 100, 12.5, true).unwrap();
        db.add_history(&t, 200, 3.0, false).unwrap();
        let h = db.history(10).unwrap();
        assert_eq!((h[0].played_at, h[1].played_at), (200, 100));
        assert_eq!(h[1].track.album.as_deref(), Some("Alb"));
        db.remove_paths(&[t.path.clone()]).unwrap();
        let h = db.history(1).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].track.title.as_str(), h[0].track.id), ("H", 0));
        db.clear_history().unwrap();
        assert!(db.history(10).unwrap().is_empty());
    }

    #[test]
    fn queue_roundtrip_skips_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.mp3");
        std::fs::write(&real, b"x").unwrap();
        let db = Database::open_in_memory().unwrap();
        let q = [
            track(real.to_str().unwrap(), "real", "A"),
            track("/nope/gone.mp3", "gone", "A"),
            track(real.to_str().unwrap(), "real", "A"),
        ];
        db.save_queue(&q).unwrap();
        let loaded = db.load_queue().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].path, real);
        db.save_queue(&[]).unwrap();
        assert!(db.load_queue().unwrap().is_empty());
    }
}
