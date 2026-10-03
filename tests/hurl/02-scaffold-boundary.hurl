# Future v1 catalog operations are not exposed over live HTTP. API paths
# reject unauthenticated callers before routing; nothing succeeds as a stub.

GET {{base_url}}/api/v1/projects
HTTP 401
[Asserts]
jsonpath "$.code" == "unauthenticated"
jsonpath "$.request_id" exists

GET {{base_url}}/api/v1/projects/00000000-0000-7000-8000-000000000000/tasks
HTTP 401

# Wrong-host requests are rejected outright (plan/12 exact Host allowlist).
GET {{base_url}}/api/v1/principal
Host: evil.example.com:7542
HTTP 400
[Asserts]
jsonpath "$.code" == "malformed_request"
