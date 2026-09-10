# SSE endpoint smoke test.
#
# Verifies the /api/v1/events endpoint is reachable and responds with the
# correct content type. Full event-stream consumption requires a streaming
# client; this test validates connectivity and headers only.

# Unfiltered SSE endpoint.
GET http://127.0.0.1:{{port}}/api/v1/events
HTTP 200
[Asserts]
header "content-type" contains "text/event-stream"

# SSE endpoint with project filter (UUID does not need to exist — the
# stream simply emits nothing until events for that project arrive).
GET http://127.0.0.1:{{port}}/api/v1/events?project_id=019421a5-7e6e-7000-8000-000000000001
HTTP 200
[Asserts]
header "content-type" contains "text/event-stream"
