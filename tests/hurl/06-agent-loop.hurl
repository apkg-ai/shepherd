# The full agent loop from docs/agent-guide.md, driven over the wire:
# next-task → claim → context → renew → report — plus the failure path
# (task returns to ready with attempt history) and voluntary release.

# Setup: a project with the review gate on (default).
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "hurl-agent-loop" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

# Two tasks; the second depends on the first.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{
  "title": "Design the schema",
  "type": "code",
  "status": "approved"
}
HTTP 201
[Asserts]
jsonpath "$.status" == "ready"
[Captures]
tid_a: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{
  "title": "Implement the API",
  "type": "code",
  "status": "approved"
}
HTTP 201
[Captures]
tid_b: jsonpath "$.id"

# B depends_on A → B is demoted until A is done.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{tid_a}}" }
HTTP 201

# 1. Find work: next-task offers A.
GET {{base_url}}/api/v1/projects/{{pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task.id" == "{{tid_a}}"
jsonpath "$.task.status" == "ready"

# 2. Claim it with identity + TTL.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-1",
    "label": "agent loop e2e"
  },
  "ttl_seconds": 600
}
HTTP 201
[Asserts]
jsonpath "$.task_id" == "{{tid_a}}"
jsonpath "$.lease_id" isString
jsonpath "$.ttl_seconds" == 600
jsonpath "$.renewed_at" not exists

# A concurrent second claim conflicts.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "cursor",
    "agent_model": "other",
    "session_id": "hurl-sess-2"
  },
  "ttl_seconds": 600
}
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:claim-conflict"

# 3. Fetch the context bundle.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/context
HTTP 200
[Asserts]
jsonpath "$.task.id" == "{{tid_a}}"
jsonpath "$.task.status" == "in_progress"
jsonpath "$.ancestor_summaries" count == 0
jsonpath "$.sibling_tasks" count == 0

# Renew the lease mid-work.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/claim/renew
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-1"
  },
  "ttl_seconds": 900
}
HTTP 200
[Asserts]
jsonpath "$.ttl_seconds" == 900
jsonpath "$.renewed_at" isString

# Renewing with a different identity conflicts.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/claim/renew
Content-Type: application/json
{
  "identity": {
    "harness": "cursor",
    "agent_model": "other",
    "session_id": "hurl-sess-2"
  }
}
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:claim-conflict"

# 5. Report a successful session with decisions, knowledge, artifacts.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/sessions
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-1"
  },
  "started_at": "2026-09-10T10:00:00Z",
  "ended_at": "2026-09-10T10:45:00Z",
  "outcome": "succeeded",
  "summary": "Designed the schema with all entities.",
  "decisions": ["Third normal form throughout."],
  "knowledge_items": [
    {
      "type": "decision",
      "title": "Schema normalization",
      "content": "Tables are third normal form; no JSON blobs for relations."
    }
  ],
  "artifacts": ["https://example.com/pr/1"]
}
HTTP 201
[Asserts]
jsonpath "$.outcome" == "succeeded"
jsonpath "$.decisions" count == 1
jsonpath "$.knowledge_items" count == 1
jsonpath "$.artifacts[0]" == "https://example.com/pr/1"
[Captures]
sid_a: jsonpath "$.id"

# Review gate on → A is in review; the session is on record.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}
HTTP 200
[Asserts]
jsonpath "$.status" == "in_review"

# Session-produced knowledge is addressable project-wide, by task.
GET {{base_url}}/api/v1/projects/{{pid}}/knowledge?task_id={{tid_a}}
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].title" == "Schema normalization"
jsonpath "$.items[0].scope" == "project"

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/sessions/{{sid_a}}
HTTP 200
[Asserts]
jsonpath "$.summary" == "Designed the schema with all entities."

# Human approves the review → A done, B auto-readies.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_a}}/approve
Content-Type: application/json
{}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

GET {{base_url}}/api/v1/projects/{{pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task.id" == "{{tid_b}}"

# Session 2 sees session 1's output in the context bundle.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-3"
  },
  "ttl_seconds": 600
}
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/context
HTTP 200
[Asserts]
jsonpath "$.ancestor_summaries" count == 1
jsonpath "$.ancestor_summaries[0].task_id" == "{{tid_a}}"
jsonpath "$.ancestor_summaries[0].decisions[0]" == "Third normal form throughout."
jsonpath "$.artifacts[0]" == "https://example.com/pr/1"

# The failure path: B's session fails → back to ready, attempt recorded.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/sessions
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-3"
  },
  "started_at": "2026-09-10T11:00:00Z",
  "ended_at": "2026-09-10T11:20:00Z",
  "outcome": "failed",
  "failure_reason": "integration tests kept failing",
  "summary": "Attempted the API wiring; blocked on test failures."
}
HTTP 201
[Asserts]
jsonpath "$.outcome" == "failed"
jsonpath "$.failure_reason" == "integration tests kept failing"

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"
jsonpath "$.attempt_count" == 1

# Voluntary release: claim again, then give up without reporting.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-4"
  },
  "ttl_seconds": 600
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/claim/release
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-4"
  }
}
HTTP 204

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Renewing a lease that no longer exists → 410 Gone.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_b}}/claim/renew
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-4"
  }
}
HTTP 410
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:lease-expired"
