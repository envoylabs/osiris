//! SQLite adapter for the `Store` port of `ris-core`.
//!
//! One file, WAL mode, one table of events. Each append is an immediate
//! transaction that re-reads the head and refuses if the events do not
//! follow it, so any number of processes may hold connections and
//! exactly one of two concurrent appends after the same head succeeds.
//! Event parts are stored as JSON so the `sqlite3` tool can read them.
//!
//! Schema history: version 1 stored format-version-1 events (no `at`);
//! version 2 stored format-version-2 events; version 3 stores format
//! version 3, whose JSON is the shown form. Opening an older file
//! upgrades it in place, inside one transaction: the old chain is
//! verified from the stored bytes, the log is rechained under the
//! current format, and the schema version is set to 3.

use std::path::Path;

use ris_core::event::{Event, Salt, Structure, TextPart};
use ris_core::ids::Hash;
use ris_core::store::{Store, StoreError, check_follows};
use rusqlite::{Connection, OptionalExtension, params};

/// The schema this build writes. A file with another version is refused.
pub const SCHEMA_VERSION: i64 = 3;

/// A store backed by one SQLite file.
pub struct SqliteStore {
    conn: Connection,
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

impl SqliteStore {
    /// Open or create the store at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        Self::init(conn)
    }

    /// An in-memory store, for tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory().map_err(backend)?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(backend)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(backend)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(backend)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                 seq       INTEGER PRIMARY KEY,
                 version   INTEGER NOT NULL,
                 prev      BLOB    NOT NULL,
                 at        INTEGER NOT NULL DEFAULT 0,
                 structure TEXT    NOT NULL,
                 text_hash BLOB    NOT NULL,
                 salt      BLOB,
                 text      TEXT,
                 hash      BLOB    NOT NULL
             );",
        )
        .map_err(backend)?;
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        match version {
            None => {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )
                .map_err(backend)?;
            }
            Some(v) if v == SCHEMA_VERSION.to_string() => {}
            Some(v) if v == "1" || v == "2" => Self::upgrade_old(&conn)?,
            Some(v) => {
                return Err(StoreError::Backend(format!(
                    "store has schema version {v}; this build reads {SCHEMA_VERSION}"
                )));
            }
        }
        Ok(Self { conn })
    }

    /// Schemas 1 and 2 held older event formats (schema 1 without
    /// `at`). Add the column if missing, verify and rechain the log
    /// under the current format, and bump the schema, in one
    /// transaction.
    fn upgrade_old(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(backend)?;
        let result = (|| {
            let has_at: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'at'",
                    [],
                    |r| r.get(0),
                )
                .map_err(backend)?;
            if has_at == 0 {
                conn.execute_batch("ALTER TABLE events ADD COLUMN at INTEGER NOT NULL DEFAULT 0")
                    .map_err(backend)?;
            }
            let old = Self::read_raw_on(conn)?;
            let new = ris_core::event::upgrade(&old)?;
            conn.execute_batch("DELETE FROM events").map_err(backend)?;
            Self::insert_on(conn, &new)?;
            conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(backend)?;
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(backend),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn insert_on(conn: &Connection, events: &[Event]) -> Result<(), StoreError> {
        let mut insert = conn
            .prepare_cached(
                "INSERT INTO events
                 (seq, version, prev, at, structure, text_hash, salt, text, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(backend)?;
        for e in events {
            let structure = serde_json::to_string(&e.structure).map_err(backend)?;
            let text = match &e.text {
                Some(t) => Some(serde_json::to_string(t).map_err(backend)?),
                None => None,
            };
            insert
                .execute(params![
                    e.seq as i64,
                    i64::from(e.version),
                    e.prev.0.as_slice(),
                    e.at as i64,
                    structure,
                    e.text_hash.0.as_slice(),
                    e.salt.as_ref().map(|s| s.0.as_slice()),
                    text,
                    e.hash.0.as_slice(),
                ])
                .map_err(backend)?;
        }
        Ok(())
    }

    fn read_from_on(conn: &Connection, from: u64) -> Result<Vec<Event>, StoreError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT seq, version, prev, at, structure, text_hash, salt, text, hash
                 FROM events WHERE seq >= ?1 ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![from as i64], row_to_event)
            .map_err(backend)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(backend)??);
        }
        Ok(events)
    }

    /// The log as stored, the structure kept as its stored text, for an
    /// upgrade to verify against the old chain.
    fn read_raw_on(conn: &Connection) -> Result<Vec<ris_core::RawEvent>, StoreError> {
        let mut stmt = conn
            .prepare(
                "SELECT seq, version, prev, at, structure, text_hash, salt, text, hash
                 FROM events ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, Option<Vec<u8>>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Vec<u8>>(8)?,
                ))
            })
            .map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, version, prev, at, structure, text_hash, salt, text, hash) =
                row.map_err(backend)?;
            let salt = match salt {
                Some(s) => Some(Salt(s.as_slice().try_into().map_err(|_| {
                    StoreError::Backend(format!("event {seq}: salt is not 16 bytes"))
                })?)),
                None => None,
            };
            let text: Option<TextPart> = match text {
                Some(t) => Some(serde_json::from_str(&t).map_err(backend)?),
                None => None,
            };
            out.push(ris_core::RawEvent {
                seq: seq as u64,
                version: version as u16,
                prev: hash_from(&prev)?,
                at: at as u64,
                structure,
                text_hash: hash_from(&text_hash)?,
                salt,
                text,
                hash: hash_from(&hash)?,
            });
        }
        Ok(out)
    }

    fn head_on(conn: &Connection) -> Result<(u64, Hash), StoreError> {
        let row: Option<(i64, Vec<u8>)> = conn
            .query_row(
                "SELECT seq, hash FROM events ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(backend)?;
        match row {
            None => Ok((0, Hash::ZERO)),
            Some((seq, hash)) => Ok((seq as u64, hash_from(&hash)?)),
        }
    }
}

fn hash_from(bytes: &[u8]) -> Result<Hash, StoreError> {
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StoreError::Backend("a stored hash is not 32 bytes".into()))?;
    Ok(Hash(arr))
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Event, StoreError>> {
    let seq: i64 = r.get(0)?;
    let version: i64 = r.get(1)?;
    let prev: Vec<u8> = r.get(2)?;
    let at: i64 = r.get(3)?;
    let structure: String = r.get(4)?;
    let text_hash: Vec<u8> = r.get(5)?;
    let salt: Option<Vec<u8>> = r.get(6)?;
    let text: Option<String> = r.get(7)?;
    let hash: Vec<u8> = r.get(8)?;
    Ok((|| {
        let structure: Structure = serde_json::from_str(&structure).map_err(backend)?;
        let text: Option<TextPart> = match text {
            Some(t) => Some(serde_json::from_str(&t).map_err(backend)?),
            None => None,
        };
        let salt = match salt {
            Some(s) => Some(Salt(s.try_into().map_err(|_| {
                StoreError::Backend("a stored salt is not 16 bytes".into())
            })?)),
            None => None,
        };
        Ok(Event {
            seq: seq as u64,
            version: version as u16,
            prev: hash_from(&prev)?,
            at: at as u64,
            structure,
            text_hash: hash_from(&text_hash)?,
            salt,
            text,
            hash: hash_from(&hash)?,
        })
    })())
}

impl Store for SqliteStore {
    fn append(&mut self, events: &[Event]) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let (len, head) = Self::head_on(&tx)?;
        check_follows(len, head, events)?;
        Self::insert_on(&tx, events)?;
        tx.commit().map_err(backend)
    }

    fn read_from(&self, from: u64) -> Result<Vec<Event>, StoreError> {
        Self::read_from_on(&self.conn, from)
    }

    fn head(&self) -> Result<(u64, Hash), StoreError> {
        Self::head_on(&self.conn)
    }
}

/// A lock held for the length of one merge, so merges happen one at a
/// time. It is an exclusive transaction on its own SQLite file: other
/// processes wait up to `wait` for it, and the operating system releases
/// it if the holding process dies, so a crash never leaves it stuck.
pub struct MergeLock {
    _conn: Connection,
}

impl MergeLock {
    pub fn acquire(path: &Path, wait: std::time::Duration) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        conn.busy_timeout(wait).map_err(backend)?;
        conn.execute_batch("BEGIN EXCLUSIVE")
            .map_err(|e| StoreError::Backend(format!("another merge holds the lock: {e}")))?;
        Ok(Self { _conn: conn })
    }
}

#[cfg(test)]
#[path = "tests/lib.rs"]
mod tests;
