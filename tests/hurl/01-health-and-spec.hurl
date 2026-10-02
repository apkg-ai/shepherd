# Health endpoint returns application/health+json with status "pass".

GET {{base_url}}/health
HTTP 200
Content-Type: application/health+json
[Asserts]
jsonpath "$.status" == "pass"
jsonpath "$.version" matches "^\\d+\\.\\d+\\.\\d+"
jsonpath "$.description" == "shepherd local daemon"
header "RateLimit-Limit" not exists
header "RateLimit-Remaining" not exists
header "RateLimit-Reset" not exists

# Served OpenAPI spec starts with "openapi:".

GET {{base_url}}/api/v1/openapi.yaml
HTTP 200
Content-Type: application/yaml
[Asserts]
body startsWith "openapi:"
