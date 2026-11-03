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
