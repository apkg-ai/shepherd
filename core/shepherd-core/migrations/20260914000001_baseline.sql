-- Applied v1 baseline; kept line-equivalent to plan/contracts/schema.sql (baseline_migration_matches_contract_schema).

PRAGMA foreign_keys=ON;

PRAGMA application_id=1397248068;

CREATE TABLE schema_meta (version INTEGER NOT NULL, export_version TEXT NOT NULL) STRICT;

INSERT INTO schema_meta VALUES (1, '2.0.0');

CREATE TABLE command_lock (id INTEGER PRIMARY KEY CHECK(id=1), value INTEGER NOT NULL DEFAULT 0) STRICT;

INSERT INTO command_lock(id) VALUES(1);

CREATE TABLE actors (
  id TEXT PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('human','agent')),
  label TEXT NOT NULL,
  revoked INTEGER NOT NULL CHECK (revoked IN (0,1)),
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE projects (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT NOT NULL,
  settings TEXT NOT NULL CHECK (json_valid(settings)),
  archived INTEGER NOT NULL CHECK (archived IN (0,1)),
  archive_actor_id TEXT REFERENCES actors(id),
  archive_reason TEXT,
  archive_created_at TEXT,
  CHECK ((archive_actor_id IS NULL AND archive_reason IS NULL AND archive_created_at IS NULL) OR (archive_actor_id IS NOT NULL AND archive_reason IS NOT NULL AND length(trim(archive_reason))>0 AND archive_created_at IS NOT NULL)),
  CHECK ((archived=1) = (archive_actor_id IS NOT NULL))
) STRICT;

CREATE TABLE goals (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  archived INTEGER NOT NULL CHECK (archived IN (0,1)),
  archive_actor_id TEXT REFERENCES actors(id),
  archive_reason TEXT,
  archive_created_at TEXT,
  UNIQUE(id,project_id),
  CHECK ((archive_actor_id IS NULL AND archive_reason IS NULL AND archive_created_at IS NULL) OR (archive_actor_id IS NOT NULL AND archive_reason IS NOT NULL AND length(trim(archive_reason))>0 AND archive_created_at IS NOT NULL)),
  CHECK ((archived=1) = (archive_actor_id IS NOT NULL))
) STRICT;

CREATE TABLE epics (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  goal_id TEXT NOT NULL REFERENCES goals(id) DEFERRABLE INITIALLY DEFERRED,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('proposed','open','active','done','cancelled')),
  archived INTEGER NOT NULL CHECK (archived IN (0,1)),
  block_actor_id TEXT REFERENCES actors(id),
  block_reason TEXT,
  block_created_at TEXT,
  archive_actor_id TEXT REFERENCES actors(id),
  archive_reason TEXT,
  archive_created_at TEXT,
  cancellation_actor_id TEXT REFERENCES actors(id),
  cancellation_reason TEXT,
  cancellation_created_at TEXT,
  CHECK ((block_actor_id IS NULL AND block_reason IS NULL AND block_created_at IS NULL) OR (block_actor_id IS NOT NULL AND block_reason IS NOT NULL AND length(trim(block_reason))>0 AND block_created_at IS NOT NULL)),
  UNIQUE(id,project_id),
  FOREIGN KEY(goal_id,project_id) REFERENCES goals(id,project_id),
  CHECK ((archive_actor_id IS NULL AND archive_reason IS NULL AND archive_created_at IS NULL) OR (archive_actor_id IS NOT NULL AND archive_reason IS NOT NULL AND length(trim(archive_reason))>0 AND archive_created_at IS NOT NULL)),
  CHECK ((cancellation_actor_id IS NULL AND cancellation_reason IS NULL AND cancellation_created_at IS NULL) OR (cancellation_actor_id IS NOT NULL AND cancellation_reason IS NOT NULL AND length(trim(cancellation_reason))>0 AND cancellation_created_at IS NOT NULL)),
  CHECK ((archived=1) = (archive_actor_id IS NOT NULL)),
  CHECK ((status='cancelled') = (cancellation_actor_id IS NOT NULL))
) STRICT;

CREATE TABLE task_types (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  key TEXT NOT NULL,
  label TEXT NOT NULL,
  archived INTEGER NOT NULL CHECK (archived IN (0,1)),
  builtin INTEGER NOT NULL CHECK (builtin IN (0,1)),
  UNIQUE(id,project_id),
  UNIQUE(project_id,key)
) STRICT;

CREATE TABLE tasks (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  epic_id TEXT NOT NULL REFERENCES epics(id) DEFERRABLE INITIALLY DEFERRED,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  type_key TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('proposed','open','active','done','cancelled')),
  phase TEXT NOT NULL CHECK (phase IN ('planning','plan_review','execution','work_review','complete')),
  planning_required INTEGER NOT NULL CHECK (planning_required IN (0,1)),
  plan_review TEXT NOT NULL CHECK (plan_review IN ('human','agent','none')),
  work_review TEXT NOT NULL CHECK (work_review IN ('human','agent','none')),
  selected_plan_revision_id TEXT REFERENCES document_revisions(id) DEFERRABLE INITIALLY DEFERRED,
  accepted_plan_submission_id TEXT REFERENCES submissions(id) DEFERRABLE INITIALLY DEFERRED,
  archived INTEGER NOT NULL CHECK (archived IN (0,1)),
  block_actor_id TEXT REFERENCES actors(id),
  block_reason TEXT,
  block_created_at TEXT,
  waiver_actor_id TEXT REFERENCES actors(id),
  waiver_reason TEXT,
  waiver_created_at TEXT,
  attempt_count INTEGER NOT NULL,
  archive_actor_id TEXT REFERENCES actors(id),
  archive_reason TEXT,
  archive_created_at TEXT,
  cancellation_actor_id TEXT REFERENCES actors(id),
  cancellation_reason TEXT,
  cancellation_created_at TEXT,
  CHECK ((block_actor_id IS NULL AND block_reason IS NULL AND block_created_at IS NULL) OR (block_actor_id IS NOT NULL AND block_reason IS NOT NULL AND length(trim(block_reason))>0 AND block_created_at IS NOT NULL)),
  CHECK ((waiver_actor_id IS NULL AND waiver_reason IS NULL AND waiver_created_at IS NULL) OR (waiver_actor_id IS NOT NULL AND waiver_reason IS NOT NULL AND length(trim(waiver_reason))>0 AND waiver_created_at IS NOT NULL)),
  UNIQUE(id,project_id),
  FOREIGN KEY(epic_id,project_id) REFERENCES epics(id,project_id),
  FOREIGN KEY(project_id,type_key) REFERENCES task_types(project_id,key),
  CHECK ((status IN ('done','cancelled')) = (phase='complete')),
  CHECK (waiver_actor_id IS NULL OR status='cancelled'),
  CHECK ((archive_actor_id IS NULL AND archive_reason IS NULL AND archive_created_at IS NULL) OR (archive_actor_id IS NOT NULL AND archive_reason IS NOT NULL AND length(trim(archive_reason))>0 AND archive_created_at IS NOT NULL)),
  CHECK ((cancellation_actor_id IS NULL AND cancellation_reason IS NULL AND cancellation_created_at IS NULL) OR (cancellation_actor_id IS NOT NULL AND cancellation_reason IS NOT NULL AND length(trim(cancellation_reason))>0 AND cancellation_created_at IS NOT NULL)),
  CHECK ((archived=1) = (archive_actor_id IS NOT NULL)),
  CHECK ((status='cancelled') = (cancellation_actor_id IS NOT NULL))
) STRICT;

CREATE TABLE documents (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  owner_kind TEXT NOT NULL CHECK (owner_kind IN ('project','epic','task')),
  owner_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('brief','plan','finding','handoff','note','decision')),
  title TEXT NOT NULL,
  latest_revision_id TEXT NOT NULL REFERENCES document_revisions(id) DEFERRABLE INITIALLY DEFERRED,
  UNIQUE(id,project_id)
) STRICT;

CREATE TABLE document_revisions (
  id TEXT PRIMARY KEY NOT NULL,
  document_id TEXT NOT NULL REFERENCES documents(id) DEFERRABLE INITIALLY DEFERRED,
  number INTEGER NOT NULL CHECK (number >= 1),
  body TEXT NOT NULL,
  links TEXT NOT NULL CHECK (json_valid(links)),
  actor_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  created_at TEXT NOT NULL,
  UNIQUE(document_id,number)
) STRICT;

CREATE TABLE claims (
  id TEXT PRIMARY KEY NOT NULL,
  task_id TEXT NOT NULL REFERENCES tasks(id) DEFERRABLE INITIALLY DEFERRED,
  actor_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  phase TEXT NOT NULL CHECK (phase IN ('plan','execute','review')),
  submission_id TEXT REFERENCES submissions(id) DEFERRABLE INITIALLY DEFERRED,
  acquired_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('active','released','expired','revoked','reported')),
  task_revision INTEGER NOT NULL,
  plan_revision_id TEXT REFERENCES document_revisions(id) DEFERRABLE INITIALLY DEFERRED,
  lease_hash TEXT NOT NULL,
  closed_at TEXT,
  close_reason TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE sessions (
  id TEXT PRIMARY KEY NOT NULL,
  task_id TEXT NOT NULL REFERENCES tasks(id) DEFERRABLE INITIALLY DEFERRED,
  claim_id TEXT NOT NULL,
  actor_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  phase TEXT NOT NULL CHECK (phase IN ('plan','execute')),
  started_at TEXT NOT NULL,
  ended_at TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('succeeded','failed')),
  summary TEXT NOT NULL,
  failure_reason TEXT NOT NULL,
  document_revision_ids TEXT NOT NULL CHECK (json_valid(document_revision_ids)),
  links TEXT NOT NULL CHECK (json_valid(links))
) STRICT;

CREATE TABLE submissions (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  task_id TEXT NOT NULL REFERENCES tasks(id) DEFERRABLE INITIALLY DEFERRED,
  kind TEXT NOT NULL CHECK (kind IN ('plan','work')),
  producer_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  document_revision_ids TEXT NOT NULL CHECK (json_valid(document_revision_ids)),
  session_id TEXT NOT NULL REFERENCES sessions(id) DEFERRABLE INITIALLY DEFERRED,
  policy TEXT NOT NULL CHECK (policy IN ('human','agent','none')),
  status TEXT NOT NULL CHECK (status IN ('pending','accepted','rejected','withdrawn')),
  created_context_revision INTEGER NOT NULL,
  withdraw_reason TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE reviews (
  id TEXT PRIMARY KEY NOT NULL,
  submission_id TEXT NOT NULL REFERENCES submissions(id) DEFERRABLE INITIALLY DEFERRED,
  actor_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  decision TEXT NOT NULL CHECK (decision IN ('approve','reject')),
  reason TEXT NOT NULL,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE events (
  action TEXT NOT NULL,
  reason TEXT NOT NULL,
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT NOT NULL REFERENCES projects(id) DEFERRABLE INITIALLY DEFERRED,
  actor_id TEXT NOT NULL REFERENCES actors(id) DEFERRABLE INITIALLY DEFERRED,
  command_id TEXT NOT NULL,
  type TEXT NOT NULL CHECK (type IN ('project.changed','goal.changed','epic.changed','task.changed','dependency.changed','claim.changed','session.recorded','document.changed','submission.changed','review.recorded','task_type.changed')),
  resource_id TEXT NOT NULL,
  resource_revision INTEGER NOT NULL,
  occurred_at TEXT NOT NULL,
  affected_ids TEXT NOT NULL CHECK (json_valid(affected_ids))
) STRICT;

CREATE TABLE epic_dependencies (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK(revision>=1),
  project_id TEXT NOT NULL REFERENCES projects(id),
  dependent_id TEXT NOT NULL,
  prerequisite_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(dependent_id,prerequisite_id),
  CHECK(dependent_id != prerequisite_id),
  FOREIGN KEY(dependent_id,project_id) REFERENCES epics(id,project_id),
  FOREIGN KEY(prerequisite_id,project_id) REFERENCES epics(id,project_id)
) STRICT;

CREATE TABLE task_dependencies (
  id TEXT PRIMARY KEY NOT NULL,
  revision INTEGER NOT NULL CHECK(revision>=1),
  project_id TEXT NOT NULL REFERENCES projects(id),
  dependent_id TEXT NOT NULL,
  prerequisite_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(dependent_id,prerequisite_id),
  CHECK(dependent_id != prerequisite_id),
  FOREIGN KEY(dependent_id,project_id) REFERENCES tasks(id,project_id),
  FOREIGN KEY(prerequisite_id,project_id) REFERENCES tasks(id,project_id)
) STRICT;

CREATE TABLE credentials (
  id TEXT PRIMARY KEY NOT NULL, actor_id TEXT NOT NULL REFERENCES actors(id),
  token_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL, revoked_at TEXT
) STRICT;

CREATE TABLE browser_sessions (
  id TEXT PRIMARY KEY NOT NULL, actor_id TEXT NOT NULL REFERENCES actors(id),
  token_hash TEXT NOT NULL UNIQUE, csrf_hash TEXT NOT NULL, csrf_ciphertext BLOB NOT NULL, csrf_nonce BLOB NOT NULL,
  created_at TEXT NOT NULL, expires_at TEXT NOT NULL
) STRICT;

CREATE TABLE idempotency (
  actor_id TEXT NOT NULL REFERENCES actors(id), key TEXT NOT NULL,
  command_id TEXT NOT NULL, request_hash TEXT NOT NULL,
  response_status INTEGER NOT NULL, response_ciphertext BLOB NOT NULL,
  nonce BLOB NOT NULL, created_at TEXT NOT NULL, expires_at TEXT NOT NULL,
  PRIMARY KEY(actor_id,key)
) STRICT;

CREATE TABLE security_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT, actor_id TEXT NOT NULL REFERENCES actors(id),
  action TEXT NOT NULL, target_actor_id TEXT, reason TEXT NOT NULL,
  command_id TEXT NOT NULL, created_at TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX one_active_claim_per_task ON claims(task_id) WHERE status='active';

CREATE UNIQUE INDEX one_pending_submission_per_task ON submissions(task_id) WHERE status='pending';

CREATE UNIQUE INDEX one_review_per_submission ON reviews(submission_id);

CREATE INDEX goals_project ON goals(project_id,created_at,id);

CREATE INDEX epics_goal ON epics(goal_id,status,created_at,id);

CREATE INDEX epics_project ON epics(project_id,created_at,id);

CREATE INDEX tasks_epic ON tasks(epic_id,status,phase,created_at,id);

CREATE INDEX tasks_project ON tasks(project_id,status,phase,created_at,id);

CREATE INDEX claims_expiry ON claims(status,expires_at);

CREATE INDEX sessions_task ON sessions(task_id,ended_at,id);

CREATE INDEX documents_owner ON documents(project_id,owner_kind,owner_id,created_at,id);

CREATE INDEX submissions_queue ON submissions(status,policy,kind,created_at,id);

CREATE INDEX events_project ON events(project_id,id);

CREATE INDEX events_resource ON events(project_id,resource_id,id);

CREATE INDEX task_deps_reverse ON task_dependencies(prerequisite_id);

CREATE INDEX task_deps_project ON task_dependencies(project_id,created_at,id);

CREATE INDEX epic_deps_reverse ON epic_dependencies(prerequisite_id);

CREATE INDEX epic_deps_project ON epic_dependencies(project_id,created_at,id);
