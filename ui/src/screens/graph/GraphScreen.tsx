import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  type NodeMouseHandler,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "../../styles/reactflow.css";
import { useRef } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { useListProjectRelationsInfinite } from "../../api/generated/relations/relations";
import { useListTasksInfinite } from "../../api/generated/tasks/tasks";
import { cursorPaging, flattenPages, useAllPages } from "../../api/paging";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { decompositionGraph, dependencyGraph, type TaskNodeType } from "../../lib/graphLayout";
import styles from "./GraphScreen.module.css";
import { TaskNode } from "./TaskNode";

type Lens = "tree" | "flow";

const LENSES: Lens[] = ["tree", "flow"];

const LENS_LABELS: Record<Lens, string> = {
  tree: "Decomposition",
  flow: "Dependency flow",
};

// Module-level so React Flow sees a stable identity across renders.
const nodeTypes = { task: TaskNode };

/**
 * The heart of shepherd (docs/04-ui.md): two lenses over the task graph.
 * Decomposition answers "what is this project made of?", dependency flow
 * answers "what was done, what's next, what's blocked on what?" — one
 * canvas, toggled, deep-linkable via ?lens=. Node selection lives in
 * ?selected= so it survives the toggle (node ids are task ids in both
 * lenses).
 */
export function GraphScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <ProjectGraph projectId={projectId} />;
}

function ProjectGraph({ projectId }: { projectId: string }) {
  const [searchParams, setSearchParams] = useSearchParams();
  const lens: Lens = searchParams.get("lens") === "flow" ? "flow" : "tree";
  const selectedId = searchParams.get("selected");
  const tabRefs = useRef(new Map<Lens, HTMLButtonElement>());

  // The graph wants the whole project, not a page: drain both feeds.
  const tasksQuery = useListTasksInfinite(projectId, { limit: 100 }, { query: cursorPaging() });
  const relationsQuery = useListProjectRelationsInfinite(
    projectId,
    { limit: 100 },
    { query: cursorPaging() },
  );
  useAllPages(tasksQuery);
  useAllPages(relationsQuery);

  const setLens = (next: Lens) => {
    setSearchParams((params) => {
      params.set("lens", next);
      return params;
    });
  };

  // WAI-ARIA tabs, automatic activation with roving tabindex — the same
  // pattern as ReviewQueueScreen's queues.
  const onTablistKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    const current = event.currentTarget.id.replace("graph-tab-", "") as Lens;
    const index = LENSES.indexOf(current);
    let next: Lens | undefined;
    if (event.key === "ArrowRight") next = LENSES[(index + 1) % LENSES.length];
    else if (event.key === "ArrowLeft") next = LENSES[(index - 1 + LENSES.length) % LENSES.length];
    else if (event.key === "Home") next = LENSES[0];
    else if (event.key === "End") next = LENSES[LENSES.length - 1];
    if (next === undefined || next === current) return;
    event.preventDefault();
    setLens(next);
    tabRefs.current.get(next)?.focus();
  };

  const tabProps = (value: Lens) => ({
    type: "button" as const,
    role: "tab",
    id: `graph-tab-${value}`,
    "aria-selected": lens === value,
    "aria-controls": `graph-panel-${value}`,
    tabIndex: lens === value ? 0 : -1,
    className: lens === value ? styles.tabActive : styles.tab,
    onKeyDown: onTablistKeyDown,
    ref: (el: HTMLButtonElement | null) => {
      if (el) tabRefs.current.set(value, el);
      else tabRefs.current.delete(value);
    },
    onClick: () => setLens(value),
  });

  const onNodeClick: NodeMouseHandler<TaskNodeType> = (_event, node) => {
    setSearchParams(
      (params) => {
        params.set("selected", node.id);
        return params;
      },
      { replace: true },
    );
  };

  const onPaneClick = () => {
    if (selectedId === null) return;
    setSearchParams(
      (params) => {
        params.delete("selected");
        return params;
      },
      { replace: true },
    );
  };

  const body = () => {
    if (tasksQuery.isPending || relationsQuery.isPending) {
      return <LoadingState label="Loading graph…" />;
    }
    if (tasksQuery.isError) {
      return <ErrorState error={tasksQuery.error} onRetry={() => void tasksQuery.refetch()} />;
    }
    if (relationsQuery.isError) {
      return (
        <ErrorState error={relationsQuery.error} onRetry={() => void relationsQuery.refetch()} />
      );
    }

    const tasks = flattenPages(tasksQuery.data?.pages);
    const relations = flattenPages(relationsQuery.data?.pages);
    if (tasks.length === 0) {
      return (
        <EmptyState title="No tasks yet">
          <p>
            <Link to={`/projects/${projectId}/tasks/new`}>Create the first task</Link> to grow the
            graph.
          </p>
        </EmptyState>
      );
    }

    const graph = lens === "tree" ? decompositionGraph : dependencyGraph;
    const { nodes, edges } = graph(tasks, relations);
    const nodesWithSelection =
      selectedId === null
        ? nodes
        : nodes.map((node) => (node.id === selectedId ? { ...node, selected: true } : node));

    return (
      <div
        role="tabpanel"
        id={`graph-panel-${lens}`}
        aria-labelledby={`graph-tab-${lens}`}
        className={styles.canvas}
      >
        {/* key={lens} remounts the canvas per lens so fitView reframes the
            new layout; live data updates never re-fit. */}
        <ReactFlow
          key={lens}
          aria-label={`Task graph, ${LENS_LABELS[lens]} lens`}
          nodes={nodesWithSelection}
          edges={edges}
          nodeTypes={nodeTypes}
          fitView
          fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
          minZoom={0.1}
          nodesDraggable={false}
          nodesConnectable={false}
          deleteKeyCode={null}
          onNodeClick={onNodeClick}
          onPaneClick={onPaneClick}
        >
          <Background variant={BackgroundVariant.Dots} gap={24} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
    );
  };

  return (
    <section className={styles.screen}>
      <PageHeader
        title="Graph"
        description="Two lenses over the task graph: how the goal decomposes, and what flows into what."
      >
        <div role="tablist" aria-label="Graph lenses" className={styles.tabs} tabIndex={-1}>
          <button {...tabProps("tree")}>{LENS_LABELS.tree}</button>
          <button {...tabProps("flow")}>{LENS_LABELS.flow}</button>
        </div>
      </PageHeader>
      {body()}
    </section>
  );
}
