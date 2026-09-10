# Knowledge CRUD (project- and task-scoped) and export/import round-trip.

# Setup: a project with the review gate off, plus one ready task.
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{
  "name": "hurl-knowledge",
  "settings": { "review_gate": false }
}
HTTP 201
[Captures]
pid: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{
  "title": "Produces knowledge",
  "type": "research",
  "status": "approved"
}
HTTP 201
[Captures]
tid: jsonpath "$.id"

# Project-scoped knowledge (conventions, goals).
POST {{base_url}}/api/v1/projects/{{pid}}/knowledge
Content-Type: application/json
{
  "type": "decision",
  "title": "Cursor pagination over offset",
  "content": "All list endpoints use cursor-based pagination.",
  "scope": "project"
}
HTTP 201
[Asserts]
jsonpath "$.scope" == "project"
jsonpath "$.task_id" not exists
[Captures]
kid_project: jsonpath "$.id"

# Task-scoped knowledge.
POST {{base_url}}/api/v1/projects/{{pid}}/knowledge
Content-Type: application/json
{
  "type": "link",
  "title": "Reference PR",
  "content": "https://example.com/pr/2",
  "scope": "task",
  "task_id": "{{tid}}"
}
HTTP 201
[Asserts]
jsonpath "$.scope" == "task"
jsonpath "$.task_id" == "{{tid}}"
[Captures]
kid_task: jsonpath "$.id"

# List all, then filter by scope, type, and task.
GET {{base_url}}/api/v1/projects/{{pid}}/knowledge
HTTP 200
[Asserts]
jsonpath "$.items" count == 2

GET {{base_url}}/api/v1/projects/{{pid}}/knowledge?scope=project
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].id" == "{{kid_project}}"

GET {{base_url}}/api/v1/projects/{{pid}}/knowledge?type=link
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].id" == "{{kid_task}}"

GET {{base_url}}/api/v1/projects/{{pid}}/knowledge?task_id={{tid}}
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].id" == "{{kid_task}}"

# Get and delete.
GET {{base_url}}/api/v1/projects/{{pid}}/knowledge/{{kid_task}}
HTTP 200
[Asserts]
jsonpath "$.title" == "Reference PR"

DELETE {{base_url}}/api/v1/projects/{{pid}}/knowledge/{{kid_task}}
HTTP 204

GET {{base_url}}/api/v1/projects/{{pid}}/knowledge/{{kid_task}}
HTTP 404

# Put the task mid-work so the export carries an in_progress task.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/claim
Content-Type: application/json
{
  "identity": {
    "harness": "claude-code",
    "agent_model": "opus-5",
    "session_id": "hurl-sess-export"
  },
  "ttl_seconds": 600
}
HTTP 201

# Export the project as a versioned document.
GET {{base_url}}/api/v1/projects/{{pid}}/export
HTTP 200
[Asserts]
jsonpath "$.version" == "1.0.0"
jsonpath "$.project.name" == "hurl-knowledge"
jsonpath "$.tasks" count == 1
jsonpath "$.knowledge" count == 1
[Captures]
export_doc: body

# Import it back: a new project is created.
POST {{base_url}}/api/v1/projects/import
Content-Type: application/json
```
{{export_doc}}
```
HTTP 201
[Asserts]
jsonpath "$.project_id" != "{{pid}}"
jsonpath "$.task_count" == 1
jsonpath "$.knowledge_count" == 1
[Captures]
new_pid: jsonpath "$.project_id"

# Claims are runtime state, not exported: the imported mid-work task is
# normalized back to ready and immediately claimable.
GET {{base_url}}/api/v1/projects/{{new_pid}}/tasks
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].status" == "ready"

GET {{base_url}}/api/v1/projects/{{new_pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task.title" == "Produces knowledge"

# The imported knowledge landed in the new project with a fresh id.
GET {{base_url}}/api/v1/projects/{{new_pid}}/knowledge
HTTP 200
[Asserts]
jsonpath "$.items" count == 1
jsonpath "$.items[0].title" == "Cursor pagination over offset"
jsonpath "$.items[0].id" != "{{kid_project}}"
