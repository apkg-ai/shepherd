# Future v1 catalog operations are not exposed by the scaffold over live HTTP.
# GETs fall through to the static dir (404); it serves no other method (405).

GET {{base_url}}/api/v1/projects
HTTP 404

GET {{base_url}}/api/v1/projects/00000000-0000-7000-8000-000000000000/tasks
HTTP 404

POST {{base_url}}/api/v1/session
HTTP 405
