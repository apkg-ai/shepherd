# shepherd

Shepherd is a local-first visual hub for long, multi-session agentic projects. It maps an entire project
as a graph of typed tasks (code, question, refactor, review, research, …) and acts as the persistent shared
memory between agent and human sessions: what was done, what's next, and what we know.

Shepherd never spawns or orchestrates agents — it is a passive central hub (storage + REST API +
visualization) that agentic tools query for their next task and report back to.

See [docs/scope.md](docs/scope.md) for the full project scope, domain model, and roadmap.
