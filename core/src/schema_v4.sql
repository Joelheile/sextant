-- Rewind search history: each question and its answer, so the search panel can scroll back.
-- Both are redacted before they are stored, and retention deletes them like raw data.
CREATE TABLE turns (
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  question TEXT NOT NULL,
  answer TEXT NOT NULL
);
CREATE INDEX turns_ts ON turns (ts);
