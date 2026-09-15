# Decision record

| ID | Decision | Reason / alternative rejected |
|---|---|---|
| D01 | Keep Rust/SQLite/React and current generation stack | Existing local architecture fits the confirmed scope; fix domain/atomicity instead of rewriting frameworks. |
| D02 | Goal organizes exactly-owned epics | User introduced multiple independent outcomes inside one project. No goal lifecycle gates. |
| D03 | Dependencies scoped to goal/epic | User chose independent goals; eliminates cross-level hidden deadlocks. |
| D04 | Separate lifecycle, phase and eligibility | Early planning must coexist with execution dependency waiting. |
| D05 | Optional planning; independent human-default reviews | Simple tasks stay simple; risky work has explicit gates. |
| D06 | Separate registered reviewer identity | User chose credential-level independence, not a different session label. |
| D07 | Block revokes claims | A block stops all affected phases; saved work remains recoverable. |
| D08 | Terminal work immutable | User chose follow-up work rather than reopen propagation. |
| D09 | Fresh storage baseline | User authorized breaking changes and chose archive-only MVP data. |
| D10 | Versioned Markdown + links | User excluded uploaded binary storage. |
| D11 | REST, CLI, stdio MCP + guide | All adapters share REST semantics and authorization. |
| D12 | No weighted/project scheduling features | User only wants hierarchy, navigable graphs and completion counts. |
| D13 | Fixed ownership after creation | Implementation default: avoids moving active dependency scopes; replacement/cancel is explicit. |
| D14 | Owner resolves waived-only/empty epic | Avoid accidental completion; waived cancellation does not fabricate success. |
| D15 | Agent policy review is agent-only | Human can withdraw/change policy explicitly; no silent role bypass. |
| D16 | Atomic synchronous completion cascades | Claims/reports must never observe partially advanced graphs. |
| D17 | Count archived history, no hard-delete endpoint | Visibility must not change dependency/completion meaning. |
| D18 | Local credentials, not same-user process isolation | Tokens enforce API roles; OS access controls determine actual file isolation. |
| D19 | No MVP/v1 mixed database | The reset and later implementation must never mutate a legacy schema accidentally. |
| D20 | Reset product code, retain engineering plumbing | User chose a green scaffold before implementation: keep workspace, CI, generators and generic UI/build primitives; remove MVP domain behavior and use Git history for selective recovery. |

Matt Pocock's [grill-me](https://github.com/mattpocock/skills/blob/main/skills/productivity/grill-me/SKILL.md) now delegates to [grilling](https://github.com/mattpocock/skills/blob/main/skills/productivity/grilling/SKILL.md). Its source was read and its decision rounds used in the design conversation; the skill was not installed in this repository. Product decisions above were confirmed in that conversation; D13 onward records explicit implementation defaults selected to make the handoff complete. No additional confirmation is required to implement the documented scope.
