import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import type { GraphNodesItem } from "../../api/generated/model/graphNodesItem";
import { makeGraphNode, makeTask } from "../../test/fixtures";
import { server } from "../../test/msw";
import { GraphSidePanel } from "./GraphSidePanel";

interface GraphSidePanelProps {
  node: GraphNodesItem | null;
  level: "epic" | "task";
  projectId: string;
  onClose: () => void;
}

function renderPanel(props: GraphSidePanelProps) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <GraphSidePanel {...props} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const projectId = "p1";

describe("GraphSidePanel", () => {
  it("renders nothing when node is null", () => {
    renderPanel({ node: null, level: "epic", projectId, onClose: vi.fn() });
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
  });

  it("renders node title and status", () => {
    const node = makeGraphNode({ title: "Deploy service", status: "active" });
    renderPanel({ node, level: "epic", projectId, onClose: vi.fn() });

    expect(screen.getByRole("heading", { name: "Deploy service" })).toBeInTheDocument();
    expect(screen.getByRole("complementary")).toBeInTheDocument();
  });

  it("renders progress counts when total > 0", () => {
    const node = makeGraphNode({
      title: "Has counts",
      counts: { total: 5, done: 2, cancelled: 0, waived: 0 },
    });
    renderPanel({ node, level: "epic", projectId, onClose: vi.fn() });

    expect(screen.getByText(/2.*5/)).toBeInTheDocument();
  });

  it("does not render progress counts when total is 0", () => {
    const node = makeGraphNode({
      title: "No counts",
      counts: { total: 0, done: 0, cancelled: 0, waived: 0 },
    });
    renderPanel({ node, level: "epic", projectId, onClose: vi.fn() });

    expect(screen.getByRole("heading", { name: "No counts" })).toBeInTheDocument();
    expect(screen.queryByText(/0.*of.*0/)).not.toBeInTheDocument();
  });

  it("renders open details link for epic level", () => {
    const node = makeGraphNode({ id: "e42", title: "Epic node" });
    renderPanel({ node, level: "epic", projectId, onClose: vi.fn() });

    const link = screen.getByRole("link", { name: "Open details" });
    expect(link).toHaveAttribute("href", "/projects/p1/epics/e42");
  });

  it("renders open details link for task level", () => {
    const node = makeGraphNode({ id: "t99", title: "Task node" });
    server.use(
      http.get("/api/v1/projects/:projectId/tasks/t99", () =>
        HttpResponse.json(makeTask({ id: "t99", project_id: "p1", phase: "execution" })),
      ),
    );
    renderPanel({ node, level: "task", projectId, onClose: vi.fn() });

    const link = screen.getByRole("link", { name: "Open details" });
    expect(link).toHaveAttribute("href", "/projects/p1/tasks/t99");
  });

  it("close button calls onClose", async () => {
    const onClose = vi.fn();
    const node = makeGraphNode({ title: "Closeable" });
    renderPanel({ node, level: "epic", projectId, onClose });

    await userEvent.click(screen.getByRole("button", { name: "Close panel" }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("renders eligibility panel", () => {
    const node = makeGraphNode({
      title: "Eligible",
      eligibility: {
        can_plan: true,
        can_execute: false,
        can_review: false,
        reasons: [],
        allowed_actions: [],
      },
    });
    renderPanel({ node, level: "epic", projectId, onClose: vi.fn() });

    expect(screen.getByRole("region", { name: "Eligibility" })).toBeInTheDocument();
  });
});
