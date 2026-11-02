-- What left the database for an agent (plan.md §5): one row per MCP tool call and per agent
-- bridge run, written before the answer goes out. Sizes only, never the content.
CREATE TABLE sent (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    client TEXT NOT NULL,
    tool TEXT NOT NULL,
    bytes INTEGER NOT NULL
);
CREATE INDEX sent_ts ON sent (ts);
