# Health endpoint returns the stable-v1 Health body with status "pass".

GET {{base_url}}/health
HTTP 200
Content-Type: application/json
[Asserts]
jsonpath "$.status" == "pass"
jsonpath "$.version" matches "^\\d+\\.\\d+\\.\\d+"
jsonpath "$.schema_version" == 1
header "RateLimit-Limit" not exists
header "RateLimit-Remaining" not exists
header "RateLimit-Reset" not exists

# Served OpenAPI spec is the JSON-syntax stable contract.

GET {{base_url}}/api/v1/openapi.yaml
HTTP 200
Content-Type: application/yaml
[Asserts]
body startsWith "{"
jsonpath "$.openapi" exists
