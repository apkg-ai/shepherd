import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../lib/queryClient";
import { ToastProvider } from "../../components/Toast";
import { makeGraphNode } from "../../test/fixtures";
import { createDependencyHandler, deleteDependencyHandler, problemResponse, server } from "../../test/msw";
import { http } from "msw";
import { GraphActions } from "./GraphActions";

const nodeA = makeGraphNode({ id: "a1", title: "Node A" });
const nodeB = makeGraphNode({ id: "b1", title: "Node B" });
const nodeC = makeGraphNode({ id: "c1", title: "Node C" });
const nodes = [nodeA, nodeB, nodeC];

function Wrapper({ children }: { children: ReactNode }) {
  const [client] = useState(() => {
    const qc = createQueryClient();
    qc.setDefaultOptions({
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false },
    });
    return qc;
  });
  return (
    <MemoryRouter>
      <ToastProvider>
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      </ToastProvider>
    </MemoryRouter>
  );
}

function renderActions(overrides: Partial<Parameters<typeof GraphActions>[0]> = {}) {
  const props = {
    projectId: "p1",
    level: "epic" as const,
    nodes,
    selectedNodeId: "a1",
    addDialogOpen: false,
    onAddDialogClose: vi.fn(),
    removeEdge: null,
    onRemoveDialogClose: vi.fn(),
    onSuccess: vi.fn(),
    ...overrides,
  };
  const result = render(<GraphActions {...props} />, { wrapper: Wrapper });
  return { ...result, props };
}

describe("GraphActions — add dependency", () => {
  it("renders add dialog when open", () => {
    renderActions({ addDialogOpen: true });

    expect(screen.getByRole("dialog", { name: "Add dependency" })).toBeInTheDocument();
  });

  it("shows selected node as dependent", () => {
    renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    expect(screen.getByText("Node A")).toBeInTheDocument();
  });

  it("lists other nodes as prerequisite candidates", () => {
    renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    const select = screen.getByLabelText("This item requires:");
    const options = select.querySelectorAll("option:not([disabled])");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("Node B");
    expect(options[1]).toHaveTextContent("Node C");
  });

  it("excludes selected node from candidates", () => {
    renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    const select = screen.getByLabelText("This item requires:");
    const optionTexts = [...select.querySelectorAll("option:not([disabled])")].map(
      (o) => o.textContent,
    );
    expect(optionTexts).not.toContain("Node A");
  });

  it("submits create dependency", async () => {
    const user = userEvent.setup();
    let capturedBody: unknown;

    server.use(
      http.post("*/api/v1/projects/p1/dependencies", async ({ request }) => {
        capturedBody = await request.json();
        return new Response(JSON.stringify({ id: "d1", revision: 1, created_at: "", updated_at: "", project_id: "p1", level: "epic", dependent_id: "a1", prerequisite_id: "b1" }), { status: 201, headers: { "Content-Type": "application/json" } });
      }),
    );

    renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    await user.selectOptions(screen.getByLabelText("This item requires:"), "b1");
    await user.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => {
      expect(capturedBody).toMatchObject({
        level: "epic",
        dependent_id: "a1",
        prerequisite_id: "b1",
      });
    });
  });

  it("calls onSuccess after create", async () => {
    const user = userEvent.setup();
    server.use(createDependencyHandler());

    const { props } = renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    await user.selectOptions(screen.getByLabelText("This item requires:"), "b1");
    await user.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => expect(props.onSuccess).toHaveBeenCalledOnce());
    expect(props.onAddDialogClose).toHaveBeenCalledOnce();
  });

  it("keeps dialog open on error", async () => {
    const user = userEvent.setup();
    server.use(
      http.post("*/api/v1/projects/p1/dependencies", () => problemResponse(409, "conflict")),
    );

    const { props } = renderActions({ addDialogOpen: true, selectedNodeId: "a1" });

    await user.selectOptions(screen.getByLabelText("This item requires:"), "b1");
    await user.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => expect(props.onSuccess).not.toHaveBeenCalled());
    expect(screen.getByRole("dialog", { name: "Add dependency" })).toBeInTheDocument();
  });
});

describe("GraphActions — remove dependency", () => {
  const edge = {
    dependencyId: "dep-1",
    dependentTitle: "Node A",
    prerequisiteTitle: "Node B",
  };

  it("renders remove dialog when edge provided", () => {
    renderActions({ removeEdge: edge });

    expect(screen.getByRole("dialog", { name: "Remove dependency" })).toBeInTheDocument();
    expect(
      screen.getByText("Removing this dependency may unblock work that was waiting on it."),
    ).toBeInTheDocument();
    expect(screen.getByText(/Node B/)).toBeInTheDocument();
    expect(screen.getByText(/Node A/)).toBeInTheDocument();
  });

  it("confirms deletion", async () => {
    const user = userEvent.setup();
    let deletedId: string | undefined;

    server.use(
      http.delete("*/api/v1/projects/p1/dependencies/:depId", ({ params }) => {
        deletedId = params.depId as string;
        return new Response(null, { status: 204 });
      }),
    );

    renderActions({ removeEdge: edge });

    await user.click(screen.getByRole("button", { name: "Remove" }));

    await waitFor(() => expect(deletedId).toBe("dep-1"));
  });

  it("calls onSuccess after delete", async () => {
    const user = userEvent.setup();
    server.use(deleteDependencyHandler());

    const { props } = renderActions({ removeEdge: edge });

    await user.click(screen.getByRole("button", { name: "Remove" }));

    await waitFor(() => expect(props.onSuccess).toHaveBeenCalledOnce());
    expect(props.onRemoveDialogClose).toHaveBeenCalledOnce();
  });
});
