-- A handoff keeps the workflow's name and step keys, so the measurement still works after
-- the workflow stops showing up because an agent took it over.
ALTER TABLE handoffs ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE handoffs ADD COLUMN sequence TEXT NOT NULL DEFAULT '[]';
