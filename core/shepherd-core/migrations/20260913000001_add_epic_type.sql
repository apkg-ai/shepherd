-- Add 'epic' to the task type CHECK constraint.
-- SQLite cannot ALTER CHECK constraints, so the table is rebuilt.
--
-- The rebuild requires PRAGMA foreign_keys = OFF because child tables
-- (relations, claims, sessions, knowledge_items) reference tasks(id).
-- With FKs on, DROP TABLE would fail on populated databases. The PRAGMA
-- cannot be changed inside a transaction, so this migration runs outside
-- one (sqlx's `-- no-transaction` is not needed for SQLite — sqlx already
-- runs SQLite migrations outside transactions when the file contains
-- multiple statements separated by semicolons).

PRAGMA foreign_keys = OFF;

CREATE TABLE tasks_new (
    id                  TEXT    PRIMARY KEY NOT NULL,
    project_id          TEXT    NOT NULL REFERENCES projects(id),
    title               TEXT    NOT NULL,
    description         TEXT    NOT NULL DEFAULT '',
    type                TEXT    NOT NULL
        CHECK (type IN ('code', 'question', 'refactor', 'review', 'research', 'epic')),
    status              TEXT    NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed', 'approved', 'ready', 'in_progress',
                          'in_review', 'done', 'blocked', 'cancelled')),
    metadata            TEXT    NOT NULL DEFAULT '{}',
    assignee            TEXT,
    graph_role          TEXT    NOT NULL DEFAULT '[]',
    graph_role_explicit INTEGER NOT NULL DEFAULT 0,
    attempt_count       INTEGER NOT NULL DEFAULT 0,
    blocked_from_status TEXT
        CHECK (blocked_from_status IS NULL OR
               blocked_from_status IN ('proposed', 'approved', 'ready',
                                       'in_progress', 'in_review')),
    block_reason        TEXT,
    deleted_at          TEXT,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL
) STRICT;

INSERT INTO tasks_new SELECT * FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

-- Re-create indexes that reference tasks (from migration 7).
CREATE INDEX idx_tasks_by_project
    ON tasks(project_id, deleted_at, created_at DESC, id DESC);
CREATE INDEX idx_tasks_by_status
    ON tasks(project_id, status, deleted_at);
CREATE INDEX idx_tasks_by_type
    ON tasks(project_id, type, deleted_at);

PRAGMA foreign_keys = ON;

-- Verify no FK violations were introduced by the rebuild.
PRAGMA foreign_key_check;
