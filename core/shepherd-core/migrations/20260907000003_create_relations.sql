CREATE TABLE relations (
    id              TEXT PRIMARY KEY NOT NULL,
    type            TEXT NOT NULL
        CHECK (type IN ('decomposition', 'depends_on')),
    source_task_id  TEXT NOT NULL REFERENCES tasks(id),
    target_task_id  TEXT NOT NULL REFERENCES tasks(id),
    created_at      TEXT NOT NULL,
    UNIQUE(type, source_task_id, target_task_id),
    CHECK(source_task_id != target_task_id)
) STRICT;
