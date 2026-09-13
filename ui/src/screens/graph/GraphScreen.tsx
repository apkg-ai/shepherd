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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  // Memoized: a fresh Set per render would defeat the layout memo below.
  const parsedExpanded = useMemo(() => parseExpanded(searchParams), [searchParams]);
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

  /**
   * The effective expansion for URL updates. With no `?expanded=` param the
   * live state is the default (roots expanded in the tree lens) — seeding
   * from an empty set would collapse every other root on the first chip
   * click.
   */
  const seedExpanded = useCallback(
    (params: URLSearchParams): Set<string> =>
      parseExpanded(params) ??
      (lens === "tree"
        ? defaultExpanded(
            flattenPages(tasksQuery.data?.pages),
            flattenPages(relationsQuery.data?.pages),
          )
        : new Set<string>()),
    [lens, tasksQuery.data, relationsQuery.data],
  );

  const toggleExpand = useCallback(
    (taskId: string) => {
      setSearchParams(
        (params) => {
          const expanded = seedExpanded(params);
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
    [setSearchParams, seedExpanded],
  );

  /**
   * Expand a parent and select + zoom to one of its children. Every
   * decomposition ancestor up to the root is expanded too — a collapsed
   * grandparent would keep the child hidden and turn the fitView into a
   * silent no-op.
   */
  const expandAndFocus = useCallback(
    (parentId: string, childId: string) => {
      setSearchParams(
        (params) => {
          const expanded = seedExpanded(params);
          const relations = flattenPages(relationsQuery.data?.pages);
          const parentOf = new Map<string, string>();
          for (const r of relations) {
            if (r.type === "decomposition" && r.source_task_id !== r.target_task_id) {
              parentOf.set(r.target_task_id, r.source_task_id);
            }
          }
          // Walk to the root; `seen` stops malformed parent cycles.
          const seen = new Set<string>();
          let current: string | undefined = parentId;
          while (current !== undefined && !seen.has(current)) {
            seen.add(current);
            expanded.add(current);
            current = parentOf.get(current);
          }
          params.set("expanded", [...expanded].join(","));
          params.set("selected", childId);
          return params;
        },
        { replace: true },
      );
      setFocusTarget(childId);
    },
    [setSearchParams, seedExpanded, relationsQuery.data],
  );

  const clearFocusTarget = useCallback(() => {
    setFocusTarget(null);
  }, []);

  const onNodeClick: NodeMouseHandler<TaskNodeType | BoundaryNodeType> = (_event, node) => {
    // Boundary nodes are virtual: selecting one would fade the whole canvas
    // (no relation references a boundary id) with no side panel to explain it.
    if (node.type === "boundary") return;
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

  // Memoized: a fresh object per render would re-render every TaskNode
  // consuming the context on each selection or live-data update.
  const graphActions = useMemo(() => ({ toggleExpand, lens }), [toggleExpand, lens]);

  // Memoized feeds and layout: dagre runs only when the data, lens, or
  // expansion actually changes — not on every selection or SSE-driven
  // render. Fresh node/edge identities per render would defeat React
  // Flow's memoization and re-render every TaskNode.
  const tasks = useMemo(() => flattenPages(tasksQuery.data?.pages), [tasksQuery.data]);
  const relations = useMemo(() => flattenPages(relationsQuery.data?.pages), [relationsQuery.data]);

  // Resolve expansion: null = no URL param → lens-dependent default.
  // Tree lens: roots expanded so epic-level structure is visible.
  // Flow lens: nothing expanded — start with epics only, drill in via chip.
  const expandedIds = useMemo(
    () =>
      parsedExpanded ?? (lens === "tree" ? defaultExpanded(tasks, relations) : new Set<string>()),
    [parsedExpanded, lens, tasks, relations],
  );

  const graph = useMemo(() => {
    // Child counts: used for the expand chip's count display and fallback
    // epic detection (projects with no first-class epics yet).
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
      // Flow lens: show top-level epics only — nested child epics (epics
      // that are decomposition children of another epic) belong in the
      // side panel, not as independent flow nodes. If no epics exist,
      // fall back to the old heuristic (tasks with children) and then to
      // all tasks — flow is the default lens, so it must never render a
      // blank canvas. Inline subtask expansion tracked in #52.
      const decompChildIds = new Set(
        relations.filter((r) => r.type === "decomposition").map((r) => r.target_task_id),
      );
      let epicTasks = tasks.filter((t) => t.type === "epic" && !decompChildIds.has(t.id));
      if (epicTasks.length === 0) {
        epicTasks = tasks.filter((t) => childCounts.has(t.id) && !decompChildIds.has(t.id));
      }
      const flowTasks = epicTasks.length > 0 ? epicTasks : tasks;
      const flowMeta = new Map(
        flowTasks.map((t) => [t.id, { childCount: childCounts.get(t.id) ?? 0, expanded: false }]),
      );
      graphNodes = dependencyGraph(flowTasks, relations, flowMeta);
    }
    let { edges } = graphNodes;
    // Boundary nodes join in the flow lens (below), so the array holds both.
    let nodes: (TaskNodeType | BoundaryNodeType)[] = graphNodes.nodes;

    // Flow lens: inject virtual Start/End boundary nodes so the graph
    // reads as a complete traversal from a single entry to a single exit.
    // A boundary whose group is empty (e.g. an epic-level depends_on cycle)
    // is skipped — a centroid over zero nodes would be NaN.
    if (lens === "flow" && nodes.length > 0) {
      const incomingTargets = new Set(edges.map((e) => e.target));
      const outgoingSources = new Set(edges.map((e) => e.source));
      const startEpics = nodes.filter((n) => !incomingTargets.has(n.id));
      const endEpics = nodes.filter((n) => !outgoingSources.has(n.id));

      // TB layout: Start above, End below. Center horizontally over the group.
      const minY = Math.min(...nodes.map((n) => n.position.y));
      const maxY = Math.max(...nodes.map((n) => n.position.y));
      const centroidX = (group: readonly (TaskNodeType | BoundaryNodeType)[]) =>
        group.reduce((sum, n) => sum + n.position.x + NODE_WIDTH / 2, 0) / group.length -
        BOUNDARY_SIZE / 2;

      if (startEpics.length > 0) {
        nodes = [
          {
            id: START_ID,
            type: "boundary",
            data: { label: "Start" },
            position: { x: centroidX(startEpics), y: minY - BOUNDARY_SIZE - BOUNDARY_GAP },
            width: BOUNDARY_SIZE,
            height: BOUNDARY_SIZE,
            draggable: false,
            connectable: false,
          },
          ...nodes,
        ];
        edges = [
          ...edges,
          ...startEpics.map((n) => ({
            id: `${START_ID}-${n.id}`,
            source: START_ID,
            target: n.id,
            type: "smoothstep" as const,
          })),
        ];
      }
      if (endEpics.length > 0) {
        nodes = [
          ...nodes,
          {
            id: END_ID,
            type: "boundary",
            data: { label: "End" },
            position: { x: centroidX(endEpics), y: maxY + NODE_HEIGHT + BOUNDARY_GAP },
            width: BOUNDARY_SIZE,
            height: BOUNDARY_SIZE,
            draggable: false,
            connectable: false,
          },
        ];
        edges = [
          ...edges,
          ...endEpics.map((n) => ({
            id: `${n.id}-${END_ID}`,
            source: n.id,
            target: END_ID,
            type: "smoothstep" as const,
          })),
        ];
      }
    }

    return { nodes, edges };
  }, [lens, tasks, relations, expandedIds]);

  // Display-only waiting indicators — no lifecycle changes.
  const { epicBlockedIds, depWaitingIds } = useMemo(() => {
    const epicBlocked = new Set<string>();
    const depWaiting = new Set<string>();
    const taskById = new Map(tasks.map((t) => [t.id, t]));

    for (const r of relations) {
      // Epic blocked: child epic waiting on parent's non-epic subtasks.
      if (r.type === "decomposition") {
        const parent = taskById.get(r.source_task_id);
        const child = taskById.get(r.target_task_id);
        if (parent?.type === "epic" && child?.type === "epic") {
          const hasUnfinishedWork = relations.some((rel) => {
            if (rel.type !== "decomposition" || rel.source_task_id !== parent.id) return false;
            const sibling = taskById.get(rel.target_task_id);
            return sibling && sibling.type !== "epic" && sibling.status !== "done" && sibling.status !== "cancelled";
          });
          if (hasUnfinishedWork) epicBlocked.add(child.id);
        }
      }

      // Dependency waiting: task in approved with an unmet depends_on.
      if (r.type === "depends_on") {
        const source = taskById.get(r.source_task_id);
        const target = taskById.get(r.target_task_id);
        if (source && source.status === "approved" && target && target.status !== "done") {
          depWaiting.add(source.id);
        }
      }
    }
    return { epicBlockedIds: epicBlocked, depWaitingIds: depWaiting };
  }, [tasks, relations]);

  // Neighbor fade: when a node is selected, compute the transitive cone and
  // dim everything outside it. The lens type determines which relation edges
  // define "neighbor." Only a selection that is a task node on the canvas
  // fades — an off-canvas selection (a subtask via the side panel in the
  // flow lens) has no rendered neighbors, and fading everything would
  // spotlight nothing.
  const lensType = lens === "tree" ? "decomposition" : "depends_on";
  const neighborIds = useMemo(
    () =>
      selectedId !== null && graph.nodes.some((n) => n.id === selectedId && n.type !== "boundary")
        ? computeNeighborIds(selectedId, relations, lensType)
        : null,
    [selectedId, graph.nodes, relations, lensType],
  );

  const nodesWithState = useMemo(
    () =>
      graph.nodes.map((node) => {
        const isSelected = node.id === selectedId;
        const isBoundary = node.id === START_ID || node.id === END_ID;
        const faded = !isBoundary && neighborIds !== null && !neighborIds.has(node.id);
        const epicBlocked = !isBoundary && epicBlockedIds.has(node.id);
        const depWaiting = !isBoundary && depWaitingIds.has(node.id);
        if (!isSelected && !faded && !epicBlocked && !depWaiting) return node;
        // Boundary nodes never fade; only the selection flag can change.
        if (node.type === "boundary") return { ...node, selected: isSelected || undefined };
        return {
          ...node,
          selected: isSelected || undefined,
          data: { ...node.data, faded, epicBlocked, depWaiting },
        };
      }),
    [graph.nodes, selectedId, neighborIds, epicBlockedIds, depWaitingIds],
  );

  // Fade edges: full opacity on edges between two neighbors, dim otherwise.
  const edgesWithFade = useMemo(
    () =>
      neighborIds === null
        ? graph.edges
        : graph.edges.map((edge) => {
            const srcIn = neighborIds.has(edge.source);
            const tgtIn = neighborIds.has(edge.target);
            if (srcIn && tgtIn) return edge;
            return {
              ...edge,
              style: { ...edge.style, opacity: srcIn || tgtIn ? 0.3 : 0.08 },
            };
          }),
    [graph.edges, neighborIds],
  );

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

    const selectedTask = selectedId !== null ? tasks.find((t) => t.id === selectedId) : undefined;

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
