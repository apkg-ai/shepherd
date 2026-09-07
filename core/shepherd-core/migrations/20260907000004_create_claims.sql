CREATE TABLE claims (
    id             TEXT    PRIMARY KEY NOT NULL,
    task_id        TEXT    NOT NULL REFERENCES tasks(id),
    identity       TEXT    NOT NULL,
    ttl_seconds    INTEGER NOT NULL,
    lease_id       TEXT    NOT NULL UNIQUE,
    acquired_at    TEXT    NOT NULL,
    expires_at     TEXT    NOT NULL,
    renewed_at     TEXT,
    released_at    TEXT,
    release_reason TEXT
        CHECK (release_reason IS NULL OR
               release_reason IN ('voluntary', 'expired', 'session_reported', 'task_deleted'))
) STRICT;
