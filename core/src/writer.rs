//! Passive writer: the app pushes records and calls `flush` about once a second.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Transaction, params};

use crate::error::Result;
use crate::model::{Event, Frame, Item, Source, Utterance, Window};
use crate::redact::{Secret, redact, redact_json};
use crate::store::Store;
use crate::text::{fnv1a, host_path};

/// Counts for logs. Never text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flushed {
    pub events: usize,
    pub frames: usize,
    /// Frames identical to the last frame of the same window.
    pub duplicate_frames: usize,
    pub utterances: usize,
    pub new_lines: usize,
}

#[derive(Default)]
pub struct Writer {
    pending: Vec<Item>,
    last_lines: HashMap<Window, Vec<i64>>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, item: Item) {
        self.pending.push(item);
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Redacts, dedupes and writes everything pushed so far in one transaction.
    /// Live timestamps after `now` are clamped to `now`. On error the records stay queued.
    pub fn flush(&mut self, store: &mut Store, now: i64) -> Result<Flushed> {
        let mut flushed = Flushed::default();
        if self.pending.is_empty() {
            return Ok(flushed);
        }
        let secret = store.secret;
        let mut last_lines = self.last_lines.clone();
        let tx = store.conn.transaction()?;
        let mut windows = HashMap::new();
        for item in &self.pending {
            match item {
                Item::Event(event) => {
                    write_event(&tx, event, now, &secret)?;
                    flushed.events += 1;
                }
                Item::Utterance(utterance) => {
                    write_utterance(&tx, utterance, now, &secret)?;
                    flushed.utterances += 1;
                }
                Item::Frame(frame) => {
                    write_frame(
                        &tx,
                        frame,
                        now,
                        &secret,
                        &mut windows,
                        &mut last_lines,
                        &mut flushed,
                    )?;
                }
            }
        }
        tx.commit()?;
        if last_lines.len() > 1024 {
            last_lines.clear();
        }
        self.last_lines = last_lines;
        self.pending.clear();
        Ok(flushed)
    }
}

/// Live events cannot be from the future. Backfill bounds its own range, and its calendar and
/// reminders reach 14 days ahead.
fn stamp(event: &Event, now: i64) -> i64 {
    match event.source {
        Source::Live => event.ts.min(now),
        Source::Backfill => event.ts,
    }
}

fn write_event(tx: &Transaction, event: &Event, now: i64, secret: &Secret) -> Result<()> {
    let mut json = event.json.clone();
    redact_json(&mut json, secret);
    tx.prepare_cached("INSERT INTO events(ts, kind, source, json) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![
            stamp(event, now),
            event.kind.as_str(),
            event.source.as_str(),
            json.to_string()
        ])?;
    Ok(())
}

fn write_utterance(
    tx: &Transaction,
    utterance: &Utterance,
    now: i64,
    secret: &Secret,
) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO utterances(start_ts, end_ts, source, text) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![
        utterance.ts_start.min(now),
        utterance.ts_end.min(now),
        utterance.source.as_str(),
        redact(utterance.text.trim(), secret)
    ])?;
    Ok(())
}

fn write_frame(
    tx: &Transaction,
    frame: &Frame,
    now: i64,
    secret: &Secret,
    windows: &mut HashMap<Window, i64>,
    last_lines: &mut HashMap<Window, Vec<i64>>,
    flushed: &mut Flushed,
) -> Result<()> {
    let window = clean_window(&frame.window, secret);
    let mut entries = Vec::with_capacity(frame.lines.len());
    for line in &frame.lines {
        let text = redact(line.text.trim(), secret);
        if !text.is_empty() {
            entries.push((line_id(tx, &text, flushed)?, line.bbox));
        }
    }
    if entries.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = entries.iter().map(|(id, _)| *id).collect();
    if last_lines.get(&window) == Some(&ids) {
        flushed.duplicate_frames += 1;
        return Ok(());
    }
    let window_id = match windows.get(&window) {
        Some(id) => *id,
        None => {
            let id = window_id(tx, &window)?;
            windows.insert(window.clone(), id);
            id
        }
    };
    tx.prepare_cached("INSERT INTO frames(ts, window_id, packed) VALUES (?1, ?2, ?3)")?
        .execute(params![frame.ts.min(now), window_id, pack(&entries)])?;
    let frame_id = tx.last_insert_rowid();
    let mut link =
        tx.prepare_cached("INSERT OR IGNORE INTO frame_lines(line_id, frame_id) VALUES (?1, ?2)")?;
    for id in &ids {
        link.execute(params![id, frame_id])?;
    }
    last_lines.insert(window, ids);
    flushed.frames += 1;
    Ok(())
}

fn clean_window(window: &Window, secret: &Secret) -> Window {
    Window {
        app: redact(&window.app, secret),
        bundle_id: window.bundle_id.clone(),
        title: redact(&window.title, secret),
        doc: redact(&window.doc, secret),
        url: redact(&host_path(&window.url), secret),
    }
}

fn window_id(tx: &Transaction, window: &Window) -> Result<i64> {
    let key = params![
        window.app,
        window.bundle_id,
        window.title,
        window.doc,
        window.url
    ];
    let found = tx
        .prepare_cached(
            "SELECT id FROM windows WHERE app = ?1 AND bundle_id = ?2 AND title = ?3 AND doc = ?4 AND url_host_path = ?5",
        )?
        .query_row(key, |row| row.get(0))
        .optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    tx.prepare_cached("INSERT INTO windows(app, bundle_id, title, doc, url_host_path) VALUES (?1, ?2, ?3, ?4, ?5)")?
        .execute(key)?;
    Ok(tx.last_insert_rowid())
}

fn line_id(tx: &Transaction, text: &str, flushed: &mut Flushed) -> Result<i64> {
    let hash = fnv1a(text) as i64;
    let found = tx
        .prepare_cached("SELECT id FROM lines WHERE hash = ?1")?
        .query_row(params![hash], |row| row.get(0))
        .optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    tx.prepare_cached("INSERT INTO lines(hash, text) VALUES (?1, ?2)")?
        .execute(params![hash, text])?;
    flushed.new_lines += 1;
    Ok(tx.last_insert_rowid())
}

/// Varint line id followed by the four box bytes, per line.
pub(crate) fn pack(entries: &[(i64, [u8; 4])]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * 7);
    for (id, bbox) in entries {
        let mut value = *id as u64;
        while value >= 0x80 {
            out.push((value & 0x7f) as u8 | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
        out.extend_from_slice(bbox);
    }
    out
}

pub(crate) fn unpack(bytes: &[u8]) -> Vec<(i64, [u8; 4])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let (mut value, mut shift) = (0u64, 0);
        while at < bytes.len() {
            let byte = bytes[at];
            at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let Some(bbox) = bytes.get(at..at + 4) else {
            break;
        };
        out.push((value as i64, [bbox[0], bbox[1], bbox[2], bbox[3]]));
        at += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TempDb;

    #[test]
    fn packed_frames_round_trip() {
        let entries = vec![
            (1, [0, 1, 2, 3]),
            (300, [255, 0, 9, 9]),
            (5_000_000_000, [4, 4, 4, 4]),
        ];
        assert_eq!(unpack(&pack(&entries)), entries);
    }

    #[test]
    fn an_utterance_is_seconds_and_text_never_audio() {
        let db = TempDb::new("utterance-schema");
        let store = db.open().expect("store");
        let columns: Vec<(String, String)> = store
            .conn
            .prepare("SELECT name, type FROM pragma_table_info('utterances') ORDER BY cid")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<std::result::Result<_, _>>()
            .expect("rows");
        let columns: Vec<(&str, &str)> = columns
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        assert_eq!(
            columns,
            [
                ("id", "INTEGER"),
                ("start_ts", "INTEGER"),
                ("end_ts", "INTEGER"),
                ("source", "TEXT"),
                ("text", "TEXT"),
            ]
        );
    }
}
