import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { ReactFlow, ReactFlowProvider, useReactFlow, type NodeMouseHandler } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useQueries } from "@tanstack/react-query";
import { useListGoals, getGetGoalGraphQueryOptions } from "../../api/generated/goals/goals";
import {
  computeProjectLayout,
  classifyDagPosition,
  topologyKey,
  type GoalCluster,
} from "../../lib/graphLayout";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { GraphNode, type GraphNodeType } from "./GraphNode";
import { GoalGroupNode, type GoalGroupNodeType } from "./GoalGroupNode";
import { GraphSidePanel } from "./GraphSidePanel";
import { GraphToolbar } from "./GraphToolbar";
import { GraphActions } from "./GraphActions";
import styles from "./ProjectGraphView.module.css";
import type { ReactNode } from "react";

const nodeTypes = { graphNode: GraphNode, goalGroup: GoalGroupNode };

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

  const [viewMode, setViewMode] = useState<"graph" | "list">("graph");
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [removeEdge, setRemoveEdge] = useState<{
    dependencyId: string;
    dependentTitle: string;
    prerequisiteTitle: string;
  } | null>(null);

  const goalsQuery = useListGoals(projectId);
  const goals = goalsQuery.data?.items;

  const goalGraphQueries = useQueries({
    queries: (goals ?? []).map((goal) => getGetGoalGraphQueryOptions(projectId, goal.id)),
  });

  const allGraphsLoaded =
    goals !== undefined &&
    goalGraphQueries.length === goals.length &&
    goalGraphQueries.every((q) => q.isSuccess);
  const anyGraphLoading = goalGraphQueries.some((q) => q.isPending);
  const anyGraphError = goalGraphQueries.find((q) => q.isError);

  const allEpicNodes = goalGraphQueries.flatMap((q) => q.data?.nodes ?? []);
  const allDependencies = goalGraphQueries.flatMap((q) => q.data?.dependencies ?? []);

  const currentTopology =
    allGraphsLoaded && goals
      ? goals
          .map((g, i) => {
            const graph = goalGraphQueries[i].data!;
            return `${g.id}:${topologyKey(graph.nodes, graph.dependencies)}`;
          })
          .join("|")
      : "";

  const layout = useMemo(() => {
    if (!currentTopology || !goals) return { nodes: [], edges: [] };

    const clusters: GoalCluster[] = goals.map((goal, i) => ({
      goalId: goal.id,
      goalTitle: goal.title,
      goalCompleted: goal.completed,
      epicCounts: goal.epic_counts,
      graph: goalGraphQueries[i].data!,
    }));

    const result = computeProjectLayout(clusters);

    return {
      nodes: result.nodes.map((node) => {
        if (node.type !== "graphNode") return node;
        const cluster = clusters.find((c) => c.graph.nodes.some((n) => n.id === node.id));
        if (!cluster) return node;
        const pos = classifyDagPosition(node.id, cluster.graph.dependencies);
        return { ...node, data: { ...node.data, ...pos } };
      }),
      edges: result.edges,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- relayout only on topology change
  }, [currentTopology]);

  const neighbors = useMemo(() => {
    if (!selectedNodeId) return new Set<string>();
    /* v8 ignore start -- selection requires React Flow canvas click; covered by E2E */
    const ids = new Set<string>();
    for (const dep of allDependencies) {
      if (dep.prerequisite_id === selectedNodeId) ids.add(dep.dependent_id);
      if (dep.dependent_id === selectedNodeId) ids.add(dep.prerequisite_id);
    }
    return ids;
    /* v8 ignore stop */
  }, [selectedNodeId, allDependencies]);

  const rfNodes: (GraphNodeType | GoalGroupNodeType)[] = useMemo(() => {
    const freshEpicData = new Map(allEpicNodes.map((n) => [n.id, n]));
    const freshGoalData = goals ? new Map(goals.map((g) => [g.id, g])) : new Map();

    return layout.nodes.map((n) => {
      if (n.type === "goalGroup") {
        const goal = freshGoalData.get(n.id);
        if (goal) {
          return {
            ...n,
            data: { title: goal.title, completed: goal.completed, epicCounts: goal.epic_counts },
          } as GoalGroupNodeType;
        }
        return n as GoalGroupNodeType;
      }

      const epicData = freshEpicData.get(n.id) ?? n.data;
      return {
        ...n,
        ariaLabel: t("graph.node.label", {
          title: epicData.title,
          status: t(`common.status.${epicData.status}`),
        }),
        data: {
          ...epicData,
          selected: n.id === selectedNodeId,
          dimmed: selectedNodeId !== null && n.id !== selectedNodeId && !neighbors.has(n.id),
          root: n.data.root ?? false,
          leaf: n.data.leaf ?? false,
        },
      } as GraphNodeType;
    });
  }, [layout.nodes, allEpicNodes, goals, selectedNodeId, neighbors, t]);

  const rfEdges = layout.edges;

  /* v8 ignore start -- React Flow canvas callbacks; covered by E2E */
  const onNodeClick: NodeMouseHandler = useCallback((_event, node) => {
    if (node.type === "goalGroup") return;
    openerRef.current = document.activeElement as HTMLElement | null;
    setSelectedNodeId(node.id);
    setTimeout(() => sidePanelRef.current?.focus(), 0);
  }, []);

  const onNodeDoubleClick: NodeMouseHandler = useCallback(
    (_event, node) => {
      if (node.type === "goalGroup") return;
      void navigate(`/projects/${projectId}/epics/${node.id}`);
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
  /* v8 ignore stop */

  const handleToggleView = useCallback(
    () => setViewMode((v) => (v === "graph" ? "list" : "graph")),
    [],
  );

  const handleAddDependency = useCallback(() => setAddDialogOpen(true), []);

  /* v8 ignore start -- fires from React Flow canvas; covered by E2E */
  const onEdgeClick = useCallback(
    (_event: React.MouseEvent, edge: { id: string; source: string; target: string }) => {
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
  /* v8 ignore stop */

  /* v8 ignore start -- mutation callback; covered by E2E */
  const handleMutationSuccess = useCallback(() => {
    for (const q of goalGraphQueries) {
      void q.refetch();
    }
  }, [goalGraphQueries]);
  /* v8 ignore stop */

  const selectedNode = selectedNodeId
    ? (allEpicNodes.find((n) => n.id === selectedNodeId) ?? null)
    : null;

  /* v8 ignore start -- stale selection guard; covered by E2E */
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

  if (anyGraphError && viewMode === "graph") {
    return (
      <ErrorState
        error={anyGraphError.error!}
        onRetry={() => {
          for (const q of goalGraphQueries) void q.refetch();
        }}
      />
    );
  }

  return (
    <>
      <GraphToolbar
        viewMode={viewMode}
        hasSelection={selectedNodeId !== null}
        onFit={handleFit}
        onZoomIn={handleZoomIn}
        onZoomOut={handleZoomOut}
        onResetLayout={handleResetLayout}
        onToggleView={handleToggleView}
        onAddDependency={handleAddDependency}
      />

      {viewMode === "list" ? (
        listContent
      ) : !goals || goals.length === 0 ? (
        <EmptyState title={t("graph.state.empty")} icon="🔗" />
      ) : anyGraphLoading && !allGraphsLoaded ? (
        <LoadingState />
      ) : allEpicNodes.length === 0 ? (
        <EmptyState title={t("graph.state.empty")} icon="🔗" />
      ) : (
        <div className={styles.canvas}>
          <div className={styles.flow} role="img" aria-label={t("graph.canvas.label")}>
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
          <GraphSidePanel
            ref={sidePanelRef}
            node={selectedNode}
            level="epic"
            projectId={projectId}
            onClose={handleClosePanel}
          />
        </div>
      )}

      <GraphActions
        projectId={projectId}
        level="epic"
        nodes={allEpicNodes}
        selectedNodeId={selectedNodeId}
        addDialogOpen={addDialogOpen}
        /* v8 ignore start -- dialog callbacks; covered by E2E */
        onAddDialogClose={() => setAddDialogOpen(false)}
        removeEdge={removeEdge}
        onRemoveDialogClose={() => setRemoveEdge(null)}
        /* v8 ignore stop */
        onSuccess={handleMutationSuccess}
      />
    </>
  );
}
