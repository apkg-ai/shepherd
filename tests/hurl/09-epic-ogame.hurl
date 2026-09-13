# OGame: partial-completion scenario using all six task types.
#
# Structure:
#   Resource System (epic, done — all subtasks complete)
#     ├── Metal mine (code, done)
#     ├── Crystal mine (code, done)
#     └── Deuterium synthesis study (research, done)
#   Buildings & Tech Tree (epic, in progress — partially done)
#     ├── Building queue system (code, done)
#     ├── Tech tree data model (refactor, done)
#     ├── Alliance tech decision (question, approved — waiting)
#     └── Defense System (nested epic, ready — waiting on own subtasks)
#         ├── Rocket launcher (code, ready)
#         ├── Light laser (code, ready)
#         └── Defense balance review (review, ready)
#   Fleet & Combat (epic, depends_on Buildings — approved/blocked)
#     ├── Ship builder (code, approved)
#     ├── Fleet dispatch (code, approved)
#     ├── Combat formula (research, approved)
#     └── Combat balance review (review, approved)
#   Galaxy View (epic, depends_on Resource System — ready since done)
#     ├── Galaxy map renderer (code, in_progress — claimed)
#     ├── Coordinate system choice (question, ready)
#     └── Planet rendering pipeline (refactor, ready)

# ── Setup ────────────────────────────────────────────────────────────

POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "OGame" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

PATCH {{base_url}}/api/v1/projects/{{pid}}
Content-Type: application/json
{ "settings": { "review_gate": false } }
HTTP 200

# ── Epics ────────────────────────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Resource System", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
resources: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Buildings & Tech Tree", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
buildings: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Defense System", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
defense: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Fleet & Combat", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
fleet: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Galaxy View", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
galaxy: jsonpath "$.id"

# ── Resource System subtasks (all will be completed) ─────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Metal mine implementation", "type": "code", "status": "approved" }
HTTP 201
[Captures]
metal_mine: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Crystal mine implementation", "type": "code", "status": "approved" }
HTTP 201
[Captures]
crystal_mine: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Deuterium synthesis study", "type": "research", "status": "approved" }
HTTP 201
[Captures]
deuterium: jsonpath "$.id"

# ── Buildings & Tech Tree subtasks ───────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Building queue system", "type": "code", "status": "approved" }
HTTP 201
[Captures]
build_queue: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Tech tree data model", "type": "refactor", "status": "approved" }
HTTP 201
[Captures]
tech_tree: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Should we support alliance tech?", "type": "question", "status": "approved" }
HTTP 201
[Captures]
alliance_q: jsonpath "$.id"

# ── Defense System subtasks ──────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Rocket launcher", "type": "code", "status": "approved" }
HTTP 201
[Captures]
rocket_launcher: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Light laser turret", "type": "code", "status": "approved" }
HTTP 201
[Captures]
light_laser: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Defense balance review", "type": "review", "status": "approved" }
HTTP 201
[Captures]
defense_review: jsonpath "$.id"

# ── Fleet & Combat subtasks ──────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Ship builder", "type": "code", "status": "approved" }
HTTP 201
[Captures]
ship_builder: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Fleet dispatch system", "type": "code", "status": "approved" }
HTTP 201
[Captures]
fleet_dispatch: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Combat formula research", "type": "research", "status": "approved" }
HTTP 201
[Captures]
combat_research: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Combat balance review", "type": "review", "status": "approved" }
HTTP 201
[Captures]
combat_review: jsonpath "$.id"

# ── Galaxy View subtasks ─────────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Galaxy map renderer", "type": "code", "status": "approved" }
HTTP 201
[Captures]
galaxy_renderer: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Coordinate system choice", "type": "question", "status": "approved" }
HTTP 201
[Captures]
coord_q: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Planet rendering pipeline", "type": "refactor", "status": "approved" }
HTTP 201
[Captures]
planet_render: jsonpath "$.id"

# ── Wire decomposition ───────────────────────────────────────────────

# Resource System → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{resources}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{metal_mine}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{resources}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{crystal_mine}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{resources}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{deuterium}}" }
HTTP 201

# Buildings → subtasks + Defense epic
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{build_queue}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{tech_tree}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{alliance_q}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{defense}}" }
HTTP 201

# Defense → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{defense}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{rocket_launcher}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{defense}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{light_laser}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{defense}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{defense_review}}" }
HTTP 201

# Fleet → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{ship_builder}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{fleet_dispatch}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{combat_research}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{combat_review}}" }
HTTP 201

# Galaxy View → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{galaxy_renderer}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{coord_q}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{planet_render}}" }
HTTP 201

# ── Wire depends_on between epics ────────────────────────────────────

# Fleet depends_on Buildings (Fleet blocked until Buildings done)
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{buildings}}" }
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# Galaxy View depends_on Resource System
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{resources}}" }
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# ── Complete Resource System (auto-completes → Galaxy View unblocks) ─

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{metal_mine}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"og-01"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{metal_mine}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"og-01"},
  "started_at": "2026-09-13T10:00:00Z", "ended_at": "2026-09-13T10:30:00Z",
  "outcome": "succeeded", "summary": "Metal mine with 10 upgrade levels."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{crystal_mine}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"og-02"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{crystal_mine}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"og-02"},
  "started_at": "2026-09-13T10:30:00Z", "ended_at": "2026-09-13T11:00:00Z",
  "outcome": "succeeded", "summary": "Crystal mine with diminishing returns."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{deuterium}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"og-03"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{deuterium}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"og-03"},
  "started_at": "2026-09-13T11:00:00Z", "ended_at": "2026-09-13T11:30:00Z",
  "outcome": "succeeded", "summary": "Deuterium synthesis formula validated."
}
HTTP 201

# ★ Resource System auto-completed → Galaxy View auto-readied
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{resources}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# ── Partially complete Buildings (2 of 4 children done) ──────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{build_queue}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"og-04"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{build_queue}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"og-04"},
  "started_at": "2026-09-13T12:00:00Z", "ended_at": "2026-09-13T12:30:00Z",
  "outcome": "succeeded", "summary": "FIFO build queue with cancel support."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tech_tree}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"og-05"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tech_tree}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"og-05"},
  "started_at": "2026-09-13T12:30:00Z", "ended_at": "2026-09-13T13:00:00Z",
  "outcome": "succeeded", "summary": "Refactored tech tree into DAG with prerequisite edges."
}
HTTP 201

# Buildings NOT done — alliance_q and defense epic still pending.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Fleet still approved — Buildings not done yet.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# ── Put Galaxy map renderer in progress (claimed) ────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_renderer}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "og-active-1",
    "label": "rendering the galaxy"
  },
  "ttl_seconds": 3600
}
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_renderer}}
HTTP 200
[Asserts]
jsonpath "$.status" == "in_progress"

# ── Block the alliance tech question ─────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{alliance_q}}/block
Content-Type: application/json
{ "reason": "Waiting for product decision on alliance features scope" }
HTTP 200
[Asserts]
jsonpath "$.status" == "blocked"

# ── Final state check: variety of statuses ───────────────────────────

# Resource System: done (auto-completed)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{resources}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"
jsonpath "$.type" == "epic"

# Buildings: ready (2/4 children done, 1 blocked, 1 pending)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{buildings}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"
jsonpath "$.type" == "epic"

# Defense: ready (all subtasks ready, no deps)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{defense}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"
jsonpath "$.type" == "epic"

# Fleet: approved (blocked by Buildings dependency)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{fleet}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"
jsonpath "$.type" == "epic"

# Galaxy View: ready (Resource System done)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"
jsonpath "$.type" == "epic"

# Alliance question: blocked
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{alliance_q}}
HTTP 200
[Asserts]
jsonpath "$.status" == "blocked"
jsonpath "$.type" == "question"

# Galaxy renderer: in_progress (claimed)
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_renderer}}
HTTP 200
[Asserts]
jsonpath "$.status" == "in_progress"
jsonpath "$.type" == "code"

# All types present
GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=epic
HTTP 200
[Asserts]
jsonpath "$.items" count == 5

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=code
HTTP 200
[Asserts]
jsonpath "$.items" count == 8

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=research
HTTP 200
[Asserts]
jsonpath "$.items" count == 2

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=question
HTTP 200
[Asserts]
jsonpath "$.items" count == 2

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=refactor
HTTP 200
[Asserts]
jsonpath "$.items" count == 2

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=review
HTTP 200
[Asserts]
jsonpath "$.items" count == 2
