-- Routines your agent saved through MCP `save_routine` (routines-plan.md E2). Code measured the
-- hours. The spans are the activities the routine cites, so it outlives the 90-day window.
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
);
CREATE TABLE routine_spans (
    routine_id INTEGER NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL
);
CREATE INDEX routine_spans_routine ON routine_spans(routine_id);
