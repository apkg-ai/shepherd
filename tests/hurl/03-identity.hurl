# Full identity lifecycle over live HTTP (plan step 007): owner login,
# agent issuance, capability restriction, revocation, logout.

# Owner login sets the session cookie and returns the CSRF token.
POST {{base_url}}/api/v1/session
Origin: {{origin}}
{
  "owner_token": "{{owner_token}}"
}
HTTP 201
[Captures]
csrf: jsonpath "$.csrf_token"
[Asserts]
jsonpath "$.actor.kind" == "human"
jsonpath "$.expires_at" exists
cookie "shepherd_session" exists
header "Set-Cookie" contains "HttpOnly"
header "Set-Cookie" contains "SameSite=Strict"

# GET /session re-serves the same CSRF without sliding the expiry.
GET {{base_url}}/api/v1/session
HTTP 200
[Asserts]
jsonpath "$.csrf_token" == "{{csrf}}"

# A cookie mutation without a CSRF token fails closed.
POST {{base_url}}/api/v1/agents
Origin: {{origin}}
{
  "label": "no-csrf"
}
HTTP 403

# A hostile Origin fails closed even with the valid cookie and CSRF.
POST {{base_url}}/api/v1/agents
Origin: https://evil.example.com
X-CSRF-Token: {{csrf}}
{
  "label": "evil-origin"
}
HTTP 403

# The owner issues an agent; the token is returned exactly once.
POST {{base_url}}/api/v1/agents
Origin: {{origin}}
X-CSRF-Token: {{csrf}}
{
  "label": "builder"
}
HTTP 201
[Captures]
agent_id: jsonpath "$.actor.id"
agent_token: jsonpath "$.token"
[Asserts]
jsonpath "$.actor.kind" == "agent"
jsonpath "$.actor.revoked" == false

# The agent's principal is derived from its credential, not its claims.
GET {{base_url}}/api/v1/principal
Authorization: Bearer {{agent_token}}
HTTP 200
[Asserts]
jsonpath "$.kind" == "agent"
jsonpath "$.id" == "{{agent_id}}"

# Agents cannot mint agents.
POST {{base_url}}/api/v1/agents
Authorization: Bearer {{agent_token}}
{
  "label": "minion"
}
HTTP 403
[Asserts]
jsonpath "$.code" == "forbidden"

# Revocation requires an Idempotency-Key (plan/07: 428 without it).
POST {{base_url}}/api/v1/agents/{{agent_id}}/revoke
Origin: {{origin}}
X-CSRF-Token: {{csrf}}
{
  "reason": "went rogue"
}
HTTP 428
[Asserts]
jsonpath "$.code" == "precondition_required"

# Revoke the agent.
POST {{base_url}}/api/v1/agents/{{agent_id}}/revoke
Origin: {{origin}}
X-CSRF-Token: {{csrf}}
Idempotency-Key: 7f000001-0000-7000-8000-000000000007
{
  "reason": "went rogue"
}
HTTP 200
[Asserts]
jsonpath "$.ok" == true

# The same key replays the stored answer.
POST {{base_url}}/api/v1/agents/{{agent_id}}/revoke
Origin: {{origin}}
X-CSRF-Token: {{csrf}}
Idempotency-Key: 7f000001-0000-7000-8000-000000000007
{
  "reason": "went rogue"
}
HTTP 200
[Asserts]
jsonpath "$.ok" == true

# The revoked token is dead everywhere.
GET {{base_url}}/api/v1/principal
Authorization: Bearer {{agent_token}}
HTTP 401
[Asserts]
jsonpath "$.code" == "unauthenticated"

# The revoked agent stays listed for owner review.
GET {{base_url}}/api/v1/agents
HTTP 200
[Asserts]
jsonpath "$.items[0].id" == "{{agent_id}}"
jsonpath "$.items[0].revoked" == true

# Logout deletes the session and expires the cookie.
DELETE {{base_url}}/api/v1/session
Origin: {{origin}}
X-CSRF-Token: {{csrf}}
HTTP 200
[Asserts]
jsonpath "$.ok" == true
header "Set-Cookie" contains "Max-Age=0"

GET {{base_url}}/api/v1/session
HTTP 401
