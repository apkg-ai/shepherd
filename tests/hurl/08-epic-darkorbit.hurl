# DarkOrbit: multi-level epic hierarchy with cascading completion.
#
# Structure:
#   Core Engine (epic, start)
#     ├── Game loop, Rendering pipeline, Input handling
#   Player System (epic, depends_on Core Engine)
#     ├── Player account, Ship loadout
#     └── Combat System (nested epic)
#         ├── Laser weapons, Rocket weapons, Damage calculation
#   Galaxy Map (epic, depends_on Core Engine)
#     ├── Star system gen, Space map rendering
#     └── PvE Encounters (nested epic)
#         ├── NPC AI, Alien spawns, Loot drops
#   Economy (epic, depends_on Player System + Galaxy Map)
#     ├── Credit system, Trading post, Auction house
#
# Verifies: epic type, decomposition cycle rejection, depends_on between
# epics, auto-completion cascade, type-filter queries.

# ── Setup ────────────────────────────────────────────────────────────

POST {{base_url}}/api/v1/projects
Content-Type: application/json
{ "name": "DarkOrbit" }
HTTP 201
[Captures]
pid: jsonpath "$.id"

# Turn off review gate for fast Done transitions.
PATCH {{base_url}}/api/v1/projects/{{pid}}
Content-Type: application/json
{ "settings": { "review_gate": false } }
HTTP 200

# ── Create epics ─────────────────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Core Engine", "type": "epic", "status": "approved" }
HTTP 201
[Asserts]
jsonpath "$.type" == "epic"
jsonpath "$.status" == "ready"
[Captures]
core_engine: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Player System", "type": "epic", "status": "approved" }
HTTP 201
[Asserts]
jsonpath "$.type" == "epic"
[Captures]
player_system: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Combat System", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
combat_system: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Galaxy Map", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
galaxy_map: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "PvE Encounters", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
pve_encounters: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Economy", "type": "epic", "status": "approved" }
HTTP 201
[Captures]
economy: jsonpath "$.id"

# ── Create leaf tasks ────────────────────────────────────────────────

# Core Engine subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Game loop implementation", "type": "code", "status": "approved" }
HTTP 201
[Captures]
game_loop: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Rendering pipeline", "type": "code", "status": "approved" }
HTTP 201
[Captures]
rendering: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Input handling", "type": "code", "status": "approved" }
HTTP 201
[Captures]
input_handling: jsonpath "$.id"

# Player System subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Player account model", "type": "code", "status": "approved" }
HTTP 201
[Captures]
player_account: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Ship selection & loadout", "type": "code", "status": "approved" }
HTTP 201
[Captures]
ship_loadout: jsonpath "$.id"

# Combat System subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Laser weapons", "type": "code", "status": "approved" }
HTTP 201
[Captures]
laser: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Rocket weapons", "type": "code", "status": "approved" }
HTTP 201
[Captures]
rocket: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Damage calculation", "type": "code", "status": "approved" }
HTTP 201
[Captures]
damage_calc: jsonpath "$.id"

# Galaxy Map subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Star system generation", "type": "code", "status": "approved" }
HTTP 201
[Captures]
star_gen: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Space map rendering", "type": "code", "status": "approved" }
HTTP 201
[Captures]
space_map: jsonpath "$.id"

# PvE Encounters subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "NPC AI behavior", "type": "code", "status": "approved" }
HTTP 201
[Captures]
npc_ai: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Alien spawn system", "type": "code", "status": "approved" }
HTTP 201
[Captures]
alien_spawns: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Loot drops", "type": "code", "status": "approved" }
HTTP 201
[Captures]
loot_drops: jsonpath "$.id"

# Economy subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Credit system", "type": "code", "status": "approved" }
HTTP 201
[Captures]
credits: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Trading post", "type": "code", "status": "approved" }
HTTP 201
[Captures]
trading: jsonpath "$.id"

POST {{base_url}}/api/v1/projects/{{pid}}/tasks
Content-Type: application/json
{ "title": "Auction house", "type": "code", "status": "approved" }
HTTP 201
[Captures]
auction: jsonpath "$.id"

# ── Wire decomposition relations ─────────────────────────────────────

# Core Engine → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{game_loop}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{rendering}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{input_handling}}" }
HTTP 201

# Player System → subtasks + nested Combat System epic
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{player_account}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{ship_loadout}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{combat_system}}" }
HTTP 201

# Combat System → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{combat_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{laser}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{combat_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{rocket}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{combat_system}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{damage_calc}}" }
HTTP 201

# Galaxy Map → subtasks + nested PvE epic
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{star_gen}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{space_map}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{pve_encounters}}" }
HTTP 201

# PvE Encounters → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{pve_encounters}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{npc_ai}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{pve_encounters}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{alien_spawns}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{pve_encounters}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{loot_drops}}" }
HTTP 201

# Economy → subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{credits}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{trading}}" }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{auction}}" }
HTTP 201

# ── Decomposition cycle rejection ───────────────────────────────────

# Try to make Game loop a parent of Core Engine (cycle) → 409.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{game_loop}}/relations
Content-Type: application/json
{ "type": "decomposition", "target_task_id": "{{core_engine}}" }
HTTP 409
[Asserts]
jsonpath "$.type" == "urn:shepherd:error:decomposition-violation"

# ── Wire depends_on between epics ────────────────────────────────────

# Player System depends_on Core Engine.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{core_engine}}" }
HTTP 201

# Player System should be demoted from ready to approved.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# Galaxy Map depends_on Core Engine.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{core_engine}}" }
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# Economy depends_on Player System.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{player_system}}" }
HTTP 201

# Economy depends_on Galaxy Map.
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}/relations
Content-Type: application/json
{ "type": "depends_on", "target_task_id": "{{galaxy_map}}" }
HTTP 201

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# ── Type filter: list only epics ─────────────────────────────────────

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=epic
HTTP 200
[Asserts]
jsonpath "$.items" count == 6

# ── PHASE 1: Complete Core Engine subtasks ───────────────────────────

# Complete Game loop
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{game_loop}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s01"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{game_loop}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s01"},
  "started_at": "2026-09-13T10:00:00Z",
  "ended_at": "2026-09-13T10:30:00Z",
  "outcome": "succeeded",
  "summary": "Game loop with delta time and fixed update step."
}
HTTP 201

# Core Engine not yet complete (2 subtasks remain).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Complete Rendering pipeline
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{rendering}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s02"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{rendering}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s02"},
  "started_at": "2026-09-13T10:30:00Z",
  "ended_at": "2026-09-13T11:00:00Z",
  "outcome": "succeeded",
  "summary": "WebGL renderer with sprite batching."
}
HTTP 201

# Core Engine still not complete (1 subtask remains).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Complete Input handling — this should trigger Core Engine auto-completion!
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{input_handling}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s03"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{input_handling}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s03"},
  "started_at": "2026-09-13T11:00:00Z",
  "ended_at": "2026-09-13T11:30:00Z",
  "outcome": "succeeded",
  "summary": "Keyboard + mouse + gamepad input via abstraction layer."
}
HTTP 201

# ★ Core Engine auto-completed! All 3 subtasks done.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{core_engine}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# ★ Player System and Galaxy Map auto-readied (depends_on Core Engine satisfied).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Economy still approved — needs both Player System AND Galaxy Map done.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# ── PHASE 2: Complete Player System branch ───────────────────────────

# Complete Player account
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_account}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s04"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_account}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s04"},
  "started_at": "2026-09-13T12:00:00Z",
  "ended_at": "2026-09-13T12:30:00Z",
  "outcome": "succeeded",
  "summary": "Player model with XP, level, and inventory slots."
}
HTTP 201

# Complete Ship loadout
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{ship_loadout}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s05"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{ship_loadout}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s05"},
  "started_at": "2026-09-13T12:30:00Z",
  "ended_at": "2026-09-13T13:00:00Z",
  "outcome": "succeeded",
  "summary": "Ship selection screen with weapon/shield slot configuration."
}
HTTP 201

# Player System not complete — Combat System (child epic) still pending.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Complete Combat System subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{laser}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s06"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{laser}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s06"},
  "started_at": "2026-09-13T13:00:00Z",
  "ended_at": "2026-09-13T13:30:00Z",
  "outcome": "succeeded",
  "summary": "Laser beam rendering with range falloff."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{rocket}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s07"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{rocket}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s07"},
  "started_at": "2026-09-13T13:30:00Z",
  "ended_at": "2026-09-13T14:00:00Z",
  "outcome": "succeeded",
  "summary": "Projectile rockets with splash damage radius."
}
HTTP 201

# Combat System not yet complete (Damage calculation remains).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{combat_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# Complete Damage calculation — triggers Combat System auto-complete,
# which triggers Player System auto-complete!
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{damage_calc}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s08"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{damage_calc}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s08"},
  "started_at": "2026-09-13T14:00:00Z",
  "ended_at": "2026-09-13T14:30:00Z",
  "outcome": "succeeded",
  "summary": "DPS formula with armor reduction and shield bypass."
}
HTTP 201

# ★ Combat System auto-completed (all 3 subtasks done).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{combat_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# ★ Player System auto-completed (Player account + Ship loadout + Combat System all done).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{player_system}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# Economy still approved — Galaxy Map not done yet.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "approved"

# ── PHASE 3: Complete Galaxy Map branch ──────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{star_gen}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s09"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{star_gen}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s09"},
  "started_at": "2026-09-13T15:00:00Z",
  "ended_at": "2026-09-13T15:30:00Z",
  "outcome": "succeeded",
  "summary": "Procedural star system with Perlin noise distribution."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{space_map}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s10"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{space_map}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s10"},
  "started_at": "2026-09-13T15:30:00Z",
  "ended_at": "2026-09-13T16:00:00Z",
  "outcome": "succeeded",
  "summary": "Minimap with fog of war and sector grid overlay."
}
HTTP 201

# Complete PvE Encounters subtasks
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{npc_ai}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s11"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{npc_ai}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s11"},
  "started_at": "2026-09-13T16:00:00Z",
  "ended_at": "2026-09-13T16:30:00Z",
  "outcome": "succeeded",
  "summary": "NPC behavior tree with patrol, chase, and attack states."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{alien_spawns}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s12"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{alien_spawns}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s12"},
  "started_at": "2026-09-13T16:30:00Z",
  "ended_at": "2026-09-13T17:00:00Z",
  "outcome": "succeeded",
  "summary": "Wave-based spawner with difficulty scaling per sector."
}
HTTP 201

# Complete Loot drops — triggers PvE auto-complete → Galaxy Map auto-complete
# → Economy auto-readies (both Player System and Galaxy Map now done)!
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{loot_drops}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s13"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{loot_drops}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s13"},
  "started_at": "2026-09-13T17:00:00Z",
  "ended_at": "2026-09-13T17:30:00Z",
  "outcome": "succeeded",
  "summary": "Weighted loot table with rarity tiers and cargo capacity check."
}
HTTP 201

# ★ PvE Encounters auto-completed.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{pve_encounters}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# ★ Galaxy Map auto-completed (Star gen + Space map + PvE Encounters all done).
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{galaxy_map}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# ★ Economy auto-readied! Both Player System and Galaxy Map are done.
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "ready"

# ── PHASE 4: Complete Economy ────────────────────────────────────────

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{credits}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s14"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{credits}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s14"},
  "started_at": "2026-09-13T18:00:00Z",
  "ended_at": "2026-09-13T18:30:00Z",
  "outcome": "succeeded",
  "summary": "Credit ledger with transaction history and anti-cheat checks."
}
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{trading}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s15"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{trading}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s15"},
  "started_at": "2026-09-13T18:30:00Z",
  "ended_at": "2026-09-13T19:00:00Z",
  "outcome": "succeeded",
  "summary": "NPC trading post with buy/sell spread and sector pricing."
}
HTTP 201

# Complete Auction house — triggers Economy auto-complete. Game is done!
POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{auction}}/claim
Content-Type: application/json
{ "identity": {"harness":"test","agent_model":"test","session_id":"s16"}, "ttl_seconds": 60 }
HTTP 201

POST {{base_url}}/api/v1/projects/{{pid}}/tasks/{{auction}}/sessions
Content-Type: application/json
{
  "identity": {"harness":"test","agent_model":"test","session_id":"s16"},
  "started_at": "2026-09-13T19:00:00Z",
  "ended_at": "2026-09-13T19:30:00Z",
  "outcome": "succeeded",
  "summary": "Player-to-player auction with bidding, buyout, and 5% fee."
}
HTTP 201

# ★ Economy auto-completed!
GET {{base_url}}/api/v1/projects/{{pid}}/tasks/{{economy}}
HTTP 200
[Asserts]
jsonpath "$.status" == "done"

# ── Final state: ALL tasks and epics are done ────────────────────────

GET {{base_url}}/api/v1/projects/{{pid}}/tasks?type=epic
HTTP 200
[Asserts]
jsonpath "$.items" count == 6
jsonpath "$.items[0].status" == "done"
jsonpath "$.items[1].status" == "done"
jsonpath "$.items[2].status" == "done"
jsonpath "$.items[3].status" == "done"
jsonpath "$.items[4].status" == "done"
jsonpath "$.items[5].status" == "done"

# No ready tasks remaining — everything is done.
GET {{base_url}}/api/v1/projects/{{pid}}/next-task
HTTP 200
[Asserts]
jsonpath "$.task" == null
