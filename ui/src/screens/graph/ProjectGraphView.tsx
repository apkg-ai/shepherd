import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { ReactFlow, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type {
  Node,
  Edge,
  NodeChange,
  NodeSelectionChange,
  EdgeChange,
  EdgeSelectionChange,
} from "@xyflow/react";
import { useListGoals, useGetGoalGraph } from "../../api/generated/goals/goals";
import type { GraphNodesItem } from "../../api/generated/model/graphNodesItem";
import {
  computeLayout,
  addDagEndpoints,
  computeLayoutWithEndpoints,
  NODE_WIDTH,
  NODE_HEIGHT,
  DAG_START_ID,
  DAG_END_ID,
} from "../../lib/graphLayout";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { GraphNode } from "./GraphNode";
import { GoalGroupNode } from "./GoalGroupNode";
import { DagEndpointNode } from "./DagEndpointNode";
import { GraphSidePanel } from "./GraphSidePanel";
import { GraphToolbar } from "./GraphToolbar";
import { GraphActions } from "./GraphActions";
import { TabsView } from "./TabsView";
import styles from "./ProjectGraphView.module.css";
import type { ReactNode } from "react";

const nodeTypes = { graphNode: GraphNode, goalGroup: GoalGroupNode, dagEndpoint: DagEndpointNode };

const EPIC_PADDING = 20;
const EPIC_HEADER_HEIGHT = 68;

interface ProjectGraphViewProps {
  projectId: string;
  listContent: ReactNode;
}

export function ProjectGraphView(props: ProjectGraphViewProps) {
  return (
    <ReactFlowProvider>
      <ProjectGraphViewInner {...props} />
    </ReactFlowProvider>
  );
}

function ProjectGraphViewInner({ projectId, listContent }: ProjectGraphViewProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { fitView, zoomIn, zoomOut } = useReactFlow();
  const sidePanelRef = useRef<HTMLElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  const [viewMode, setViewMode] = useState<"graph" | "list" | "table">("graph");
  const [hideCompleted, setHideCompleted] = useState(false);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [expandedGoalId, setExpandedGoalId] = useState<string | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [removeEdge, setRemoveEdge] = useState<{
    dependencyId: string;
    dependentTitle: string;
    prerequisiteTitle: string;
  } | null>(null);

  const goalsQuery = useListGoals(projectId);
  const goals = goalsQuery.data?.items;

  const visibleGoals = useMemo(
    () => (goals ?? []).filter((g) => !hideCompleted || !g.completed),
    [goals, hideCompleted],
  );

  // Fetch graph only for the expanded goal
  const expandedGoalGraphQuery = useGetGoalGraph(projectId, expandedGoalId ?? "", {
    query: { enabled: expandedGoalId !== null },
  });
  const expandedGraph = expandedGoalGraphQuery.data;
  const { refetch } = expandedGoalGraphQuery;

  const allEpicNodes = useMemo(() => expandedGraph?.nodes ?? [], [expandedGraph]);
  const allDependencies = useMemo(() => expandedGraph?.dependencies ?? [], [expandedGraph]);

  /* v8 ignore start -- layout and callbacks only exercised with React Flow; covered by E2E */
  const handleToggleGoal = useCallback((goalId: string) => {
    setExpandedGoalId((prev) => (prev === goalId ? null : goalId));
    setSelectedNodeId(null);
    setAddDialogOpen(false);
    setRemoveEdge(null);
  }, []);

  // Build the graph layout — same DAG structure as epic/task views
  const layout = useMemo(() => {
    if (visibleGoals.length === 0) return { nodes: [] as Node[], edges: [] as Edge[] };

    // Pre-compute expanded goal container dimensions BEFORE dagre runs
    let epicLayout: { nodes: Node[]; edges: Edge[] } | null = null;
    let containerWidth = NODE_WIDTH;
    let containerHeight = NODE_HEIGHT;
    let epicMinX = 0;
    let epicMinY = 0;

    if (expandedGoalId && expandedGraph) {
      epicLayout = computeLayout(expandedGraph.nodes, expandedGraph.dependencies);
      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;
      for (const n of epicLayout.nodes) {
        minX = Math.min(minX, n.position.x);
        minY = Math.min(minY, n.position.y);
        maxX = Math.max(maxX, n.position.x + NODE_WIDTH);
        maxY = Math.max(maxY, n.position.y + NODE_HEIGHT);
      }
      if (Number.isFinite(minX)) {
        epicMinX = minX;
        epicMinY = minY;
        containerWidth = Math.max(NODE_WIDTH, maxX - minX + EPIC_PADDING * 2);
        containerHeight = EPIC_HEADER_HEIGHT + (maxY - minY) + EPIC_PADDING;
      }
    }

    // Build goal nodes with CORRECT dimensions (expanded goal gets container size)
    const goalNodes: Node[] = visibleGoals.map((goal) => ({
      id: goal.id,
      type: "goalGroup",
      position: { x: 0, y: 0 },
      ariaLabel: t("graph.group.label", { title: goal.title }),
      data: {
        title: goal.title,
        completed: goal.completed,
        epicCounts: goal.epic_counts,
        expanded: goal.id === expandedGoalId,
        onToggleExpand: handleToggleGoal,
        goalId: goal.id,
      },
      style: {
        width: goal.id === expandedGoalId ? containerWidth : NODE_WIDTH,
        height: goal.id === expandedGoalId ? containerHeight : NODE_HEIGHT,
      },
    }));

    // Dagre now computes positions using the ACTUAL dimensions
    const withEndpoints = addDagEndpoints(goalNodes, []);
    const dagLayout = computeLayoutWithEndpoints(withEndpoints.nodes, withEndpoints.edges);

    // Inject epic children inside the expanded container
    if (expandedGoalId && epicLayout) {
      for (const n of epicLayout.nodes) {
        dagLayout.nodes.push({
          ...n,
          parentId: expandedGoalId,
          extent: "parent" as const,
          position: {
            x: n.position.x - epicMinX + EPIC_PADDING,
            y: n.position.y - epicMinY + EPIC_HEADER_HEIGHT,
          },
        });
      }
      dagLayout.edges.push(...epicLayout.edges);
    }

    return dagLayout;
  }, [visibleGoals, expandedGoalId, expandedGraph, handleToggleGoal, t]);

  // Derived from the actual node list so a collapsed goal or failed refetch
  // clears the side panel without a state-reset effect.
  const selection = selectedNodeId
    ? (allEpicNodes.find((n) => n.id === selectedNodeId)?.id ?? null)
    : null;

  const neighbors = useMemo(() => {
    if (!selection) return new Set<string>();
    const ids = new Set<string>();
    for (const dep of allDependencies) {
      if (dep.prerequisite_id === selection) ids.add(dep.dependent_id);
      if (dep.dependent_id === selection) ids.add(dep.prerequisite_id);
    }
    return ids;
  }, [selection, allDependencies]);

  const rfNodes = useMemo(() => {
    const freshData = new Map(allEpicNodes.map((n) => [n.id, n]));
    return layout.nodes.map((n) => {
      if (n.type === "goalGroup" || n.type === "dagEndpoint") return n;
      const data = freshData.get(n.id) ?? (n.data as GraphNodesItem);
      return {
        ...n,
        ariaLabel: t("graph.node.label", {
          title: data.title,
          status: t(`common.status.${data.status}`),
        }),
        data: {
          ...data,
          typeIcon: "\u{1F3D7}\u{FE0F}",
          selected: n.id === selection,
          dimmed: selection !== null && n.id !== selection && !neighbors.has(n.id),
          expandable: false,
          expanded: false,
          nodeId: n.id,
        },
      };
    });
  }, [layout.nodes, allEpicNodes, selection, neighbors, t]);

  const rfEdges = layout.edges;

  const labeledEdges = useMemo(() => {
    const nodeMap = new Map<string, string>();
    for (const g of visibleGoals) nodeMap.set(g.id, g.title);
    for (const n of allEpicNodes) nodeMap.set(n.id, n.title);
    return rfEdges.map((e) => {
      if (e.id.startsWith("__start_"))
        return {
          ...e,
          ariaLabel: t("graph.edge.startTo", { title: nodeMap.get(e.target) ?? "node" }),
        };
      if (e.id.startsWith("__end_"))
        return {
          ...e,
          ariaLabel: t("graph.edge.toEnd", { title: nodeMap.get(e.source) ?? "node" }),
        };
      return {
        ...e,
        ariaLabel: t("graph.edge.link", {
          source: nodeMap.get(e.source) ?? "node",
          target: nodeMap.get(e.target) ?? "node",
        }),
      };
    });
  }, [rfEdges, visibleGoals, allEpicNodes, t]);

  /* v8 ignore start -- React Flow canvas callbacks; covered by E2E */
  // React Flow routes both pointer clicks and Enter/Space selection through
  // the same select changes; nodes are fully controlled, so selection is
  // tracked here rather than in React Flow's store.
  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      const picked = changes.find(
        (c): c is NodeSelectionChange => c.type === "select" && c.selected,
      );
      if (!picked) return;
      const nodeType = layout.nodes.find((n) => n.id === picked.id)?.type;
      if (nodeType === "goalGroup" || nodeType === "dagEndpoint") return;
      if (selectedNodeId === null) {
        openerRef.current = document.activeElement as HTMLElement | null;
      }
      setSelectedNodeId(picked.id);
      setTimeout(() => sidePanelRef.current?.focus(), 0);
    },
    [selectedNodeId, layout.nodes],
  );

  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      const picked = changes.find(
        (c): c is EdgeSelectionChange =>
          c.type === "select" && c.selected && !c.id.startsWith("__"),
      );
      if (!picked) return;
      const edge = rfEdges.find((e) => e.id === picked.id);
      if (!edge) return;
      const prereq = allEpicNodes.find((n) => n.id === edge.source);
      const dep = allEpicNodes.find((n) => n.id === edge.target);
      if (!prereq || !dep) return;
      setRemoveEdge({
        dependencyId: picked.id,
        prerequisiteTitle: prereq.title,
        dependentTitle: dep.title,
      });
    },
    [rfEdges, allEpicNodes],
  );

  const onNodeDoubleClick = useCallback(
    (_event: React.MouseEvent, node: { id: string; type?: string }) => {
      if (node.id === DAG_START_ID || node.id === DAG_END_ID) return;
      if (node.type === "goalGroup") {
        void navigate(`/projects/${projectId}/goals/${node.id}`);
      } else {
        void navigate(`/projects/${projectId}/epics/${node.id}`);
      }
    },
    [navigate, projectId],
  );

  const handleClosePanel = useCallback(() => {
    setSelectedNodeId(null);
    openerRef.current?.focus();
    openerRef.current = null;
  }, []);

  const onPaneClick = useCallback(() => handleClosePanel(), [handleClosePanel]);
  const handleFit = useCallback(() => void fitView({ duration: 200 }), [fitView]);
  const handleZoomIn = useCallback(() => void zoomIn({ duration: 200 }), [zoomIn]);
  const handleZoomOut = useCallback(() => void zoomOut({ duration: 200 }), [zoomOut]);

  const handleMutationSuccess = useCallback(() => {
    void refetch();
  }, [refetch]);
  /* v8 ignore stop */

  // Graph -> list, list -> graph, table -> list (matching the button label in each mode).
  const handleToggleView = useCallback(
    () => setViewMode((v) => (v === "list" ? "graph" : "list")),
    [],
  );

  const handleToggleTable = useCallback(
    () => setViewMode((v) => (v === "table" ? "graph" : "table")),
    [],
  );

  const handleToggleHideCompleted = useCallback(() => setHideCompleted((v) => !v), []);

  const handleAddDependency = useCallback(() => setAddDialogOpen(true), []);

  const selectedNode = selection ? (allEpicNodes.find((n) => n.id === selection) ?? null) : null;

  useEffect(() => {
    if (expandedGoalGraphQuery.isError) {
      setExpandedGoalId(null);
    }
  }, [expandedGoalGraphQuery.isError]);

  useEffect(() => {
    if (hideCompleted && expandedGoalId) {
      const goal = goals?.find((g) => g.id === expandedGoalId);
      if (goal?.completed) {
        setExpandedGoalId(null);
        setSelectedNodeId(null);
      }
    }
  }, [hideCompleted, expandedGoalId, goals]);

  useEffect(() => {
    if (expandedGoalId === null) return;
    const timer = setTimeout(() => fitView({ duration: 200 }), 50);
    return () => clearTimeout(timer);
  }, [expandedGoalId, fitView]);
  /* v8 ignore stop */

  if (goalsQuery.isPending && viewMode === "graph") {
    return <LoadingState />;
  }

  if (goalsQuery.isError && viewMode === "graph") {
    return (
      <>
        <ErrorState error={goalsQuery.error} onRetry={() => void goalsQuery.refetch()} />
        {listContent}
      </>
    );
  }

  const toolbarProps = {
    hasSelection: selection !== null,
    onFit: handleFit,
    onZoomIn: handleZoomIn,
    onZoomOut: handleZoomOut,
    onAddDependency: handleAddDependency,
    viewMode: viewMode as "graph" | "list" | "table",
    onToggleView: handleToggleView,
    onToggleTable: handleToggleTable,
    hideCompleted,
    onToggleHideCompleted: handleToggleHideCompleted,
  };

  return (
    <>
      {viewMode === "table" ? (
        <>
          <GraphToolbar {...toolbarProps} />
          <TabsView projectId={projectId} hideCompleted={hideCompleted} />
        </>
      ) : viewMode === "list" ? (
        <>
          <GraphToolbar {...toolbarProps} />
          {listContent}
        </>
      ) : !visibleGoals.length ? (
        <>
          <GraphToolbar {...toolbarProps} />
          <EmptyState title={t("graph.state.empty")} icon="🔗" />
        </>
      ) : (
        <div className={styles.canvas}>
          <GraphToolbar {...toolbarProps} floating />
          <div className={styles.flow} data-testid="graph-canvas">
            <ReactFlow
              nodes={rfNodes}
              edges={labeledEdges}
              nodeTypes={nodeTypes}
              aria-label={t("graph.canvas.label")}
              onNodesChange={onNodesChange}
              onNodeDoubleClick={onNodeDoubleClick}
              onPaneClick={onPaneClick}
              onEdgesChange={onEdgesChange}
              fitView
              nodesDraggable={false}
              nodesConnectable={false}
              zoomOnDoubleClick={false}
              minZoom={0.1}
              maxZoom={2}
              proOptions={{ hideAttribution: true }}
            />
          </div>
          {selectedNode && (
            <GraphSidePanel
              ref={sidePanelRef}
              node={selectedNode}
              level="epic"
              projectId={projectId}
              onClose={handleClosePanel}
            />
          )}
        </div>
      )}

      {expandedGoalId && (
        <GraphActions
          projectId={projectId}
          level="epic"
          nodes={expandedGraph?.nodes ?? []}
          selectedNodeId={selection}
          addDialogOpen={addDialogOpen}
          onAddDialogClose={() => setAddDialogOpen(false)}
          removeEdge={removeEdge}
          onRemoveDialogClose={() => setRemoveEdge(null)}
          onSuccess={handleMutationSuccess}
        />
      )}
    </>
  );
}
