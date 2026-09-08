-- Project listing (soft-delete filter + cursor pagination).
CREATE INDEX idx_projects_listing
    ON projects(deleted_at, created_at DESC, id DESC);

-- Task listing by project (soft-delete, status/type filter, pagination).
CREATE INDEX idx_tasks_by_project
    ON tasks(project_id, deleted_at, created_at DESC, id DESC);
CREATE INDEX idx_tasks_by_status
    ON tasks(project_id, status, deleted_at);
CREATE INDEX idx_tasks_by_type
    ON tasks(project_id, type, deleted_at);

-- Relation graph traversal.
CREATE INDEX idx_relations_source
    ON relations(source_task_id, type);
CREATE INDEX idx_relations_target
    ON relations(target_task_id, type);

-- Active claim lookup (one per task).
CREATE INDEX idx_claims_active
    ON claims(task_id, released_at, expires_at);

-- Expired claim sweep.
CREATE INDEX idx_claims_expiry
    ON claims(released_at, expires_at)
    WHERE released_at IS NULL;

-- Session listing by task.
CREATE INDEX idx_sessions_by_task
    ON sessions(task_id, created_at DESC, id DESC);

-- Knowledge listing by project/scope/task.
CREATE INDEX idx_knowledge_by_project
    ON knowledge_items(project_id, scope, created_at DESC, id DESC);
CREATE INDEX idx_knowledge_by_task
    ON knowledge_items(task_id, created_at DESC, id DESC)
    WHERE task_id IS NOT NULL;
CREATE INDEX idx_knowledge_by_session
    ON knowledge_items(session_id)
    WHERE session_id IS NOT NULL;
