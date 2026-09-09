# Error responses: 404, 409, 422, request body validation.

# 404 for nonexistent project.
GET {{base_url}}/api/v1/projects/00000000-0000-0000-0000-000000000000
HTTP 404
[Asserts]
header "Content-Type" contains "application/problem+json"
jsonpath "$.type" == "urn:shepherd:error:not-found"
jsonpath "$.status" == 404
jsonpath "$.title" exists
jsonpath "$.detail" exists

# 404 for PATCH nonexistent project.
PATCH {{base_url}}/api/v1/projects/00000000-0000-0000-0000-000000000000
Content-Type: application/json
{ "name": "ghost" }
HTTP 404
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:not-found"

# 404 for DELETE nonexistent project.
DELETE {{base_url}}/api/v1/projects/00000000-0000-0000-0000-000000000000
HTTP 404
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:not-found"

# Invalid UUID in path → 422 with the spec's ProblemDetail shape.
GET {{base_url}}/api/v1/projects/not-a-uuid
HTTP 422
[Asserts]
header "Content-Type" contains "application/problem+json"
jsonpath "$.type" == "urn:shepherd:error:validation-error"
jsonpath "$.status" == 422
jsonpath "$.errors[0].field" == "/path/project_id"

# Create project with missing required field "name".
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{}
HTTP 422
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:validation-error"
jsonpath "$.errors[0].field" == "/body/name"

# Setup: create project for task validation tests.
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "hurl-validation" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

# 404 for PATCH nonexistent task.
PATCH {{base_url}}/api/v1/projects/{{pid}}/tasks/00000000-0000-0000-0000-000000000000
Content-Type: application/json
{ "title": "ghost" }
HTTP 404
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:not-found"

# Create task missing required "title" and "type".
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "description": "no title or type" }
HTTP 422
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:validation-error"

# Create relation with invalid target UUID.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Source", "type": "code", "status": "approved" }
HTTP 201
[Captures]
tid: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/relations
Content-Type: application/json
{
  "type": "depends_on",
  "target_task_id": "not-a-uuid"
}
HTTP 422
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:validation-error"

# 404 for DELETE nonexistent relation.
DELETE {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/relations/00000000-0000-0000-0000-000000000000
HTTP 404
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:not-found"

# Approve an already-ready task → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/approve
Content-Type: application/json
{}
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:invalid-transition"

# Reject a non-in_review task → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/reject
Content-Type: application/json
{ "reason": "nope" }
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:invalid-transition"

# Block a cancelled task → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/cancel
HTTP 200

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid}}/block
Content-Type: application/json
{ "reason": "can't block cancelled" }
HTTP 409

# Unblock a non-blocked task → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Not blocked", "type": "code" }
HTTP 201
[Captures]
tid2: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{tid2}}/unblock
HTTP 409
