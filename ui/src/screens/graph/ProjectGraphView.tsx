import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { ReactFlow, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { Node, Edge } from "@xyflow/react";
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
const EPIC_HEADER_HEIGHT = 52;

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

    // Build goal nodes (no inter-goal dependencies, so edges are empty)
    const goalNodes: Node[] = visibleGoals.map((goal) => ({
      id: goal.id,
      type: "goalGroup",
      position: { x: 0, y: 0 },
      data: {
        title: goal.title,
        completed: goal.completed,
        epicCounts: goal.epic_counts,
        expanded: goal.id === expandedGoalId,
        onToggleExpand: handleToggleGoal,
        goalId: goal.id,
      },
      style: { width: NODE_WIDTH, height: NODE_HEIGHT },
    }));

    // Add start/end nodes and compute DAG layout (start → all goals → end)
    const withEndpoints = addDagEndpoints(goalNodes, []);
    const dagLayout = computeLayoutWithEndpoints(withEndpoints.nodes, withEndpoints.edges);

    // If a goal is expanded, inject its epic sub-DAG inside the container
    if (expandedGoalId && expandedGraph) {
      const epicLayout = computeLayout(expandedGraph.nodes, expandedGraph.dependencies);

      // Compute bounding box of the epic sub-DAG
      let minX = Infinity;
      let minY = Infinity;
      let maxX = 0;
      let maxY = 0;
      for (const n of epicLayout.nodes) {
        minX = Math.min(minX, n.position.x);
        minY = Math.min(minY, n.position.y);
        maxX = Math.max(maxX, n.position.x + NODE_WIDTH);
        maxY = Math.max(maxY, n.position.y + NODE_HEIGHT);
      }
      if (!Number.isFinite(minX)) {
        minX = 0;
        minY = 0;
      }

      const containerWidth = Math.max(NODE_WIDTH, maxX - minX + EPIC_PADDING * 2);
      const containerHeight = EPIC_HEADER_HEIGHT + (maxY - minY) + EPIC_PADDING;

      // Update the expanded goal node's dimensions
      const expandedIdx = dagLayout.nodes.findIndex((n) => n.id === expandedGoalId);
      if (expandedIdx >= 0) {
        dagLayout.nodes[expandedIdx] = {
          ...dagLayout.nodes[expandedIdx],
          style: { width: containerWidth, height: containerHeight },
        };
      }

      // Add epic children inside the container
      for (const n of epicLayout.nodes) {
        dagLayout.nodes.push({
          ...n,
          parentId: expandedGoalId,
          extent: "parent" as const,
          position: {
            x: n.position.x - minX + EPIC_PADDING,
            y: n.position.y - minY + EPIC_HEADER_HEIGHT,
          },
        });
      }

      dagLayout.edges.push(...epicLayout.edges);
    }

    return dagLayout;
  }, [visibleGoals, expandedGoalId, expandedGraph, handleToggleGoal]);

  // All epic nodes for the side panel and actions
  const allEpicNodes: GraphNodesItem[] = expandedGraph?.nodes ?? [];
  const allDependencies = expandedGraph?.dependencies ?? [];

  const neighbors = useMemo(() => {
    if (!selectedNodeId) return new Set<string>();
    const ids = new Set<string>();
    for (const dep of allDependencies) {
      if (dep.prerequisite_id === selectedNodeId) ids.add(dep.dependent_id);
      if (dep.dependent_id === selectedNodeId) ids.add(dep.prerequisite_id);
    }
    return ids;
  }, [selectedNodeId, allDependencies]);

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
          selected: n.id === selectedNodeId,
          dimmed: selectedNodeId !== null && n.id !== selectedNodeId && !neighbors.has(n.id),
          root: false,
          leaf: false,
          expandable: false,
          expanded: false,
          nodeId: n.id,
        },
      };
    });
  }, [layout.nodes, allEpicNodes, selectedNodeId, neighbors, t]);

  const rfEdges = layout.edges;

  /* v8 ignore start -- React Flow canvas callbacks; covered by E2E */
  const onNodeClick = useCallback(
    (_event: React.MouseEvent, node: { id: string; type?: string }) => {
      if (node.type === "goalGroup" || node.id === DAG_START_ID || node.id === DAG_END_ID) return;
      openerRef.current = document.activeElement as HTMLElement | null;
      setSelectedNodeId(node.id);
      setTimeout(() => sidePanelRef.current?.focus(), 0);
    },
    [],
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
  const handleResetLayout = useCallback(() => void fitView({ duration: 200 }), [fitView]);

  const onEdgeClick = useCallback(
    (_event: React.MouseEvent, edge: { id: string; source: string; target: string }) => {
      if (edge.id.startsWith("__")) return;
      const prereq = allEpicNodes.find((n) => n.id === edge.source);
      const dep = allEpicNodes.find((n) => n.id === edge.target);
      if (!prereq || !dep) return;
      setRemoveEdge({
        dependencyId: edge.id,
        prerequisiteTitle: prereq.title,
        dependentTitle: dep.title,
      });
    },
    [allEpicNodes],
  );

  const handleMutationSuccess = useCallback(() => {
    void expandedGoalGraphQuery.refetch();
  }, [expandedGoalGraphQuery.refetch]);
  /* v8 ignore stop */

  const handleToggleView = useCallback(
    () => setViewMode((v) => (v === "graph" ? "list" : "graph")),
    [],
  );

  const handleToggleTable = useCallback(
    () => setViewMode((v) => (v === "graph" ? "table" : "graph")),
    [],
  );

  const handleToggleHideCompleted = useCallback(() => setHideCompleted((v) => !v), []);

  const handleAddDependency = useCallback(() => setAddDialogOpen(true), []);

  const selectedNode = selectedNodeId
    ? (allEpicNodes.find((n) => n.id === selectedNodeId) ?? null)
    : null;

  useEffect(() => {
    if (
      selectedNodeId &&
      allEpicNodes.length > 0 &&
      !allEpicNodes.some((n) => n.id === selectedNodeId)
    ) {
      setSelectedNodeId(null);
    }
  }, [selectedNodeId, allEpicNodes]);
  /* v8 ignore stop */

  if (goalsQuery.isPending && viewMode === "graph") {
    return <LoadingState />;
  }

  if (goalsQuery.isError && viewMode === "graph") {
    return <ErrorState error={goalsQuery.error} onRetry={() => void goalsQuery.refetch()} />;
  }

  return (
    <>
      <GraphToolbar
        viewMode={viewMode}
        hasSelection={selectedNodeId !== null}
        hideCompleted={hideCompleted}
        onFit={handleFit}
        onZoomIn={handleZoomIn}
        onZoomOut={handleZoomOut}
        onResetLayout={handleResetLayout}
        onToggleView={handleToggleView}
        onToggleTable={handleToggleTable}
        onAddDependency={handleAddDependency}
        onToggleHideCompleted={handleToggleHideCompleted}
      />

      {viewMode === "table" ? (
        <TabsView projectId={projectId} hideCompleted={hideCompleted} />
      ) : viewMode === "list" ? (
        listContent
      ) : !visibleGoals.length ? (
        <EmptyState title={t("graph.state.empty")} icon="🔗" />
      ) : (
        <div className={styles.canvas}>
          <div className={styles.flow} data-testid="graph-canvas">
            <ReactFlow
              nodes={rfNodes}
              edges={rfEdges}
              nodeTypes={nodeTypes}
              onNodeClick={onNodeClick}
              onNodeDoubleClick={onNodeDoubleClick}
              onPaneClick={onPaneClick}
              onEdgeClick={onEdgeClick}
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
          nodes={allEpicNodes}
          selectedNodeId={selectedNodeId}
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
