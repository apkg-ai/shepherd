/**
 * REST seeding helpers for the E2E suite — the same agent-side calls
 * documented in docs/agent-guide.md, driven from Node. Kept dependency-free
 * (plain fetch, minimal local types) so the e2e tsconfig stays isolated
 * from the app's project references.
 */

const BASE = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:7543";

export interface SeededProject {
  id: string;
  name: string;
}

export interface SeededTask {
  id: string;
  title: string;
  status: string;
}

export const AGENT_IDENTITY = {
  harness: "playwright",
  agent_model: "opus-5",
  session_id: "e2e-agent",
  label: "E2E agent",
};

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${BASE}/api/v1${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", ...init?.headers },
  });
  if (!response.ok) {
    throw new Error(
      `${init?.method ?? "GET"} ${path} → ${response.status}: ${await response.text()}`,
    );
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function createProject(
  name: string,
  options: { description?: string; reviewGate?: boolean } = {},
): Promise<SeededProject> {
  return api("/projects", {
    method: "POST",
    body: JSON.stringify({
      name,
      description: options.description ?? "",
      settings: { review_gate: options.reviewGate ?? true },
    }),
  });
}

export function createTask(
  projectId: string,
  title: string,
  options: {
    status?: "approved" | "proposed";
    type?: string;
    description?: string;
    metadata?: Record<string, unknown>;
  } = {},
): Promise<SeededTask> {
  return api(`/projects/${projectId}/tasks`, {
    method: "POST",
    body: JSON.stringify({
      title,
      description: options.description ?? "",
      type: options.type ?? "code",
      status: options.status ?? "approved",
      metadata: options.metadata ?? {},
    }),
  });
}

/** Relation source is always the path task (decomposition source = parent). */
export function createRelation(
  projectId: string,
  sourceTaskId: string,
  type: "decomposition" | "depends_on",
  targetTaskId: string,
): Promise<unknown> {
  return api(`/projects/${projectId}/tasks/${sourceTaskId}/relations`, {
    method: "POST",
    body: JSON.stringify({ type, target_task_id: targetTaskId }),
  });
}

export async function reportSession(
  projectId: string,
  taskId: string,
  outcome: "succeeded" | "failed",
  options: { summary?: string; failureReason?: string } = {},
): Promise<void> {
  await api(`/projects/${projectId}/tasks/${taskId}/claim`, {
    method: "POST",
    body: JSON.stringify({ identity: AGENT_IDENTITY, ttl_seconds: 600 }),
  });
  await api(`/projects/${projectId}/tasks/${taskId}/sessions`, {
    method: "POST",
    body: JSON.stringify({
      identity: AGENT_IDENTITY,
      started_at: "2026-09-11T10:00:00Z",
      ended_at: "2026-09-11T10:10:00Z",
      outcome,
      summary: options.summary ?? `E2E session (${outcome})`,
      ...(outcome === "failed" && {
        failure_reason: options.failureReason ?? "E2E simulated failure",
      }),
    }),
  });
}

/** approved → claimed → succeeded session → in_review (gate on). */
export async function seedInReviewTask(projectId: string, title: string): Promise<SeededTask> {
  const task = await createTask(projectId, title);
  await reportSession(projectId, task.id, "succeeded");
  return task;
}

export function createKnowledge(
  projectId: string,
  options: {
    title: string;
    content: string;
    type?: "note" | "link" | "decision" | "transcript";
    scope?: "project" | "task" | "session";
    taskId?: string;
  },
): Promise<{ id: string }> {
  return api(`/projects/${projectId}/knowledge`, {
    method: "POST",
    body: JSON.stringify({
      type: options.type ?? "note",
      title: options.title,
      content: options.content,
      scope: options.scope ?? "project",
      ...(options.taskId !== undefined && { task_id: options.taskId }),
    }),
  });
}

/** Full export document for a project — the real thing, for import tests. */
export function exportProject(projectId: string): Promise<Record<string, unknown>> {
  return api(`/projects/${projectId}/export`);
}
