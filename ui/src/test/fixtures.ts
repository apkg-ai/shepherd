import type {
  Identity,
  KnowledgeItem,
  Project,
  Relation,
  Session,
  Task,
} from "../api/generated/model";

/**
 * Deterministic spec-complete builders. Tests assert against these — never
 * against the faker noise the generated background handlers produce.
 */

let counter = 0;
export function uuid(): string {
  counter += 1;
  return `00000000-0000-4000-8000-${String(counter).padStart(12, "0")}`;
}

export function identity(overrides: Partial<Identity> = {}): Identity {
  return {
    harness: "claude-code",
    agent_model: "opus-5",
    session_id: "sess-1",
    label: "Test agent",
    ...overrides,
  };
}

export function project(overrides: Partial<Project> = {}): Project {
  return {
    id: uuid(),
    name: "Test project",
    description: "A project used in tests",
    settings: { review_gate: true },
    created_at: "2026-09-01T10:00:00Z",
    updated_at: "2026-09-02T11:30:00Z",
    ...overrides,
  };
}

export function task(overrides: Partial<Task> = {}): Task {
  return {
    id: uuid(),
    project_id: uuid(),
    title: "Test task",
    description: "Do the thing",
    type: "code",
    status: "ready",
    metadata: {},
    assignee: null,
    graph_role: [],
    attempt_count: 0,
    created_at: "2026-09-03T09:00:00Z",
    updated_at: "2026-09-03T09:15:00Z",
    ...overrides,
  };
}

export function relation(overrides: Partial<Relation> = {}): Relation {
  return {
    id: uuid(),
    type: "depends_on",
    source_task_id: uuid(),
    target_task_id: uuid(),
    created_at: "2026-09-03T09:00:00Z",
    ...overrides,
  };
}

export function session(overrides: Partial<Session> = {}): Session {
  return {
    id: uuid(),
    task_id: uuid(),
    identity: identity(),
    started_at: "2026-09-04T10:00:00Z",
    ended_at: "2026-09-04T10:20:00Z",
    outcome: "succeeded",
    failure_reason: null,
    summary: "Implemented the thing",
    decisions: [],
    knowledge_items: [],
    artifacts: [],
    created_at: "2026-09-04T10:20:00Z",
    ...overrides,
  };
}

export function knowledgeItem(overrides: Partial<KnowledgeItem> = {}): KnowledgeItem {
  return {
    id: uuid(),
    project_id: uuid(),
    type: "note",
    title: "Test knowledge",
    content: "Something worth remembering",
    scope: "task",
    task_id: null,
    session_id: null,
    created_at: "2026-09-05T08:00:00Z",
    ...overrides,
  };
}

/** Wraps items as a cursor page (list response shape). */
export function page<TItem>(
  items: TItem[],
  next_cursor: string | null = null,
): { items: TItem[]; has_more: boolean; next_cursor: string | null } {
  return { items, has_more: next_cursor !== null, next_cursor };
}
