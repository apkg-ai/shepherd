import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { makeGoal, makeEpic, makeTask, makePage } from "../../test/fixtures";
import { server } from "../../test/msw";
import { TabsView } from "./TabsView";

const PROJECT_ID = "p1";

const goalOpen = makeGoal({
  id: "g1",
  project_id: PROJECT_ID,
  title: "Combat System",
  completed: false,
  epic_counts: { total: 4, done: 1, cancelled: 0, waived: 0 },
});

const goalDone = makeGoal({
  id: "g2",
  project_id: PROJECT_ID,
  title: "Economy",
  completed: true,
  epic_counts: { total: 2, done: 2, cancelled: 0, waived: 0 },
});

const epicActive = makeEpic({
  id: "e1",
  project_id: PROJECT_ID,
  goal_id: "g1",
  title: "Damage Model",
  status: "active",
  task_counts: { total: 4, done: 1, cancelled: 0, waived: 0 },
});

const epicDone = makeEpic({
  id: "e2",
  project_id: PROJECT_ID,
  goal_id: "g1",
  title: "Targeting System",
  status: "done",
  task_counts: { total: 3, done: 3, cancelled: 0, waived: 0 },
});

const taskOpen = makeTask({
  id: "t1",
  project_id: PROJECT_ID,
  epic_id: "e1",
  title: "Define damage types",
  status: "open",
  phase: "planning",
  type_key: "code",
});

const taskDone = makeTask({
  id: "t2",
  project_id: PROJECT_ID,
  epic_id: "e1",
  title: "Implement armor calculation",
  status: "done",
  phase: "complete",
  type_key: "code",
});

function goalListHandler() {
  return http.get(`/api/v1/projects/${PROJECT_ID}/goals`, () =>
    HttpResponse.json(makePage([goalOpen, goalDone])),
  );
}

function epicListHandler() {
  return http.get(`*/api/v1/projects/:projectId/epics`, () =>
    HttpResponse.json(makePage([epicActive, epicDone])),
  );
}

function taskListHandler() {
  return http.get(`*/api/v1/projects/:projectId/tasks`, () =>
    HttpResponse.json(makePage([taskOpen, taskDone])),
  );
}

function renderTabsView(hideCompleted = false) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  server.use(goalListHandler(), epicListHandler(), taskListHandler());
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <TabsView projectId={PROJECT_ID} hideCompleted={hideCompleted} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("TabsView", () => {
  it("renders goal rows", async () => {
    renderTabsView();
    expect(await screen.findByText(/Combat System/)).toBeInTheDocument();
    expect(screen.getByText(/Economy/)).toBeInTheDocument();
  });

  it("expands goal to show epics", async () => {
    renderTabsView();
    const user = userEvent.setup();

    await screen.findByText(/Combat System/);
    const expandButtons = screen.getAllByRole("button", { name: "Expand goal" });
    await user.click(expandButtons[0]);

    expect(await screen.findByText(/Damage Model/)).toBeInTheDocument();
    expect(screen.getByText(/Targeting System/)).toBeInTheDocument();
  });

  it("expands epic to show tasks", async () => {
    renderTabsView();
    const user = userEvent.setup();

    await screen.findByText(/Combat System/);
    const goalExpand = screen.getAllByRole("button", { name: "Expand goal" });
    await user.click(goalExpand[0]);

    const epicExpands = await screen.findAllByRole("button", { name: "Expand epic" });
    await user.click(epicExpands[0]);

    expect(await screen.findByText(/Define damage types/)).toBeInTheDocument();
  });

  it("hides completed goals when hideCompleted is true", async () => {
    renderTabsView(true);
    expect(await screen.findByText(/Combat System/)).toBeInTheDocument();
    expect(screen.queryByText("Economy")).not.toBeInTheDocument();
  });

  it("goal title links to goal detail", async () => {
    renderTabsView();
    const link = await screen.findByRole("link", { name: /Combat System/ });
    expect(link).toHaveAttribute("href", "/projects/p1/goals/g1");
  });

  it("epic title links to epic detail", async () => {
    renderTabsView();
    const user = userEvent.setup();

    await screen.findByText(/Combat System/);
    const expandButtons = screen.getAllByRole("button", { name: "Expand goal" });
    await user.click(expandButtons[0]);

    const link = await screen.findByRole("link", { name: /Damage Model/ });
    expect(link).toHaveAttribute("href", "/projects/p1/epics/e1");
  });

  it("task title links to task detail", async () => {
    renderTabsView();
    const user = userEvent.setup();

    await screen.findByText(/Combat System/);
    const goalExpand = screen.getAllByRole("button", { name: "Expand goal" });
    await user.click(goalExpand[0]);

    const epicExpands = await screen.findAllByRole("button", { name: "Expand epic" });
    await user.click(epicExpands[0]);

    const link = await screen.findByRole("link", { name: /Define damage types/ });
    expect(link).toHaveAttribute("href", "/projects/p1/tasks/t1");
  });

  it("shows loading state while data fetches", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    server.use(
      http.get(`/api/v1/projects/${PROJECT_ID}/goals`, async () => {
        await new Promise((resolve) => setTimeout(resolve, 100));
        return HttpResponse.json(makePage([goalOpen]));
      }),
    );
    render(
      <QueryClientProvider client={queryClient}>
        <MemoryRouter>
          <TabsView projectId={PROJECT_ID} hideCompleted={false} />
        </MemoryRouter>
      </QueryClientProvider>,
    );

    expect(screen.getByRole("status")).toBeInTheDocument();
  });
});
