import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  useReactFlow,
  type NodeMouseHandler,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "../../styles/reactflow.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { useListProjectRelationsInfinite } from "../../api/generated/relations/relations";
import { useListTasksInfinite } from "../../api/generated/tasks/tasks";
import { cursorPaging, flattenPages, useAllPages } from "../../api/paging";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import {
  BOUNDARY_SIZE,
  NODE_HEIGHT,
  NODE_WIDTH,
  decompositionGraph,
  dependencyGraph,
  type BoundaryNodeType,
  type TaskNodeType,
} from "../../lib/graphLayout";
import { computeNeighborIds } from "../../lib/graphNeighbors";
import { BoundaryNode } from "./BoundaryNode";
import { GraphActionsContext } from "./GraphActions";
import styles from "./GraphScreen.module.css";
import { GraphSidePanel } from "./GraphSidePanel";
import { TaskNode } from "./TaskNode";

type Lens = "tree" | "flow";

const LENSES: Lens[] = ["tree", "flow"];

const LENS_LABELS: Record<Lens, string> = {
  tree: "Decomposition",
  flow: "Dependency flow",
};

// Module-level so React Flow sees a stable identity across renders.
const nodeTypes = { task: TaskNode, boundary: BoundaryNode };

const START_ID = "__start__";
const END_ID = "__end__";
const BOUNDARY_GAP = 80;

/**
 * Parse `?expanded=a,b,c` into a Set.
 * - Param absent → `null` (caller should compute the default: roots expanded).
 * - Param present but empty (`?expanded=`) → empty Set (user collapsed everything).
 * - Param with values → those IDs.
 */
function parseExpanded(params: URLSearchParams): Set<string> | null {
  if (!params.has("expanded")) return null;
  const raw = params.get("expanded")!;
  return raw ? new Set(raw.split(",").filter(Boolean)) : new Set();
}

/**
 * Default expansion: root nodes (tasks with no decomposition parent) are
 * expanded so the epic-level structure is visible on first load. This gives
 * an overview-first reading pattern — epics → drill-in.
 */
function defaultExpanded(
  tasks: readonly { id: string }[],
  relations: readonly { type: string; target_task_id: string }[],
): Set<string> {
  const hasParent = new Set<string>();
  for (const r of relations) {
    if (r.type === "decomposition") hasParent.add(r.target_task_id);
  }
  const roots = new Set<string>();
  for (const t of tasks) {
    if (!hasParent.has(t.id)) roots.add(t.id);
  }
  return roots;
}

/**
 * Child of `<ReactFlow>`: after an expand-and-focus, pans the camera to the
 * newly revealed node. Renders nothing — runs an effect only.
 */
function FitToNode({ nodeId, onDone }: { nodeId: string | null; onDone: () => void }) {
  const { fitView } = useReactFlow();
  useEffect(() => {
    if (!nodeId) return;
    // Let the new nodes settle in the DOM before measuring.
    const frame = requestAnimationFrame(() => {
      fitView({ nodes: [{ id: nodeId }], duration: 300, padding: 0.5 });
      onDone();
    });
    return () => cancelAnimationFrame(frame);
  }, [nodeId, fitView, onDone]);
  return null;
}

/**
 * The heart of shepherd (docs/04-ui.md): two lenses over the task graph.
 * Decomposition answers "what is this project made of?", dependency flow
 * answers "what was done, what's next, what's blocked on what?" — one
 * canvas, toggled, deep-linkable via ?lens=. Node selection lives in
 * ?selected= so it survives the toggle (node ids are task ids in both
 * lenses).
 *
 * In the tree lens, subtrees collapse by default — only root/epic nodes
 * appear until a user clicks the expand chip. Expansion state is stored in
 * ?expanded= to support deep links and lens switching.
 *
 * Both lenses support neighbor fade: on selection, non-neighboring nodes
 * (outside the transitive dependency/decomposition cone) dim, spotlighting
 * the blocking chain.
 */
export function GraphScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <ProjectGraph projectId={projectId} />;
}

function ProjectGraph({ projectId }: { projectId: string }) {
  const [searchParams, setSearchParams] = useSearchParams();
  const lens: Lens = searchParams.get("lens") === "tree" ? "tree" : "flow";
  const selectedId = searchParams.get("selected");
  const parsedExpanded = parseExpanded(searchParams);
  const tabRefs = useRef(new Map<Lens, HTMLButtonElement>());
  const [focusTarget, setFocusTarget] = useState<string | null>(null);

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

  const select = (taskId: string) => {
    setSearchParams(
      (params) => {
        params.set("selected", taskId);
        return params;
      },
      { replace: true },
    );
  };

  const toggleExpand = useCallback(
    (taskId: string) => {
      setSearchParams(
        (params) => {
          const expanded = parseExpanded(params) ?? new Set<string>();
          if (expanded.has(taskId)) expanded.delete(taskId);
          else expanded.add(taskId);
          // Always set explicitly — even empty — so the default (roots
          // expanded) is only used on truly fresh loads without a param.
          params.set("expanded", [...expanded].join(","));
          return params;
        },
        { replace: true },
      );
    },
    [setSearchParams],
  );

  /** Expand a parent and select + zoom to one of its children. */
  const expandAndFocus = useCallback(
    (parentId: string, childId: string) => {
      setSearchParams(
        (params) => {
          const expanded = parseExpanded(params) ?? new Set<string>();
          expanded.add(parentId);
          params.set("expanded", [...expanded].join(","));
          params.set("selected", childId);
          return params;
        },
        { replace: true },
      );
      setFocusTarget(childId);
    },
    [setSearchParams],
  );

  const clearFocusTarget = useCallback(() => {
    setFocusTarget(null);
  }, []);

  const onNodeClick: NodeMouseHandler<TaskNodeType> = (_event, node) => {
    select(node.id);
  };

  const clearSelection = () => {
    if (selectedId === null) return;
    setSearchParams(
      (params) => {
        params.delete("selected");
        return params;
      },
      { replace: true },
    );
  };

  const graphActions = { toggleExpand, lens };

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

    // Resolve expansion: null = no URL param → default to roots expanded
    // so the epic-level structure is visible on first load.
    const expandedIds = parsedExpanded ?? defaultExpanded(tasks, relations);

    // Epic detection: tasks that have at least one decomposition child.
    // Used by the flow lens to show only epics (subtasks are hidden).
    const childCounts = new Map<string, number>();
    for (const r of relations) {
      if (r.type === "decomposition") {
        childCounts.set(r.source_task_id, (childCounts.get(r.source_task_id) ?? 0) + 1);
      }
    }

    let graphNodes: ReturnType<typeof decompositionGraph>;
    if (lens === "tree") {
      graphNodes = decompositionGraph(tasks, relations, expandedIds);
    } else {
      // Flow lens: show only epics (tasks with decomposition children).
      // Subtask-level edges vanish automatically via lensEdges endpoint filter.
      const epicTasks = tasks.filter((t) => childCounts.has(t.id));
      const flowMeta = new Map(
        epicTasks.map((t) => [
          t.id,
          { childCount: childCounts.get(t.id) ?? 0, expanded: false },
        ]),
      );
      graphNodes = dependencyGraph(epicTasks, relations, flowMeta);
    }
    let { nodes, edges } = graphNodes;

    // Flow lens: inject virtual Start/End boundary nodes so the graph
    // reads as a complete traversal from a single entry to a single exit.
    if (lens === "flow" && nodes.length > 0) {
      const incomingTargets = new Set(edges.map((e) => e.target));
      const outgoingSources = new Set(edges.map((e) => e.source));
      const startEpics = nodes.filter((n) => !incomingTargets.has(n.id));
      const endEpics = nodes.filter((n) => !outgoingSources.has(n.id));

      // TB layout: Start above, End below. Center horizontally over the group.
      const minY = Math.min(...nodes.map((n) => n.position.y));
      const maxY = Math.max(...nodes.map((n) => n.position.y));
      const centroidX = (group: typeof nodes) =>
        group.reduce((sum, n) => sum + n.position.x + NODE_WIDTH / 2, 0) / group.length -
        BOUNDARY_SIZE / 2;

      const startNode: BoundaryNodeType = {
        id: START_ID,
        type: "boundary",
        data: { label: "Start" },
        position: { x: centroidX(startEpics), y: minY - BOUNDARY_SIZE - BOUNDARY_GAP },
        width: BOUNDARY_SIZE,
        height: BOUNDARY_SIZE,
        draggable: false,
        connectable: false,
      };
      const endNode: BoundaryNodeType = {
        id: END_ID,
        type: "boundary",
        data: { label: "End" },
        position: { x: centroidX(endEpics), y: maxY + NODE_HEIGHT + BOUNDARY_GAP },
        width: BOUNDARY_SIZE,
        height: BOUNDARY_SIZE,
        draggable: false,
        connectable: false,
      };

      const boundaryEdges = [
        ...startEpics.map((n) => ({
          id: `${START_ID}-${n.id}`,
          source: START_ID,
          target: n.id,
          type: "smoothstep" as const,
        })),
        ...endEpics.map((n) => ({
          id: `${n.id}-${END_ID}`,
          source: n.id,
          target: END_ID,
          type: "smoothstep" as const,
        })),
      ];

      nodes = [startNode as unknown as TaskNodeType, ...nodes, endNode as unknown as TaskNodeType];
      edges = [...edges, ...boundaryEdges];
    }

    const selectedTask = selectedId !== null ? tasks.find((t) => t.id === selectedId) : undefined;

    // Neighbor fade: when a node is selected, compute the transitive cone and
    // dim everything outside it. The lens type determines which relation edges
    // define "neighbor."
    const lensType = lens === "tree" ? "decomposition" : "depends_on";
    const neighborIds =
      selectedId !== null ? computeNeighborIds(selectedId, relations, lensType) : null;

    const nodesWithState = nodes.map((node) => {
      const isSelected = node.id === selectedId;
      const isBoundary = node.id === START_ID || node.id === END_ID;
      const faded = !isBoundary && neighborIds !== null && !neighborIds.has(node.id);
      if (!isSelected && !faded) return node;
      return {
        ...node,
        selected: isSelected || undefined,
        data: { ...node.data, faded },
      };
    });

    // Fade edges: full opacity on edges between two neighbors, dim otherwise.
    const edgesWithFade =
      neighborIds === null
        ? edges
        : edges.map((edge) => {
            const srcIn = neighborIds.has(edge.source);
            const tgtIn = neighborIds.has(edge.target);
            if (srcIn && tgtIn) return edge;
            return {
              ...edge,
              style: { ...edge.style, opacity: srcIn || tgtIn ? 0.3 : 0.08 },
            };
          });

    return (
      <div
        role="tabpanel"
        id={`graph-panel-${lens}`}
        aria-labelledby={`graph-tab-${lens}`}
        className={styles.layout}
      >
        <div className={styles.canvas}>
          {/* key={lens} remounts the canvas per lens so fitView reframes the
              new layout; live data updates never re-fit. */}
          <GraphActionsContext.Provider value={graphActions}>
            <ReactFlow
              key={lens}
              aria-label={`Task graph, ${LENS_LABELS[lens]} lens`}
              nodes={nodesWithState}
              edges={edgesWithFade}
              nodeTypes={nodeTypes}
              fitView
              fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
              minZoom={0.1}
              nodesDraggable={false}
              nodesConnectable={false}
              deleteKeyCode={null}
              onNodeClick={onNodeClick}
              onPaneClick={clearSelection}
            >
              <Background variant={BackgroundVariant.Dots} gap={24} />
              <Controls showInteractive={false} />
              <FitToNode nodeId={focusTarget} onDone={clearFocusTarget} />
            </ReactFlow>
          </GraphActionsContext.Provider>
        </div>
        {selectedTask !== undefined ? (
          <GraphSidePanel
            task={selectedTask}
            projectId={projectId}
            tasks={tasks}
            relations={relations}
            lens={lens}
            onSelect={select}
            onExpandAndFocus={expandAndFocus}
            onClose={clearSelection}
          />
        ) : null}
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
