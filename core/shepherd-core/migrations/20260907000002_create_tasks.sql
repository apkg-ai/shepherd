CREATE TABLE tasks (
    id                  TEXT    PRIMARY KEY NOT NULL,
    project_id          TEXT    NOT NULL REFERENCES projects(id),
    title               TEXT    NOT NULL,
    description         TEXT    NOT NULL DEFAULT '',
    type                TEXT    NOT NULL
        CHECK (type IN ('code', 'question', 'refactor', 'review', 'research')),
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
