# Local identity and capabilities (AUTH-01)

## Trust boundary

One OS user owns the installation. Tokens separate cooperating tools' API permissions; they are not an OS sandbox against an agent that can read the owner's files. A process with access to the owner token can act as owner. Document this concrete limit; do not claim secure isolation between same-user processes.

First startup creates owner actor plus random 32-byte base64url owner token in <data-dir>/owner-token (0600, parent 0700). Store SHA-256 hash in credentials; never log token. CLI owner mode uses --token-file pointing there. Browser login pastes that token into POST /session over loopback and receives random 32-byte session cookie HttpOnly, SameSite=Strict, Path=/, Max-Age=43200; local HTTP means Secure=false. A 32-byte CSRF token is generated at login, encrypted at rest with the replay key, and returned and required as X-CSRF-Token on every cookie-authenticated mutation except login. /session GET refreshes CSRF in memory and returns fixed expiry, no sliding extension. Logout deletes session; no general owner password/account system.

Owner creates labeled agents. Each issuance creates a distinct Actor UUID and random token returned once. Store only SHA-256 digest and actor ID. Revocation atomically marks actor revoked, closes its credentials/claims, and recomputes affected task availability. No token rotation preserving a second identity to bypass self-review; registering an agent intentionally creates a new principal and is owner-only. Agents do not get permission to register more agents. One human actor can have browser sessions and owner bearer token.

## Capability matrix

| Operation | Owner | Agent |
|---|---|---|
| Read live/archived project resources | yes | yes |
| Create project/goal; edit defaults/types; archive/export/import | yes | no |
| Create epic/task | yes, accepted | yes, proposal gate applies |
| Edit nonterminal unclaimed descriptions | yes | yes, same active-work guards |
| Lower planning/review requirements | yes, idle only | no |
| Accept proposals, cancel, waive, explicit empty-epic completion | yes | no |
| Add valid dependency | yes | yes |
| Remove dependency; unblock | yes | no |
| Block task/epic with reason | yes | yes |
| Create/append documents | yes | yes; immutable-output guards apply |
| Claim plan/execute; report/release | yes | yes, own claim only |
| Review human policy | yes | no |
| Review agent policy | no (change policy only after withdrawal) | yes, distinct producer and active review claim |
| Withdraw pending submission | yes | only producer |
| Credentials and browser sessions | yes | no |

If-Match and tokens are independent: knowing a revision does not authorize a mutation. Claimed actor labels in request bodies never grant rights. Authorize in core so CLI/MCP/new handlers cannot bypass checks. Human UI can view agent review queue but shows “Awaiting independent agent”; no disguised approve button.

## HTTP and credential handling

Bind only 127.0.0.1 in v1. Allowed Host values: 127.0.0.1:<configured port> and localhost:<configured port>. Exact Origin allowlist: http://127.0.0.1:<port>, http://localhost:<port>; development origin http://localhost:5173 only with explicit dev flag. Parse URLs; no starts_with matching. Cookie requests require matching Origin for mutations and CSRF; bearer clients may omit Origin, but a supplied untrusted Origin is rejected. Cross-origin credentialed CORS is development-only and explicit. Authentication header + cookie together: bearer wins; invalid bearer never falls back to owner cookie.

Every response sets request ID. Redact Authorization, Cookie, Set-Cookie, X-CSRF-Token, X-Lease-Token and login/credential bodies. Disable proxy and redirects in local HTTP adapters so tokens cannot leave the loopback service. Do not put tokens in URLs or MCP tool arguments. Serve a restrictive CSP: default-src self; script-src self; style-src self unsafe-inline (React Flow needs inline geometry); img-src self data:; connect-src self; frame-ancestors none; base-uri none. No broad fake RateLimit headers.

Claim secrets use random 32 bytes and constant-time hash comparison. To replay claim grants safely, encrypt idempotency response bodies with AES-256-GCM, random 96-bit nonce, actor/key/request hash as authenticated associated data. Key is <data-dir>/replay-key (0600); full backups must include it, project exports exclude it. Token issuance/browser login are not replayable and never persisted as plaintext. Missing replay key puts daemon in diagnostic-only mode; do not silently regenerate and lose replay semantics.

Additional crypto/locking crates are pinned in [dependencies](dependencies.md): getrandom 0.3.4, sha2 0.10.9; AES-GCM 0.10.3, fs2 0.4.3, subtle 2.6.1. These are scoped dependencies for credential randomness/hash/comparison/replay and daemon locking, not an authentication framework. Compile/test with pinned Rust in the identity step.
