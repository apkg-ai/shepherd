CREATE TABLE knowledge_items (
    id         TEXT PRIMARY KEY NOT NULL,
    type       TEXT NOT NULL
        CHECK (type IN ('link', 'transcript', 'decision', 'note')),
    title      TEXT NOT NULL,
    content    TEXT NOT NULL,
    scope      TEXT NOT NULL
        CHECK (scope IN ('task', 'session', 'project')),
    task_id    TEXT REFERENCES tasks(id),
    session_id TEXT REFERENCES sessions(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    created_at TEXT NOT NULL
) STRICT;
