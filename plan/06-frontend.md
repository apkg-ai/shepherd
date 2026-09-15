# Frontend implementation specification

## Retained foundation (UI-01)

React/TypeScript/Vite, hash router, TanStack Query, Orval fetch/hooks/Zod, React Flow/Dagre, CSS modules and existing design tokens remain. No new UI framework or global state library. Query cache owns server state; component state owns unsaved edits, selected graph node, expanded sections and dialogs. Never copy the entire server graph into a second mutable store.

Reuse components/Button, Dialog, ReasonDialog, ConfirmDialog, FormField, PageHeader, Toast and states; extend StatusBadge and TypeBadge. Retain keyboard focus restoration and existing accessible dialogs. Replace taskTree decomposition inference with explicit goal/epic membership. New screens live under screens/goals, screens/epics, screens/documents and screens/identity. Shared capability and eligibility rendering belongs in components/EligibilityPanel.tsx, not each screen.

## Routes, operations and screen acceptance

All routes below are hash paths. p, g, e, t denote UUID parameters. All forms show field errors inline and an error summary linked to the first invalid control. Create success navigates to new detail; edit success updates cache then returns to detail. Cancel returns without writes; dirty forms request discard confirmation. There is no generic drag-to-change-status interaction.

| Route | Data/operations | Layout and actions | Empty/error behavior |
|---|---|---|---|
| / | listProjects/createProject/importProject | Project cards: name and epic counts; Create, Import; archived toggle | Explain project purpose + Create or Import; show schema errors and retry list failure |
| /login | createBrowserSession | Owner-token password input, explain where local token file is; Sign in | No token persistence in localStorage; clear input after success/failure |
| /projects/:p | getProject/listGoals | Project title, goal cards, epic counts, New goal, Review queue, Knowledge, Settings | 0 goals CTA; no empty full-project graph |
| /projects/:p/goals/new | createGoal | Title required, description Markdown textarea | Owner only |
| /projects/:p/goals/:g | getGoal/getGoalGraph/listEpics | Goal brief, counts, epic graph/list toggle, New epic | No epics CTA; breadcrumb back to project |
| /projects/:p/goals/:g/edit | updateGoal | Title/description | Revision conflict comparison |
| /projects/:p/goals/:g/epics/new | createEpic | Fixed goal, title, description | Cannot change owner in form |
| /projects/:p/epics/:e | getEpic/getEpicGraph/listTasks/listDocuments | Epic header, status/eligibility, tasks graph/list, documents, history; New task | 0 tasks does not imply done; explicit Complete if permitted |
| /projects/:p/epics/:e/edit | updateEpic | Title/description | Locked during affected active claims/review; explain blocker |
| /projects/:p/epics/:e/tasks/new | createTask/listTaskTypes | Fixed epic; title, description, type; Planning required; Plan review; Work review | Defaults loaded before form enabled |
| /projects/:p/tasks/:t | getTask/getTaskContext/listTaskSessions/listSubmissions/listDocuments | Task overview, plan, output, review, timeline; action panel from allowed_actions | Missing plan explains next action; no fabricated summary |
| /projects/:p/tasks/:t/edit | updateTask/listTaskTypes | Same editable fields; ownership read-only | Preserve dirty inputs on 412; offer Compare / Discard local / Resubmit with fresh revision |
| /projects/:p/review | listEpics/listTasks/listSubmissions | Tabs Proposals / Plans / Work; policy filter; oldest-first queue UI | “No work awaiting review”; no misleading done count |
| /projects/:p/submissions/:s | getSubmission/listSubmissionReviews/getDocumentRevision | Exact submitted revisions, producer, policy, diff vs prior rejected submission; Approve/Request changes | Stale or withdrawn submission: read-only explanation |
| /projects/:p/documents/:d | getDocument/listDocumentRevisions/getDocumentRevision | Title/kind/owner; revision selector; Markdown preview/history; New revision | Missing doc 404 with owner navigation |
| /projects/:p/documents/new | createDocument | Owner/kind fixed by link; title/body/links | Links validate as HTTP(S); draft remains local on failure |
| /projects/:p/documents/:d/edit | createDocumentRevision | Plain textarea + preview; edit starts from selected latest revision | No rich text dependency; Save creates new revision |
| /projects/:p/knowledge | listDocuments owner_kind=project | Project notes/decisions/briefs with kind filters; New document | Replaces knowledge-items screen semantics |
| /projects/:p/history | listHistory | Actor, action, resource links, reason via detail records; Load more | Chronological audit, not editable |
| /projects/:p/settings | getProject/updateProject/archiveProject/exportProject/listTaskTypes/createTaskType/updateTaskType | Four defaults, archive project, export, type registry | Explicit warning defaults affect new tasks only |
| /settings/agents | listAgents/createAgent/revokeAgent | Label, status, issue/revoke token; show new token once with Copy | Token hidden after dismissal; revoke explains claim revocation |

Add fixed-scope query parameters to document-new route: owner_kind, owner_id, kind; validate against loaded project entities. Project import lives in registry dialog calling importProject; show schema errors and import success link. Archive controls appear on detail/settings; no Delete. Owner can claim and report plan/execute work in task action panel: Claim phase, Save output, Report success/failure, Release; store lease token only in memory, lose/release via TTL on browser close. Human reviews require no review claim. This makes manual tasks usable through browser while agent execution remains external.

## Task detail wireframe

```text
Space Game / Combat prototype / Combat rules / Implement targeting
[Started] [Execution]  Waiting on: Damage model [link]
Type: code       Planning: required       Plan review: agent       Work review: human
--------------------------------------------------------------------------
Overview | Plan | Output & reviews | Sessions | History
Selected plan: Targeting plan · revision 3 · approved by Reviewer B
[View exact revision]             [Allowed actions and disabled reasons]
Description / acceptance context / links
```

At narrow widths place actions beneath the header and side panel below graph/list. Minimum 320 CSS px; no body horizontal scrolling. Graph canvas may pan within its bounds. Never encode state using color alone: label + icon, accessible names, focusable node and edge controls. Dialog Escape closes unless mutation committing; restore focus to trigger. Loading skeleton must retain headings and dimensions.

## Dependency graphs

Goal graph contains epics only, epic graph tasks only. Use getGoalGraph/getEpicGraph; graph node IDs are resource UUIDs. Dagre layout prerequisite→dependent left-to-right; disconnected nodes form their own columns/components. Preserve viewport on event refresh and selection by UUID. Layout recomputes only when topology changes, not on claim heartbeat. Start/end nodes are derived layout hints, not editable domain fields.

Click node opens right side panel; double-click/Enter opens detail. Selected node highlights immediate prerequisites/dependents; dim others but do not hide them. Side panel shows readable state, counts (epics), current phase (task detail query), gate reasons and Open details. Toolbar: Fit, zoom, layout reset, list view. No custom drag positions persisted in v1. User pan/zoom stays local per route for the current browser session.

Add dependency dialog chooses “This item requires [prerequisite]”; candidate list stays within same goal/epic. Submit createDependency with explicit IDs. Edge arrow is prerequisite→dependent. Remove edge opens reasoned explanation that removal can unblock work; owner-only deleteDependency. Server rejects cycles, scope mismatches and unsafe active-work changes; show its code/message and keep selection. Disabled actions use permitted-action reasons, not independently calculated status conditions.

Above contract graph limit show list fallback and graph_too_large explanation; never draw incomplete edges as a complete graph. Playwright covers independent goals, disconnected nodes, branches, joins, cancelled nodes and keyboard navigation.

## Forms, queries and real-time state

All requests flow through shepherdFetch. Add credentials same-origin, CSRF for cookie mutations, If-Match from loaded revision, Idempotency-Key per submitted intent; keep the same key during transport retry. Do not generate a fresh key on each network attempt. No optimistic workflow/claim/review/graph mutations: wait for committed response. Text drafts remain local until acknowledged. On 412 retain local draft and show latest remote content side by side; a deliberate resubmit uses latest revision and a new key.

Orval owns API models/hooks and Zod input validation; do not hand-edit generated files. Use generated query key factories. Single project SSE subscription in AppLayout; batch invalidations for 200 ms. Every event invalidates the specific resource detail and its collection plus project/goal/epic counters, work queue, graph and reviews when affected. claim.changed does not force graph relayout. document.changed invalidates revision collections and context. Credential/session events are local identity refreshes, not project domain SSE.

Disconnect displays “Reconnecting” but retains cached data; disable mutations only when an actual request cannot reach server, not solely because SSE is down. On reconnect replay from last stored event ID; resync_required invalidates all active project queries and replaces cursor. 401 clears owner UI session and navigates login while preserving nonsecret text draft in memory; 403 explains missing capability, no login loop. 404 shows Not found with parent links. 500/503 shows retry with request_id.

Markdown preview uses a new shared Markdown component with react-markdown 10.1.0, no raw HTML plugin, allowed HTTP(S) links only, images disabled, external links rel=noopener noreferrer. Existing textarea remains editor. Do not execute scripts, fetch link metadata, or render file:// links. The pinned renderer source is recorded in [dependency choices](dependencies.md); add XSS regression tests in the document UI step.
