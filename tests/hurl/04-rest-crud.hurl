# REST happy path over the live daemon: hierarchy, claim, report, history.

POST {{base_url}}/api/v1/projects
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e201
{
  "name": "Hurl project"
}
HTTP 201
[Captures]
project: jsonpath "$.id"
[Asserts]
jsonpath "$.revision" == 1
jsonpath "$.settings.proposal_gate" == true

POST {{base_url}}/api/v1/projects/{{project}}/goals
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e202
{
  "title": "Ship it"
}
HTTP 201
[Captures]
goal: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{project}}/epics
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e203
{
  "goal_id": "{{goal}}",
  "title": "First epic"
}
HTTP 201
[Captures]
epic: jsonpath "$.id"
[Asserts]
jsonpath "$.eligibility" exists

POST {{base_url}}/api/v1/projects/{{project}}/tasks
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e204
{
  "epic_id": "{{epic}}",
  "title": "First task",
  "type_key": "code"
}
HTTP 201
[Captures]
task: jsonpath "$.id"
[Asserts]
jsonpath "$.status" == "open"
jsonpath "$.eligibility.can_execute" == true

GET {{base_url}}/api/v1/projects/{{project}}/tasks/{{task}}
Authorization: Bearer {{owner_token}}
HTTP 200
[Asserts]
header "ETag" == "\"1\""

# Mutations without their command headers are 428 before routing.
PATCH {{base_url}}/api/v1/projects/{{project}}/tasks/{{task}}
Authorization: Bearer {{owner_token}}
Content-Type: application/json
{
  "description": "no headers"
}
HTTP 428
[Asserts]
jsonpath "$.code" == "precondition_required"

POST {{base_url}}/api/v1/projects/{{project}}/tasks/{{task}}/claims
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e205
If-Match: "1"
{
  "phase": "execute"
}
HTTP 201
[Captures]
claim: jsonpath "$.claim.id"
lease: jsonpath "$.lease_token"
[Asserts]
jsonpath "$.context.task.id" == "{{task}}"

POST {{base_url}}/api/v1/projects/{{project}}/claims/{{claim}}/report
Authorization: Bearer {{owner_token}}
Idempotency-Key: 00000000-0000-7000-8000-00000000e206
X-Lease-Token: {{lease}}
{
  "outcome": "failed",
  "summary": "first pass",
  "failure_reason": "needs a retry",
  "document_revision_ids": [],
  "links": []
}
HTTP 201
[Asserts]
jsonpath "$.outcome" == "failed"

GET {{base_url}}/api/v1/projects/{{project}}/history
Authorization: Bearer {{owner_token}}
HTTP 200
[Asserts]
jsonpath "$.items" count > 0
jsonpath "$.items[0].id" matches "^[1-9][0-9]*$"
