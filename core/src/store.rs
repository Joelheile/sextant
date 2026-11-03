//! SQLCipher database: open with a key, versioned migrations, read-only readers, retention.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::model::EventKind;
use crate::redact::Secret;
use crate::time::{DAY, Local};
use crate::writer::unpack;

/// The 0.1.0 schema. A later change appends a migration here.
const MIGRATIONS: &[&str] = &[include_str!("schema.sql")];

const EMAIL_SECRET: &str = "email_secret";
const LAST_VACUUM: &str = "last_vacuum";

/// Decrypted pages each connection keeps, in KiB. SQLCipher turns memory mapping off, so this
/// cache is what spares a read the AES and HMAC pass.
const CACHE_KIB: i64 = 16 * 1024;
/// The WAL file shrinks back to this after a checkpoint. Every refresh rewrites the engine tables.
const WAL_LIMIT_BYTES: i64 = 64 * 1024 * 1024;
const VACUUM_EVERY: i64 = 7 * DAY;
/// No capture for this long counts as away, so the vacuum never pauses capture while you work.
const AWAY_S: i64 = 15 * 60;

pub type Key = [u8; 32];

pub struct Store {
    pub(crate) conn: Connection,
    path: PathBuf,
    key: Key,
    /// Keys the email hashes. Lives in the `meta` table, so it never leaves the encrypted file.
    pub(crate) secret: Secret,
}

/// Rows removed by [`Store::retention`] and [`Store::forget`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Retained {
    pub frames: usize,
    pub utterances: usize,
    pub events: usize,
    pub lines: usize,
    pub windows: usize,
}

/// Row counts of the raw tables. Numbers only, safe for logs and tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub events: usize,
    pub frames: usize,
    pub lines: usize,
    pub windows: usize,
    pub utterances: usize,
}

impl Store {
    /// Opens or creates the encrypted database and migrates it to the current schema. `secret` is
    /// stored on first open (the app passes random bytes); a database that has one keeps it.
    pub fn open(path: &Path, key: &Key, secret: &Secret) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        unlock(&conn, key)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", -CACHE_KIB)?;
        conn.pragma_update(None, "journal_size_limit", WAL_LIMIT_BYTES)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn, MIGRATIONS)?;
        conn.execute_batch("PRAGMA optimize = 0x10002")?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)",
            params![EMAIL_SECRET, secret],
        )?;
        let secret = load_secret(&conn)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            key: *key,
            secret,
        })
    }

    /// A read-only connection to the same file. WAL mode lets it run while the writer writes.
    pub fn reader(&self) -> Result<Self> {
        Self::open_read_only(&self.path, &self.key)
    }

    /// Opens an existing database without writing: no migration, no journal change. Fails with
    /// `SchemaTooOld` when the app has not migrated the file yet.
    pub fn open_read_only(path: &Path, key: &Key) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        unlock(&conn, key)?;
        conn.pragma_update(None, "cache_size", -CACHE_KIB)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let found = user_version(&conn)?;
        if found != MIGRATIONS.len() as i32 {
            return Err(Error::SchemaTooOld {
                found,
                expected: MIGRATIONS.len() as i32,
            });
        }
        let secret = load_secret(&conn)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            key: *key,
            secret,
        })
    }

    /// The key for email hashes, as stored in the database.
    pub fn secret(&self) -> Secret {
        self.secret
    }

    pub fn counts(&self) -> Result<Counts> {
        let count = |table: &str| -> Result<usize> {
            Ok(self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })? as usize)
        };
        Ok(Counts {
            events: count("events")?,
            frames: count("frames")?,
            lines: count("lines")?,
            windows: count("windows")?,
            utterances: count("utterances")?,
        })
    }

    /// Live events of one kind, for example how many `focus` events were written.
    pub fn event_count(&self, kind: EventKind) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE kind = ?1",
            params![kind.as_str()],
            |row| row.get::<_, i64>(0),
        )? as usize)
    }

    /// A short-lived keyed write connection, so a read-only reader can append a row.
    pub(crate) fn write_conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        unlock(&conn, &self.key)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(conn)
    }

    /// Hourly upkeep: updates the query planner's statistics and, once a week while the user is
    /// away, rewrites the file so deleted rows give their space back (plan.md §5). `true` when
    /// it vacuumed.
    pub fn maintain(&mut self, now: i64) -> Result<bool> {
        self.conn.execute_batch("PRAGMA optimize")?;
        let last: i64 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![LAST_VACUUM],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let away = self
            .last_capture()?
            .is_none_or(|(ts, _)| now - ts >= AWAY_S);
        if now - last < VACUUM_EVERY || !away {
            return Ok(false);
        }
        self.conn.execute_batch("VACUUM")?;
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            params![LAST_VACUUM, now],
        )?;
        Ok(true)
    }

    /// When the last screen was captured and in which app.
    pub fn last_capture(&self) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT f.ts, w.app FROM frames f JOIN windows w ON w.id = f.window_id
                 ORDER BY f.ts DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn schema_version(&self) -> Result<i32> {
        user_version(&self.conn)
    }

    /// Deletes raw capture rows older than `days` (plan.md §5) and lines and windows nothing
    /// references, then refreshes, so the engine tables hold only what the remaining rows make.
    pub fn retention(&mut self, when: Local, days: u32) -> Result<Retained> {
        self.forget(when, i64::MIN, when.now - i64::from(days) * DAY)
    }

    /// Deletes raw rows from `from` up to `to` (Unix seconds, `to` excluded), with the lines and
    /// windows only they used and every utterance that overlaps the range, then refreshes.
    pub fn forget(&mut self, when: Local, from: i64, to: i64) -> Result<Retained> {
        let tx = self.conn.transaction()?;
        let mut gone = Retained::default();
        let mut line_ids = HashSet::new();
        let mut window_ids = HashSet::new();
        {
            let mut old =
                tx.prepare("SELECT id, window_id, packed FROM frames WHERE ts >= ?1 AND ts < ?2")?;
            let mut unlink =
                tx.prepare("DELETE FROM frame_lines WHERE line_id = ?1 AND frame_id = ?2")?;
            let mut rows = old.query(params![from, to])?;
            while let Some(row) = rows.next()? {
                let (frame_id, window_id): (i64, i64) = (row.get(0)?, row.get(1)?);
                window_ids.insert(window_id);
                for (line_id, _) in unpack(&row.get::<_, Vec<u8>>(2)?) {
                    unlink.execute(params![line_id, frame_id])?;
                    line_ids.insert(line_id);
                }
            }
        }
        let range = params![from, to];
        gone.frames = tx.execute("DELETE FROM frames WHERE ts >= ?1 AND ts < ?2", range)?;
        gone.utterances = tx.execute(
            "DELETE FROM utterances WHERE end_ts >= ?1 AND start_ts < ?2",
            range,
        )?;
        gone.events = tx.execute("DELETE FROM events WHERE ts >= ?1 AND ts < ?2", range)?;
        tx.execute("DELETE FROM sent WHERE ts >= ?1 AND ts < ?2", range)?;
        {
            let mut line = tx.prepare(
                "DELETE FROM lines WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM frame_lines WHERE line_id = ?1)",
            )?;
            for id in &line_ids {
                gone.lines += line.execute(params![id])?;
            }
            let mut window = tx.prepare(
                "DELETE FROM windows WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM frames WHERE window_id = ?1)",
            )?;
            for id in &window_ids {
                gone.windows += window.execute(params![id])?;
            }
        }
        tx.commit()?;
        self.refresh(when)?;
        Ok(gone)
    }

    /// Bytes the database uses, without the free pages a vacuum would return.
    pub fn used_bytes(&self) -> Result<u64> {
        let bytes: i64 = self.conn.query_row(
            "SELECT (p.page_count - f.freelist_count) * s.page_size
             FROM pragma_page_count() p, pragma_freelist_count() f, pragma_page_size() s",
            [],
            |row| row.get(0),
        )?;
        Ok(bytes.max(0) as u64)
    }

    /// Deletes the oldest day of raw data until the database uses at most `cap` bytes. The
    /// freed pages are reused, so the file stops growing. Returns the days deleted.
    pub fn cap_storage(&mut self, when: Local, cap: u64) -> Result<usize> {
        let mut days = 0;
        while self.used_bytes()? > cap {
            let Some(oldest) = self.oldest_raw()? else {
                break;
            };
            self.forget(when, i64::MIN, oldest + DAY)?;
            days += 1;
        }
        Ok(days)
    }

    fn oldest_raw(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT MIN(ts) FROM (SELECT MIN(ts) AS ts FROM frames
                UNION ALL SELECT MIN(ts) FROM events
                UNION ALL SELECT MIN(start_ts) FROM utterances)",
            [],
            |row| row.get(0),
        )?)
    }
}

fn unlock(conn: &Connection, key: &Key) -> Result<()> {
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))?;
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == ErrorCode::NotADatabase =>
        {
            Err(Error::BadKey)
        }
        Err(other) => Err(other.into()),
    }
}

fn load_secret(conn: &Connection) -> Result<Secret> {
    let found: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![EMAIL_SECRET],
            |row| row.get(0),
        )
        .optional()?;
    found
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(Error::NoSecret)
}

fn user_version(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Applies every migration newer than the database's `user_version`, one transaction each.
pub(crate) fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let found = user_version(conn)?;
    let supported = migrations.len() as i32;
    if found > supported {
        return Err(Error::SchemaTooNew { found, supported });
    }
    for (index, sql) in migrations.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_in_order_and_stop_at_newer_versions() {
        let mut conn = Connection::open_in_memory().expect("memory db");
        migrate(&mut conn, &["CREATE TABLE a(x)"]).expect("v1");
        assert_eq!(user_version(&conn).expect("version"), 1);
        migrate(
            &mut conn,
            &["CREATE TABLE a(x)", "ALTER TABLE a ADD COLUMN y"],
        )
        .expect("v2");
        assert_eq!(user_version(&conn).expect("version"), 2);
        conn.execute("INSERT INTO a(x, y) VALUES (1, 2)", [])
            .expect("new column exists");
        let err = migrate(&mut conn, &["CREATE TABLE a(x)"]).expect_err("downgrade");
        assert!(matches!(
            err,
            Error::SchemaTooNew {
                found: 2,
                supported: 1
            }
        ));
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store
            .conn
            .query_row(sql, [], |row| row.get(0))
            .expect("count query")
    }

    #[test]
    fn retention_rebuilds_engine_rows_from_the_raw_rows_left() {
        use crate::fixtures::{TempDb, WEEK_NOW, WEEK_START, load, local, week};

        let db = TempDb::new("retention-engine");
        let mut store = db.open().expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        let tables = ["activities", "tasks", "task_activities"];
        for table in tables {
            assert!(count(&store, &format!("SELECT count(*) FROM {table}")) > 0);
        }
        let before = count(&store, "SELECT count(*) FROM activities");

        let cutoff = WEEK_START + 2 * DAY;
        store
            .retention(local(cutoff + 30 * DAY), 30)
            .expect("retention");
        let newer = |sql: &str| count(&store, &format!("{sql} {cutoff}"));
        assert_eq!(newer("SELECT count(*) FROM events WHERE ts <"), 0);
        assert_eq!(newer("SELECT count(*) FROM activities WHERE end_ts <"), 0);
        assert_eq!(newer("SELECT count(*) FROM tasks WHERE last_ts <"), 0);
        assert_eq!(
            count(
                &store,
                "SELECT count(*) FROM task_activities WHERE activity_id NOT IN (SELECT id FROM activities)"
            ),
            0
        );
        let kept = count(&store, "SELECT count(*) FROM activities");
        assert!(kept > 0 && kept < before, "{kept} of {before} activities");

        store
            .retention(local(WEEK_NOW + 60 * DAY), 30)
            .expect("retention");
        for table in tables {
            assert_eq!(
                count(&store, &format!("SELECT count(*) FROM {table}")),
                0,
                "{table}"
            );
        }
    }
    #[test]
    fn forgetting_a_range_keeps_what_came_before_and_after() {
        use crate::fixtures::{TempDb, WEEK_NOW, WEEK_START, load, local, week};

        let db = TempDb::new("forget-range");
        let mut store = db.open().expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        let (from, to) = (WEEK_START + 2 * DAY, WEEK_START + 3 * DAY);
        let within = |store: &Store| {
            count(
                store,
                &format!("SELECT count(*) FROM events WHERE ts >= {from} AND ts < {to}"),
            )
        };
        let outside = |store: &Store| {
            count(
                store,
                &format!("SELECT count(*) FROM events WHERE ts < {from} OR ts >= {to}"),
            )
        };
        let kept = outside(&store);
        assert!(within(&store) > 0 && kept > 0, "test setup");

        let gone = store.forget(local(WEEK_NOW), from, to).expect("forget");

        assert!(gone.events > 0);
        assert_eq!(within(&store), 0);
        assert_eq!(outside(&store), kept);
        assert_eq!(
            count(
                &store,
                &format!("SELECT count(*) FROM frames WHERE ts >= {from} AND ts < {to}")
            ),
            0
        );
    }

    #[test]
    fn the_storage_cap_deletes_the_oldest_days_first() {
        use crate::fixtures::{TempDb, WEEK_NOW, load, local, week};

        let db = TempDb::new("storage-cap");
        let mut store = db.open().expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        let used = store.used_bytes().expect("used");
        let newest = count(&store, "SELECT MAX(ts) FROM events");

        assert_eq!(store.cap_storage(local(WEEK_NOW), used).expect("cap"), 0);
        let days = store.cap_storage(local(WEEK_NOW), used - 1).expect("cap");

        assert!(days >= 1);
        assert!(store.used_bytes().expect("used") < used);
        assert_eq!(count(&store, "SELECT MAX(ts) FROM events"), newest);
    }
}
