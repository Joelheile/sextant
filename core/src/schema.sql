-- Sextant's database (plan.md §5), one schema for 0.1.0. Later changes append a migration.
--
-- Three groups of tables:
-- - Raw capture: what the Mac saw and heard. Retention deletes it by time.
-- - Derived: rebuilt from the raw capture by every refresh, so it never needs a migration of
--   its rows. A refresh deletes and refills all of it in one transaction.
-- - Yours: routines your agent saved, the sent log, the last agent run and meta. Refresh never
--   touches them.
--
-- Times are Unix seconds. Tables are STRICT, so a value of the wrong type fails the insert.

-- Raw capture ---------------------------------------------------------------------------------

-- Screen text, one row per distinct line. Frames point at lines, so text that stays on screen
-- is stored once.
CREATE TABLE lines (
    id INTEGER PRIMARY KEY,
    hash INTEGER NOT NULL UNIQUE,
    text TEXT NOT NULL
) STRICT;
CREATE VIRTUAL TABLE lines_fts USING fts5(
    text, content='lines', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER lines_ai AFTER INSERT ON lines BEGIN
    INSERT INTO lines_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER lines_ad AFTER DELETE ON lines BEGIN
    INSERT INTO lines_fts(lines_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TABLE windows (
    id INTEGER PRIMARY KEY,
    app TEXT NOT NULL,
    bundle_id TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    doc TEXT NOT NULL DEFAULT '',
    url_host_path TEXT NOT NULL DEFAULT '',
    UNIQUE (app, bundle_id, title, doc, url_host_path)
) STRICT;

-- One OCR or Accessibility read of a window. `packed` holds its line ids and boxes in order;
-- `frame_lines` is the reverse index that search and retention use.
CREATE TABLE frames (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    window_id INTEGER NOT NULL,
    packed BLOB NOT NULL
) STRICT;
CREATE INDEX frames_ts ON frames (ts);
CREATE INDEX frames_window ON frames (window_id);
CREATE TABLE frame_lines (
    line_id INTEGER NOT NULL,
    frame_id INTEGER NOT NULL,
    PRIMARY KEY (line_id, frame_id)
) STRICT, WITHOUT ROWID;

-- Call transcripts. Never audio.
CREATE TABLE utterances (
    id INTEGER PRIMARY KEY,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('mic', 'system')),
    text TEXT NOT NULL
) STRICT;
CREATE INDEX utterances_start ON utterances (start_ts);
CREATE VIRTUAL TABLE utterances_fts USING fts5(
    text, content='utterances', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER utterances_ai AFTER INSERT ON utterances BEGIN
    INSERT INTO utterances_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER utterances_ad AFTER DELETE ON utterances BEGIN
    INSERT INTO utterances_fts(utterances_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

-- Focus changes, idle and pause marks, and actions. The payload keys per kind are listed on
-- `Payload` in model.rs.
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('focus', 'idle', 'pause', 'press', 'edit')),
    json TEXT NOT NULL
) STRICT;
CREATE INDEX events_ts ON events (ts);

-- Derived -------------------------------------------------------------------------------------

-- Focus on one object, such as a document, a page or a window title.
CREATE TABLE activities (
    id INTEGER PRIMARY KEY,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    app TEXT NOT NULL,
    object_key TEXT NOT NULL,
    step_id INTEGER NOT NULL
) STRICT;
-- Pages and the timeline read activities by start time.
CREATE INDEX activities_start ON activities (start_ts);
-- The name of each kind of activity.
CREATE TABLE steps (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
) STRICT;
-- Activities joined by a shared identifier, such as an invoice number.
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    label TEXT NOT NULL,
    first_ts INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    active_s INTEGER NOT NULL
) STRICT;
CREATE TABLE task_activities (
    task_id INTEGER NOT NULL,
    activity_id INTEGER NOT NULL,
    PRIMARY KEY (task_id, activity_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX task_activities_activity ON task_activities (activity_id);

-- Yours ---------------------------------------------------------------------------------------

-- Routines your agent saved through MCP `save_routine`. Code measured the hours. The spans are
-- the activities the routine cites, so a routine outlives the 90-day window.
CREATE TABLE routines (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    title TEXT NOT NULL,
    how TEXT NOT NULL,
    trigger TEXT NOT NULL,
    approval TEXT NOT NULL,
    why TEXT NOT NULL,
    cites TEXT NOT NULL,
    hours_week REAL NOT NULL CHECK (hours_week >= 0)
) STRICT;
CREATE TABLE routine_spans (
    routine_id INTEGER NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL
) STRICT;
CREATE INDEX routine_spans_routine ON routine_spans (routine_id);

-- What left the database for an agent: one row per MCP answer and per bridge run, written
-- before the answer goes out. Sizes only, never the content.
CREATE TABLE sent (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    client TEXT NOT NULL,
    tool TEXT NOT NULL,
    bytes INTEGER NOT NULL
) STRICT;
CREATE INDEX sent_ts ON sent (ts);

-- The last agent bridge run. A new run replaces it.
CREATE TABLE agent_runs (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ts INTEGER NOT NULL,
    agent TEXT NOT NULL CHECK (agent IN ('claude', 'codex'))
) STRICT;

-- The email hash secret and the last vacuum time.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value ANY NOT NULL
) STRICT, WITHOUT ROWID;
