-- Routines replace the workflow finder (routines-plan.md E4b). Steps stay, because they name
-- activities; refresh rebuilds them, so they are recreated empty without the scores.
DROP TABLE workflows;
DROP TABLE follows;
DROP TABLE commitments;
DROP TABLE overrides;
DROP TABLE corrections;
DROP TABLE handoffs;
DROP TABLE judgments;
DROP TABLE agent_ideas;
DROP TABLE steps;
CREATE TABLE steps(
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);
DROP INDEX tasks_workflow;
ALTER TABLE tasks DROP COLUMN workflow_id;
-- Episodes are called activities everywhere now, as on the MCP pages.
ALTER TABLE episodes RENAME TO activities;
DROP INDEX episodes_end;
CREATE INDEX activities_end ON activities(end_ts);
ALTER TABLE task_episodes RENAME TO task_activities;
ALTER TABLE task_activities RENAME COLUMN episode_id TO activity_id;
DROP INDEX task_episodes_episode;
CREATE INDEX task_activities_activity ON task_activities(activity_id);
ALTER TABLE mentions RENAME COLUMN episode_id TO activity_id;
ALTER TABLE entities RENAME COLUMN episode_count TO activity_count;
