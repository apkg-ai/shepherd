# Catalog boundary: API paths reject unauthenticated callers before routing,
# the deferred portability operations answer truthful problem 404s, and wrong
# hosts are rejected outright.

GET {{base_url}}/api/v1/projects
HTTP 401
[Asserts]
jsonpath "$.code" == "unauthenticated"
jsonpath "$.request_id" exists

GET {{base_url}}/api/v1/projects/00000000-0000-7000-8000-000000000000/tasks
HTTP 401

# Deferred portability routes (follow-up step) are problem 404s, not the shell.
GET {{base_url}}/api/v1/projects/00000000-0000-7000-8000-000000000000/export
Authorization: Bearer {{owner_token}}
HTTP 404
[Asserts]
header "Content-Type" contains "application/problem+json"
jsonpath "$.code" == "not_found"

POST {{base_url}}/api/v1/imports
Authorization: Bearer {{owner_token}}
Content-Type: application/json
{}
HTTP 404
[Asserts]
jsonpath "$.code" == "not_found"

# Wrong-host requests are rejected outright (plan/12 exact Host allowlist).
GET {{base_url}}/api/v1/principal
Host: evil.example.com:7542
HTTP 400
[Asserts]
jsonpath "$.code" == "malformed_request"
