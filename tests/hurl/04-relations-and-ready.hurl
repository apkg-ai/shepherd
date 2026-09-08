# Relations: depends_on + decomposition, cycle rejection, single-parent,
# ready demotion, auto-ready on dependency removal, next-task.

# Setup project.
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "hurl-relations" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

# Create three tasks, all approved → auto-ready.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Task A", "type": "code", "status": "approved" }
HTTP 201
[Asserts]
jsonpath "$.status" == "ready"
[Captures]
a: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Task B", "type": "code", "status": "approved" }
HTTP 201
[Captures]
b: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Task C", "type": "code", "status": "approved" }
HTTP 201
[Captures]
c: jsonpath "$.id"

# Add depends_on: A depends on B.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}/relations
Content-Type: application/json
{
  "type": "depends_on",
  "target_task_id": "{{b}}"
}
HTTP 201
[Asserts]
jsonpath "$.type" == "depends_on"
jsonpath "$.source_task_id" == {{a}}
jsonpath "$.target_task_id" == {{b}}
[Captures]
rel_ab: jsonpath "$.id"

# A should be demoted to approved (dep B is not done).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# Add depends_on: B depends on C.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{b}}/relations
Content-Type: application/json
{
  "type": "depends_on",
  "target_task_id": "{{c}}"
}
HTTP 201
[Captures]
rel_bc: jsonpath "$.id"

# B should be demoted to approved.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{b}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# Attempt cycle: C depends on A → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{c}}/relations
Content-Type: application/json
{
  "type": "depends_on",
  "target_task_id": "{{a}}"
}
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:dependency-cycle"

# Next-task: only C is ready.
GET {{base_url}}/api/v1/projects/{{pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task.id" == {{c}}

# List relations for A.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}/relations
HTTP 200
[Asserts]
jsonpath "$.items" count == 1

# Remove A→B dependency → A should auto-ready.
DELETE {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}/relations/{{rel_ab}}
HTTP 204

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Decomposition: create parent-child.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Child", "type": "code", "status": "approved" }
HTTP 201
[Captures]
child: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}/relations
Content-Type: application/json
{
  "type": "decomposition",
  "target_task_id": "{{child}}"
}
HTTP 201

# Second parent → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{c}}/relations
Content-Type: application/json
{
  "type": "decomposition",
  "target_task_id": "{{child}}"
}
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:decomposition-violation"

# Next-task with nothing ready returns null task.
# (Cancel all ready tasks first to clear the queue.)
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{a}}/cancel
HTTP 200
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{c}}/cancel
HTTP 200
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{child}}/cancel
HTTP 200

GET {{base_url}}/api/v1/projects/{{pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task" == null
