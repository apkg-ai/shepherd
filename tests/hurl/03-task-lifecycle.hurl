# Full task lifecycle: create → approve → block → unblock → cancel.
# Also covers create-as-approved auto-ready, task CRUD, and list filters.

# Setup: create a project.
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "hurl-task-lifecycle" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

# Create a task as proposed (default).
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{
  "title": "Proposed task",
  "type": "code",
  "metadata": { "priority": "high" }
}
HTTP 201
[Asserts]
jsonpath "$.status" == "proposed"
jsonpath "$.type" == "code"
jsonpath "$.metadata.priority" == "high"
jsonpath "$.attempt_count" == 0
jsonpath "$.assignee" == null
[Captures]
tid_proposed: jsonpath "$.id"

# Get the task.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}
HTTP 200
[Asserts]
jsonpath "$.title" == "Proposed task"

# Update the task title.
PATCH {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}
Content-Type: application/json
{ "title": "Renamed task" }
HTTP 200
[Asserts]
jsonpath "$.title" == "Renamed task"

# Approve the proposed task → should become ready (no deps).
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}/approve
Content-Type: application/json
{}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Block it.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}/block
Content-Type: application/json
{ "reason": "waiting for design" }
HTTP 200
[Asserts]
jsonpath "$.status" == "blocked"

# Unblock → should restore to ready.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}/unblock
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Cancel.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}/cancel
HTTP 200
[Asserts]
jsonpath "$.status" == "cancelled"

# Create a task as approved → auto-readies.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{
  "title": "Auto-ready task",
  "type": "research",
  "status": "approved"
}
HTTP 201
[Asserts]
jsonpath "$.status" == "ready"
jsonpath "$.type" == "research"
[Captures]
tid_ready: jsonpath "$.id"

# List tasks with type filter.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=research
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].title" == "Auto-ready task"

# List tasks with status filter.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks?status=cancelled
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].title" == "Renamed task"

# Delete a task.
DELETE {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}
HTTP 204

# Verify deleted.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid_proposed}}
HTTP 404
