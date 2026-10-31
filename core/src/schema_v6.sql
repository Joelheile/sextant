-- Tasks replace cases: episodes joined in time order by shared identifiers. Refresh fills
-- `workflow_id`, so a workflow's tasks are an index seek.
DROP TABLE cases;
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    label TEXT NOT NULL,
    workflow_id INTEGER,
    first_ts INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    active_s INTEGER NOT NULL
);
CREATE INDEX tasks_workflow ON tasks(workflow_id);
CREATE TABLE task_episodes (
    task_id INTEGER NOT NULL,
    episode_id INTEGER NOT NULL,
    PRIMARY KEY (task_id, episode_id)
);
CREATE INDEX task_episodes_episode ON task_episodes(episode_id);

-- Recreated, not renamed: the old `entity_id` values would read as task ids until the next refresh.
DROP TABLE commitments;
CREATE TABLE commitments(
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    source TEXT NOT NULL,
    text TEXT NOT NULL,
    due_ts INTEGER,
    task_id INTEGER,
    overdue INTEGER NOT NULL
);

-- One naming scheme: `start_ts`/`end_ts` for spans, `_count` for counts, `_id` for ids.
ALTER TABLE utterances RENAME COLUMN ts_start TO start_ts;
ALTER TABLE utterances RENAME COLUMN ts_end TO end_ts;
ALTER TABLE steps RENAME COLUMN episodes TO episode_count;
ALTER TABLE entities RENAME COLUMN episodes TO episode_count;
ALTER TABLE workflows RENAME COLUMN case_count TO task_count;
ALTER TABLE follows RENAME COLUMN from_step TO from_step_id;
ALTER TABLE follows RENAME COLUMN to_step TO to_step_id;

-- Retention deletes windows no frame uses and episodes past the window.
CREATE INDEX frames_window ON frames(window_id);
CREATE INDEX episodes_end ON episodes(end_ts);

-- The last agent bridge run (your own Claude Code or Codex) and the automation ideas it found.
-- One run is kept; a new run replaces it.
CREATE TABLE agent_runs (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ts INTEGER NOT NULL,
    agent TEXT NOT NULL CHECK (agent IN ('claude', 'codex'))
);
CREATE TABLE agent_ideas (
    id INTEGER PRIMARY KEY,
    workflow_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    how TEXT NOT NULL,
    hours_week REAL NOT NULL CHECK (hours_week >= 0),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1)
);
CREATE INDEX agent_ideas_workflow ON agent_ideas(workflow_id);
