//! SQLCipher database: open with a key, versioned migrations, read-only readers, retention.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, params};

use crate::error::{Error, Result};
use crate::time::DAY;
use crate::writer::unpack;

const MIGRATIONS: &[&str] = &[include_str!("schema_v1.sql")];

pub type Key = [u8; 32];

pub struct Store {
    pub(crate) conn: Connection,
    path: PathBuf,
    key: Key,
    /// Fixed UTC offset used to cut local days. The app sets it from the OS at start.
    pub(crate) utc_offset_s: i32,
}

/// Rows removed by [`Store::retention`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Retained {
    pub frames: usize,
    pub utterances: usize,
    pub events: usize,
    pub lines: usize,
    pub windows: usize,
}

impl Store {
    /// Opens or creates the encrypted database and migrates it to the current schema.
    pub fn open(path: &Path, key: &Key) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        unlock(&conn, key)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn, MIGRATIONS)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            key: *key,
            utc_offset_s: 0,
        })
    }

    /// A read-only connection to the same file. WAL mode lets it run while the writer writes.
    pub fn reader(&self) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&self.path, flags)?;
        unlock(&conn, &self.key)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        let found = user_version(&conn)?;
        if found != MIGRATIONS.len() as i32 {
            return Err(Error::SchemaTooOld {
                found,
                expected: MIGRATIONS.len() as i32,
            });
        }
        Ok(Self {
            conn,
            path: self.path.clone(),
            key: self.key,
            utc_offset_s: self.utc_offset_s,
        })
    }

    pub fn set_utc_offset(&mut self, seconds: i32) {
        self.utc_offset_s = seconds;
    }

    pub fn schema_version(&self) -> Result<i32> {
        user_version(&self.conn)
    }

    /// Deletes raw rows older than `days`, then lines and windows nothing references (plan.md §5).
    pub fn retention(&mut self, now: i64, days: u32) -> Result<Retained> {
        let cutoff = now - i64::from(days) * DAY;
        let tx = self.conn.transaction()?;
        let mut gone = Retained::default();
        let mut line_ids = HashSet::new();
        let mut window_ids = HashSet::new();
        {
            let mut old = tx.prepare("SELECT id, window_id, packed FROM frames WHERE ts < ?1")?;
            let mut unlink =
                tx.prepare("DELETE FROM frame_lines WHERE line_id = ?1 AND frame_id = ?2")?;
            let mut rows = old.query(params![cutoff])?;
            while let Some(row) = rows.next()? {
                let (frame_id, window_id): (i64, i64) = (row.get(0)?, row.get(1)?);
                window_ids.insert(window_id);
                for (line_id, _) in unpack(&row.get::<_, Vec<u8>>(2)?) {
                    unlink.execute(params![line_id, frame_id])?;
                    line_ids.insert(line_id);
                }
            }
        }
        gone.frames = tx.execute("DELETE FROM frames WHERE ts < ?1", params![cutoff])?;
        gone.utterances =
            tx.execute("DELETE FROM utterances WHERE ts_end < ?1", params![cutoff])?;
        gone.events = tx.execute("DELETE FROM events WHERE ts < ?1", params![cutoff])?;
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
        Ok(gone)
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
}
