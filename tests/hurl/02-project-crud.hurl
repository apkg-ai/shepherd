# Project CRUD: create → get → list → update → delete → 404.

# Create a project.
POST {{base_url}}/api/v1/projects
Content-Type: application/json
{
  "name": "hurl-project",
  "description": "Created by hurl E2E test",
  "settings": { "review_gate": true }
}
HTTP 201
[Asserts]
jsonpath "$.name" == "hurl-project"
jsonpath "$.description" == "Created by hurl E2E test"
jsonpath "$.settings.review_gate" == true
jsonpath "$.id" matches "^[0-9a-f-]{36}$"
jsonpath "$.created_at" exists
jsonpath "$.updated_at" exists
header "RateLimit-Limit" exists
[Captures]
project_id: jsonpath "$.id"

# Get the project by ID.
GET {{base_url}}/api/v1/projects/{{project_id}}
HTTP 200
[Asserts]
jsonpath "$.id" == {{project_id}}
jsonpath "$.name" == "hurl-project"

# List projects — should contain the one we just created.
GET {{base_url}}/api/v1/projects
HTTP 200
[Asserts]
jsonpath "$.items" count >= 1
jsonpath "$.has_more" == false

# Update the project name.
PATCH {{base_url}}/api/v1/projects/{{project_id}}
Content-Type: application/json
{
  "name": "hurl-renamed"
}
HTTP 200
[Asserts]
jsonpath "$.name" == "hurl-renamed"
jsonpath "$.description" == "Created by hurl E2E test"

# Update review_gate setting.
PATCH {{base_url}}/api/v1/projects/{{project_id}}
Content-Type: application/json
{
  "settings": { "review_gate": false }
}
HTTP 200
[Asserts]
jsonpath "$.settings.review_gate" == false

# Delete the project.
DELETE {{base_url}}/api/v1/projects/{{project_id}}
HTTP 204

# Get after delete → 404.
GET {{base_url}}/api/v1/projects/{{project_id}}
HTTP 404
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:not-found"
jsonpath "$.status" == 404
