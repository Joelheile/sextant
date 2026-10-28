CREATE TABLE lines(
    id INTEGER PRIMARY KEY,
    hash INTEGER NOT NULL UNIQUE,
    text TEXT NOT NULL
);
CREATE VIRTUAL TABLE lines_fts USING fts5(
    text, content='lines', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER lines_ai AFTER INSERT ON lines BEGIN
    INSERT INTO lines_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER lines_ad AFTER DELETE ON lines BEGIN
    INSERT INTO lines_fts(lines_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TABLE windows(
    id INTEGER PRIMARY KEY,
    app TEXT NOT NULL,
    bundle_id TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    doc TEXT NOT NULL DEFAULT '',
    url_host_path TEXT NOT NULL DEFAULT '',
    UNIQUE(app, bundle_id, title, doc, url_host_path)
);
CREATE TABLE frames(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    window_id INTEGER NOT NULL,
    packed BLOB NOT NULL
);
CREATE INDEX frames_ts ON frames(ts);
CREATE TABLE frame_lines(
    line_id INTEGER NOT NULL,
    frame_id INTEGER NOT NULL,
    PRIMARY KEY(line_id, frame_id)
) WITHOUT ROWID;

CREATE TABLE utterances(
    id INTEGER PRIMARY KEY,
    ts_start INTEGER NOT NULL,
    ts_end INTEGER NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('mic', 'system')),
    text TEXT NOT NULL
);
CREATE INDEX utterances_ts ON utterances(ts_start);
CREATE VIRTUAL TABLE utterances_fts USING fts5(
    text, content='utterances', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER utterances_ai AFTER INSERT ON utterances BEGIN
    INSERT INTO utterances_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER utterances_ad AFTER DELETE ON utterances BEGIN
    INSERT INTO utterances_fts(utterances_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TABLE events(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('live', 'backfill')),
    json TEXT NOT NULL
);
CREATE INDEX events_ts ON events(ts);

-- Engine tables are rebuilt by refresh(), except overrides, corrections and handoffs.
CREATE TABLE episodes(
    id INTEGER PRIMARY KEY,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    app TEXT NOT NULL,
    object_key TEXT NOT NULL,
    step_id INTEGER NOT NULL,
    live INTEGER NOT NULL
);
CREATE TABLE steps(
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    episodes INTEGER NOT NULL,
    minutes REAL NOT NULL,
    days INTEGER NOT NULL,
    potential REAL NOT NULL,
    reasons TEXT NOT NULL
);
CREATE TABLE entities(
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    episodes INTEGER NOT NULL,
    UNIQUE(kind, value)
);
CREATE TABLE mentions(
    entity_id INTEGER NOT NULL,
    episode_id INTEGER NOT NULL,
    PRIMARY KEY(entity_id, episode_id)
) WITHOUT ROWID;
CREATE TABLE follows(
    from_step INTEGER NOT NULL,
    to_step INTEGER NOT NULL,
    count INTEGER NOT NULL,
    median_gap_s INTEGER NOT NULL,
    copies INTEGER NOT NULL,
    days INTEGER NOT NULL,
    PRIMARY KEY(from_step, to_step)
) WITHOUT ROWID;
CREATE TABLE cases(
    entity_id INTEGER PRIMARY KEY,
    first_ts INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    variant TEXT
);
CREATE TABLE workflows(
    id INTEGER PRIMARY KEY,
    variant TEXT NOT NULL UNIQUE,
    basis TEXT NOT NULL,
    name TEXT NOT NULL,
    sequence TEXT NOT NULL,
    case_count INTEGER NOT NULL,
    hours_week REAL NOT NULL,
    potential REAL NOT NULL,
    desire REAL NOT NULL,
    benefit REAL NOT NULL,
    procrastination REAL NOT NULL,
    kind TEXT NOT NULL,
    action TEXT NOT NULL,
    first_step TEXT NOT NULL,
    copies TEXT NOT NULL,
    enjoyment TEXT NOT NULL,
    client INTEGER NOT NULL,
    hidden INTEGER NOT NULL,
    rough INTEGER NOT NULL,
    reasons TEXT NOT NULL
);
CREATE TABLE commitments(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    source TEXT NOT NULL,
    text TEXT NOT NULL,
    due_ts INTEGER,
    entity_id INTEGER,
    overdue INTEGER NOT NULL
);
CREATE TABLE overrides(
    key TEXT NOT NULL,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(key, field)
) WITHOUT ROWID;
CREATE TABLE corrections(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    phrase TEXT NOT NULL,
    rule TEXT NOT NULL,
    key TEXT NOT NULL,
    arg TEXT NOT NULL
);
CREATE TABLE handoffs(
    id INTEGER PRIMARY KEY,
    workflow_id INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    status TEXT NOT NULL,
    note TEXT NOT NULL
);
