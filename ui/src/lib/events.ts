import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { getGetEventsUrl } from "../api/generated/events/events";
import { invalidatePaths } from "../api/invalidate";

/**
 * SSE liveness (S8, docs/04-ui.md): one EventSource per active project,
 * mapping each domain event to the query paths it stales. Events coalesce
 * on a short trailing timer so an agent hammering REST triggers one refetch
 * wave, not one per event. No replay in v1 — a reconnect after an error
 * refetches everything project-scoped instead.
 */

/** Event names from openapi/shepherd-events.asyncapi.yaml. */
export const EVENT_TYPES = [
  "project.created",
  "project.updated",
  "task.created",
  "task.updated",
  "task.status_changed",
  "relation.added",
  "relation.removed",
  "claim.acquired",
  "claim.renewed",
  "claim.released",
  "claim.expired",
  "session.recorded",
  "knowledge.added",
] as const;

export type EventType = (typeof EVENT_TYPES)[number];

const COALESCE_MS = 200;

/**
 * Pure event → stale-path mapping, exported for tests. Payload shapes come
 * from the AsyncAPI catalog; `task_id` is present on every task-scoped
 * event.
 */
export function eventInvalidations(
  type: EventType,
  payload: { task_id?: string },
  projectId: string,
): string[] {
  const base = `/api/v1/projects/${projectId}`;
  const taskPaths = (extra: string[] = []) => {
    const paths = [`${base}/tasks`, ...extra];
    if (payload.task_id !== undefined) paths.push(`${base}/tasks/${payload.task_id}`);
    return paths;
  };
  switch (type) {
    case "project.created":
      return ["/api/v1/projects"];
    case "project.updated":
      return ["/api/v1/projects", base];
    case "task.created":
    case "task.updated":
      return taskPaths();
    case "task.status_changed":
      // Status flips ripple into readiness — refresh the relations feed's
      // consumers too (the graph restyles nodes and edge animation).
      return taskPaths([`${base}/next-task`]);
    case "relation.added":
    case "relation.removed":
      return [`${base}/relations`, `${base}/tasks`];
    case "claim.acquired":
    case "claim.renewed":
    case "claim.released":
    case "claim.expired":
      return taskPaths();
    case "session.recorded":
      return taskPaths(
        payload.task_id !== undefined ? [`${base}/tasks/${payload.task_id}/sessions`] : [],
      );
    case "knowledge.added":
      return [`${base}/knowledge`];
  }
}

/**
 * Subscribes to the project's SSE stream and invalidates stale queries as
 * events arrive. Mounted once in AppLayout whenever a project is active —
 * the graph, task lists, review badge, detail, and knowledge screens all go
 * live off this single subscription. No-op where EventSource doesn't exist
 * (jsdom).
 */
export function useProjectEvents(projectId: string | undefined): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    if (projectId === undefined || typeof EventSource !== "function") return;

    const source = new EventSource(getGetEventsUrl({ project_id: projectId }));
    const pending = new Set<string>();
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    let hadError = false;

    const queueInvalidation = (paths: string[]) => {
      for (const path of paths) pending.add(path);
      flushTimer ??= setTimeout(() => {
        flushTimer = undefined;
        const paths = [...pending];
        pending.clear();
        invalidatePaths(queryClient, ...paths);
      }, COALESCE_MS);
    };

    for (const type of EVENT_TYPES) {
      source.addEventListener(type, (event: MessageEvent<string>) => {
        let payload: { task_id?: string } = {};
        try {
          payload = JSON.parse(event.data) as { task_id?: string };
        } catch {
          // Malformed frame — invalidate off the event type alone.
        }
        queueInvalidation(eventInvalidations(type, payload, projectId));
      });
    }

    source.onerror = () => {
      hadError = true;
    };
    // No replay in v1: after a dropped connection, anything might have
    // happened — refetch every query touching this project.
    source.onopen = () => {
      if (!hadError) return;
      hadError = false;
      void queryClient.invalidateQueries({
        predicate: (query) =>
          query.queryKey.some(
            (part) => typeof part === "string" && part.includes(`/projects/${projectId}`),
          ),
      });
    };

    return () => {
      source.close();
      if (flushTimer !== undefined) clearTimeout(flushTimer);
    };
  }, [projectId, queryClient]);
}
