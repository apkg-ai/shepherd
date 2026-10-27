import type { Actor } from "../api/generated/model/actor";
import type { BrowserSession } from "../api/generated/model/browserSession";
import type { Counts } from "../api/generated/model/counts";
import type { Eligibility } from "../api/generated/model/eligibility";
import type { Epic } from "../api/generated/model/epic";
import type { EntityStatus } from "../api/generated/model/entityStatus";
import type { Goal } from "../api/generated/model/goal";
import type { Project } from "../api/generated/model/project";
import type { ProjectSettings } from "../api/generated/model/projectSettings";

const now = () => new Date().toISOString();

const emptyCounts: Counts = { total: 0, done: 0, cancelled: 0, waived: 0 };

const defaultSettings: ProjectSettings = {
  proposal_gate: false,
  planning_required: true,
  plan_review: "human",
  work_review: "human",
};

const defaultEligibility: Eligibility = {
  can_plan: false,
  can_execute: false,
  can_review: false,
  reasons: [],
  allowed_actions: [],
};

export function makeActor(overrides?: Partial<Actor>): Actor {
  return {
    id: crypto.randomUUID(),
    kind: "human",
    label: "Owner",
    revoked: false,
    ...overrides,
  };
}

export function makeBrowserSession(overrides?: Partial<BrowserSession>): BrowserSession {
  return {
    actor: makeActor(),
    csrf_token: "test-csrf-token",
    expires_at: new Date(Date.now() + 3_600_000).toISOString(),
    ...overrides,
  };
}

export function makeProject(overrides?: Partial<Project>): Project {
  const ts = now();
  return {
    id: crypto.randomUUID(),
    revision: 1,
    created_at: ts,
    updated_at: ts,
    name: "Test project",
    description: "",
    settings: defaultSettings,
    archived: false,
    epic_counts: { ...emptyCounts },
    archives: [],
    ...overrides,
  };
}

export function makeGoal(overrides?: Partial<Goal>): Goal {
  const ts = now();
  return {
    id: crypto.randomUUID(),
    revision: 1,
    created_at: ts,
    updated_at: ts,
    project_id: crypto.randomUUID(),
    title: "Test goal",
    description: "",
    archived: false,
    completed: false,
    epic_counts: { ...emptyCounts },
    archives: [],
    ...overrides,
  };
}

export function makeEpic(overrides?: Partial<Epic> & { status?: EntityStatus }): Epic {
  const ts = now();
  return {
    id: crypto.randomUUID(),
    revision: 1,
    created_at: ts,
    updated_at: ts,
    project_id: crypto.randomUUID(),
    goal_id: crypto.randomUUID(),
    title: "Test epic",
    description: "",
    status: "open",
    archived: false,
    blocks: [],
    eligibility: { ...defaultEligibility },
    task_counts: { ...emptyCounts },
    cancellations: [],
    archives: [],
    ...overrides,
  };
}

export function makePage<T>(items: T[], nextCursor?: string) {
  return {
    items,
    ...(nextCursor !== undefined && { next_cursor: nextCursor }),
  };
}
