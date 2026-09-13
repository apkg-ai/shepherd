import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EVENT_TYPES, eventInvalidations, useProjectEvents, type EventType } from "./events";

const PROJECT = "00000000-0000-4000-8000-00000000000a";
const TASK = "00000000-0000-4000-8000-00000000000b";

describe("eventInvalidations", () => {
  it("maps task events to the list and detail paths", () => {
    expect(eventInvalidations("task.status_changed", { task_id: TASK }, PROJECT)).toEqual([
      `/api/v1/projects/${PROJECT}/tasks`,
      `/api/v1/projects/${PROJECT}/next-task`,
      `/api/v1/projects/${PROJECT}/tasks/${TASK}`,
    ]);
  });

  it("maps relation events to the bulk feed and task lists", () => {
    expect(eventInvalidations("relation.added", {}, PROJECT)).toEqual([
      `/api/v1/projects/${PROJECT}/relations`,
      `/api/v1/projects/${PROJECT}/tasks`,
    ]);
  });

  it("maps sessions, knowledge, and project events", () => {
    expect(eventInvalidations("session.recorded", { task_id: TASK }, PROJECT)).toContain(
      `/api/v1/projects/${PROJECT}/tasks/${TASK}/sessions`,
    );
    expect(eventInvalidations("knowledge.added", {}, PROJECT)).toEqual([
      `/api/v1/projects/${PROJECT}/knowledge`,
    ]);
    expect(eventInvalidations("project.updated", {}, PROJECT)).toEqual([
      "/api/v1/projects",
      `/api/v1/projects/${PROJECT}`,
    ]);
  });

  it("covers the whole catalog", () => {
    for (const type of EVENT_TYPES) {
      expect(eventInvalidations(type, { task_id: TASK }, PROJECT).length).toBeGreaterThan(0);
    }
  });
});

/** Minimal EventSource fake capturing listeners for manual dispatch. */
class FakeEventSource {
  static instances: FakeEventSource[] = [];
  readonly url: string;
  readonly listeners = new Map<string, (event: MessageEvent<string>) => void>();
  onerror: (() => void) | null = null;
  onopen: (() => void) | null = null;
  closed = false;

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void) {
    this.listeners.set(type, listener);
  }
  close() {
    this.closed = true;
  }
  emit(type: EventType, payload: unknown) {
    this.listeners.get(type)?.({ data: JSON.stringify(payload) } as MessageEvent<string>);
  }
}

function Harness({ projectId }: { projectId?: string }) {
  useProjectEvents(projectId);
  return null;
}

describe("useProjectEvents", () => {
  let queryClient: QueryClient;
  let invalidated: unknown[];

  function renderHarness(projectId?: string) {
    return render(
      <QueryClientProvider client={queryClient}>
        <Harness projectId={projectId} />
      </QueryClientProvider>,
    );
  }

  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("EventSource", FakeEventSource);
    FakeEventSource.instances = [];
    queryClient = new QueryClient();
    invalidated = [];
    vi.spyOn(queryClient, "invalidateQueries").mockImplementation((filters) => {
      invalidated.push(filters);
      return Promise.resolve();
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("subscribes to every catalog event on the project stream", () => {
    renderHarness(PROJECT);

    const [source] = FakeEventSource.instances;
    expect(source.url).toBe(`/api/v1/events?project_id=${PROJECT}`);
    expect([...source.listeners.keys()].sort()).toEqual([...EVENT_TYPES].sort());
  });

  it("coalesces a burst of events into one invalidation wave", () => {
    renderHarness(PROJECT);
    const [source] = FakeEventSource.instances;

    source.emit("task.status_changed", { project_id: PROJECT, task_id: TASK });
    source.emit("session.recorded", { project_id: PROJECT, task_id: TASK });
    expect(invalidated).toHaveLength(0);

    vi.runAllTimers();
    expect(invalidated).toHaveLength(1);

    // The wave hits queries for both events' paths.
    const { predicate } = invalidated[0] as {
      predicate: (q: { queryKey: unknown[] }) => boolean;
    };
    expect(predicate({ queryKey: [`/api/v1/projects/${PROJECT}/tasks`] })).toBe(true);
    expect(
      predicate({ queryKey: ["infinite", `/api/v1/projects/${PROJECT}/tasks/${TASK}/sessions`] }),
    ).toBe(true);
    expect(predicate({ queryKey: [`/api/v1/projects/${PROJECT}/knowledge`] })).toBe(false);
  });

  it("refetches everything project-scoped after a reconnect", () => {
    renderHarness(PROJECT);
    const [source] = FakeEventSource.instances;

    source.onopen?.();
    expect(invalidated).toHaveLength(0);

    source.onerror?.();
    source.onopen?.();
    expect(invalidated).toHaveLength(1);
    const { predicate } = invalidated[0] as {
      predicate: (q: { queryKey: unknown[] }) => boolean;
    };
    expect(predicate({ queryKey: [`/api/v1/projects/${PROJECT}/relations`] })).toBe(true);
    expect(predicate({ queryKey: ["/api/v1/projects/other"] })).toBe(false);
  });

  it("closes the stream and clears the flush timer on unmount", () => {
    const { unmount } = renderHarness(PROJECT);
    const [source] = FakeEventSource.instances;
    source.emit("task.created", { project_id: PROJECT, task_id: TASK });

    unmount();
    expect(source.closed).toBe(true);
    vi.runAllTimers();
    expect(invalidated).toHaveLength(0);
  });

  it("does nothing without a project or without EventSource", () => {
    renderHarness(undefined);
    expect(FakeEventSource.instances).toHaveLength(0);

    vi.stubGlobal("EventSource", undefined);
    renderHarness(PROJECT);
    expect(FakeEventSource.instances).toHaveLength(0);
  });
});
