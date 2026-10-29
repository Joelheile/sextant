-- How much of each recurring step an agent could do, judged once per step key by the on-device
-- model. Refresh uses it in place of the category prior. Keys and reasons hold no raw text.
CREATE TABLE judgments (key TEXT PRIMARY KEY, share REAL NOT NULL, reason TEXT NOT NULL, ts INTEGER NOT NULL);
