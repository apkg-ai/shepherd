# Retained and added dependencies

Retain current package manifests and lockfiles for existing functionality. New additions only at their owning step:

| Package | Pin | Purpose | Evidence |
|---|---|---|---|
| reqwest | =0.13.5, default-features=false, features=[json] | Shared Rust local REST client | [Crate documentation](https://docs.rs/reqwest/0.13.5/reqwest/) |
| rmcp | =3.3.0, default-features=false, features=[server,macros,transport-io,schemars] | Official Rust stdio MCP server | [Crate documentation](https://docs.rs/rmcp/3.3.0/rmcp/) |
| react-markdown | 10.1.0 | Safe Markdown preview without raw HTML | [Release](https://github.com/remarkjs/react-markdown/releases/tag/10.1.0) |
| getrandom | =0.3.4 | Random token bytes; version already in lockfile | core/Cargo.lock inspected |
| sha2 | =0.10.9 | Credential/request hashes; existing lockfile version | core/Cargo.lock inspected |
| aes-gcm | =0.10.3 | Encrypted idempotency response cache | [Crate documentation](https://docs.rs/aes-gcm/0.10.3/aes_gcm/) |
| fs2 | =0.4.3 | Cross-process daemon file lock | [Crate documentation](https://docs.rs/fs2/0.4.3/fs2/) |
| subtle | =2.6.1 | Constant-time credential digest comparison | Verify via pinned crate resolution; no custom crypto comparison |
| tracing | =0.1.44 | Structured diagnostic events; existing lockfile version | core/Cargo.lock inspected |

Use an internal tracing Subscriber that writes the documented JSON log fields if no subscriber is already available; no tracing-subscriber dependency is needed for v1's fixed format. Use rmcp's schemars re-export rather than independently choosing another version. Existing clap handles CLI arguments. Add Tokio io-std/io-util/signal features where required. The new crates are selected; this documentation task does not install or compile them. Owner step must resolve lockfile and prove compatibility on Rust 1.98.1 before merging. Do not silently substitute another SDK/framework on failure; document a reproduced incompatibility.

MCP transport reference: [official stdio transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). Use negotiated rmcp-supported protocol version; tools only, no sampling, roots, prompts, resource subscriptions or MCP Tasks extension. stdout carries protocol messages only; stderr diagnostics.

Orval scratch generation must set override.query.version=5 or output.packageJson to ui/package.json; otherwise generation outside ui/ defaults to TanStack Query v4. The actual UI already declares v5. This was observed during planning checks.
