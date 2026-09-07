CREATE TABLE sessions (
    id             TEXT PRIMARY KEY NOT NULL,
    task_id        TEXT NOT NULL REFERENCES tasks(id),
    identity       TEXT NOT NULL,
    started_at     TEXT NOT NULL,
    ended_at       TEXT NOT NULL,
    outcome        TEXT NOT NULL
        CHECK (outcome IN ('succeeded', 'failed')),
    failure_reason TEXT,
    summary        TEXT NOT NULL DEFAULT '',
    decisions      TEXT NOT NULL DEFAULT '[]',
    artifacts      TEXT NOT NULL DEFAULT '[]',
    created_at     TEXT NOT NULL
) STRICT;
