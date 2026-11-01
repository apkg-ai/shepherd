import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import { I18nextProvider } from "react-i18next";
import i18next from "i18next";
import { http, HttpResponse } from "msw";
import { TaskActions } from "./TaskActions";
import { problemResponse, server } from "../../../test/msw";
import { makeTask } from "../../../test/fixtures";
import { createQueryClient } from "../../../lib/queryClient";
import type { Eligibility } from "../../../api/generated/model/eligibility";
import "../../../i18n";

const noop = () => {};

const noEligibility: Eligibility = {
  can_plan: false,
  can_execute: false,
  can_review: false,
  reasons: [],
  allowed_actions: [],
};

const taskResponse = makeTask({
  id: "t1",
  project_id: "p1",
  epic_id: "e1",
  title: "T",
  revision: 2,
  eligibility: noEligibility,
});

function taskGetHandler() {
  return http.get("/api/v1/projects/p1/tasks/t1", () => HttpResponse.json(taskResponse));
}

function renderActions(
  eligibility: Eligibility,
  onMutationError: (message: string) => void = noop,
) {
  const client = createQueryClient({ onMutationError });
  client.setDefaultOptions({
    queries: { retry: false, gcTime: 0 },
    mutations: { retry: false },
  });
  return render(
    <I18nextProvider i18n={i18next}>
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <TaskActions projectId="p1" taskId="t1" revision={1} eligibility={eligibility} />
        </MemoryRouter>
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

describe("TaskActions", () => {
  it("renders only allowed_actions buttons — no inference from status", () => {
    renderActions({
      ...noEligibility,
      can_plan: true,
      allowed_actions: ["claimTask", "blockTask"],
    });

    expect(screen.getByRole("button", { name: "Claim planning" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Block" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Accept" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Claim execution" })).not.toBeInTheDocument();
  });

  it("splits claimTask into both phases when both flags are set", () => {
    renderActions({
      ...noEligibility,
      can_plan: true,
      can_execute: true,
      allowed_actions: ["claimTask"],
    });

    expect(screen.getByRole("button", { name: "Claim planning" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Claim execution" })).toBeInTheDocument();
  });

  it("shows no actions message when allowed_actions is empty", () => {
    renderActions(noEligibility);
    expect(screen.getByText("No actions available")).toBeInTheDocument();
  });

  it("shows no actions when claimTask is allowed but no claim flag is set", () => {
    renderActions({ ...noEligibility, allowed_actions: ["claimTask"] });
    expect(screen.getByText("No actions available")).toBeInTheDocument();
  });

  it("sends claim request with correct headers on click", async () => {
    const user = userEvent.setup();
    const headers: Record<string, string | null> = {};

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/claims", ({ request }) => {
        headers["if-match"] = request.headers.get("if-match");
        headers["idempotency-key"] = request.headers.get("idempotency-key");
        return HttpResponse.json({
          claim: {
            id: "c1",
            task_id: "t1",
            actor_id: "a1",
            phase: "plan",
            status: "active",
            expires_at: new Date(Date.now() + 300_000).toISOString(),
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
          lease_token: "test-lease",
          context: {
            project: { id: "p1", name: "P", description: "" },
            goal: { id: "g1", title: "G", description: "" },
            epic: { id: "e1", title: "E", description: "" },
            task: { id: "t1", title: "T", description: "", type_key: "code" },
            documents: [],
            history: [],
          },
        });
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, can_plan: true, allowed_actions: ["claimTask"] });

    await user.click(screen.getByRole("button", { name: "Claim planning" }));

    await waitFor(() => {
      expect(headers["if-match"]).toBe('"1"');
      expect(headers["idempotency-key"]).toBeTruthy();
    });
  });

  it("busy controls prevent duplicate intent", async () => {
    const user = userEvent.setup();
    let callCount = 0;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/accept", async () => {
        callCount++;
        await new Promise((r) => setTimeout(r, 100));
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["acceptTask"] });

    const button = screen.getByRole("button", { name: "Accept" });
    await user.click(button);

    expect(button).toHaveAttribute("aria-busy", "true");

    await waitFor(() => expect(callCount).toBe(1));
  });

  it("surfaces a failed action through the global mutation-error handler", async () => {
    const user = userEvent.setup();
    const onMutationError = vi.fn();

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/accept", () =>
        problemResponse(409, "wrong_phase", { title: "Wrong phase", detail: "Task is complete." }),
      ),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["acceptTask"] }, onMutationError);

    const button = screen.getByRole("button", { name: "Accept" });
    await user.click(button);

    await waitFor(() => expect(onMutationError).toHaveBeenCalledWith("Task is complete."));
    await waitFor(() => expect(button).not.toBeDisabled());
  });

  it("opens reason dialog for block action", async () => {
    const user = userEvent.setup();
    renderActions({ ...noEligibility, allowed_actions: ["blockTask"] });

    await user.click(screen.getByRole("button", { name: "Block" }));

    expect(await screen.findByLabelText("Reason")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Block task" })).toBeInTheDocument();
  });

  it("opens reason dialog for cancel action", async () => {
    const user = userEvent.setup();
    renderActions({ ...noEligibility, allowed_actions: ["cancelTask"] });

    await user.click(screen.getByRole("button", { name: "Cancel task" }));

    expect(await screen.findByRole("dialog", { name: "Cancel task" })).toBeInTheDocument();
  });

  it("opens reason dialog for waive action", async () => {
    const user = userEvent.setup();
    renderActions({ ...noEligibility, allowed_actions: ["waiveTask"] });

    await user.click(screen.getByRole("button", { name: "Waive task" }));

    expect(await screen.findByRole("dialog", { name: "Waive task" })).toBeInTheDocument();
  });

  it("submits block reason to API", async () => {
    const user = userEvent.setup();
    let capturedBody: unknown;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/block", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["blockTask"] });

    await user.click(screen.getByRole("button", { name: "Block" }));
    await user.type(screen.getByLabelText("Reason"), "Waiting on dep");
    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toEqual({ reason: "Waiting on dep" });
    });
  });

  it("keeps the reason dialog open with the reason when the mutation fails", async () => {
    const user = userEvent.setup();
    const onMutationError = vi.fn();
    let calls = 0;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/block", async () => {
        calls++;
        if (calls === 1) {
          return problemResponse(412, "revision_conflict", {
            title: "Stale revision",
            detail: "Task changed while blocking.",
          });
        }
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["blockTask"] }, onMutationError);

    await user.click(screen.getByRole("button", { name: "Block" }));
    await user.type(screen.getByLabelText("Reason"), "Waiting on dep");
    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() =>
      expect(onMutationError).toHaveBeenCalledWith("Task changed while blocking."),
    );
    expect(screen.getByRole("dialog", { name: "Block task" })).toBeInTheDocument();
    expect(screen.getByLabelText("Reason")).toHaveValue("Waiting on dep");

    await user.click(screen.getByRole("button", { name: "Submit" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Block task" })).not.toBeInTheDocument(),
    );
    expect(calls).toBe(2);
  });

  it("unblock action sends request", async () => {
    const user = userEvent.setup();
    let called = false;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/unblock", () => {
        called = true;
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["unblockTask"] });

    await user.click(screen.getByRole("button", { name: "Unblock" }));
    await waitFor(() => expect(called).toBe(true));
  });

  it("submits cancel reason to API", async () => {
    const user = userEvent.setup();
    let capturedBody: unknown;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/cancel", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["cancelTask"] });

    await user.click(screen.getByRole("button", { name: "Cancel task" }));
    await user.type(screen.getByLabelText("Reason"), "No longer needed");
    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toEqual({ reason: "No longer needed" });
    });
  });

  it("submits waive reason to API", async () => {
    const user = userEvent.setup();
    let capturedBody: unknown;

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/waive", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["waiveTask"] });

    await user.click(screen.getByRole("button", { name: "Waive task" }));
    await user.type(screen.getByLabelText("Reason"), "Handled elsewhere");
    await user.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() => {
      expect(capturedBody).toEqual({ reason: "Handled elsewhere" });
    });
  });

  it("disables other buttons while one action is busy", async () => {
    const user = userEvent.setup();

    server.use(
      http.post("/api/v1/projects/p1/tasks/t1/accept", async () => {
        await new Promise((r) => setTimeout(r, 200));
        return HttpResponse.json(taskResponse);
      }),
      taskGetHandler(),
    );

    renderActions({ ...noEligibility, allowed_actions: ["acceptTask", "blockTask"] });

    await user.click(screen.getByRole("button", { name: "Accept" }));

    expect(screen.getByRole("button", { name: "Accept" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Block" })).toBeDisabled();

    await waitFor(() => expect(screen.getByRole("button", { name: "Block" })).not.toBeDisabled());
  });

  it("renders all visible actions from allowed_actions", () => {
    renderActions({
      ...noEligibility,
      allowed_actions: ["cancelTask", "waiveTask", "acceptTask"],
    });

    expect(screen.getByRole("button", { name: "Cancel task" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Waive task" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Accept" })).toBeInTheDocument();
  });
});
