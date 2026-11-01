import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { ReactFlow, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useQueries } from "@tanstack/react-query";
import type { Node, Edge } from "@xyflow/react";
import { MarkerType } from "@xyflow/react";
import { useListGoals, getGetGoalGraphQueryOptions } from "../../api/generated/goals/goals";
import { getGetEpicGraphQueryOptions } from "../../api/generated/epics/epics";
import type { GraphNodesItem } from "../../api/generated/model/graphNodesItem";
import {
  computeLayout,
  addDagEndpoints,
  computeLayoutWithEndpoints,
  topologyKey,
  DAG_START_ID,
  DAG_END_ID,
} from "../../lib/graphLayout";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { GraphNode } from "./GraphNode";
import { DagEndpointNode } from "./DagEndpointNode";
import { GraphSidePanel } from "./GraphSidePanel";
import { GraphToolbar } from "./GraphToolbar";
import { GraphActions } from "./GraphActions";
import styles from "./ProjectGraphView.module.css";
import type { ReactNode } from "react";

const nodeTypes = { graphNode: GraphNode, dagEndpoint: DagEndpointNode };

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
  const [expandedEpics, setExpandedEpics] = useState<Set<string>>(new Set());
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [removeEdge, setRemoveEdge] = useState<{
    dependencyId: string;
    dependentTitle: string;
    prerequisiteTitle: string;
  } | null>(null);

  const goalsQuery = useListGoals(projectId);
  const goals = goalsQuery.data?.items;

  // Fetch goal graphs for all goals
  const goalGraphQueries = useQueries({
    queries: (goals ?? []).map((goal) => getGetGoalGraphQueryOptions(projectId, goal.id)),
  });

  // Fetch epic graphs for expanded epics
  const expandedEpicIds = useMemo(() => [...expandedEpics], [expandedEpics]);
  const epicGraphQueries = useQueries({
    queries: expandedEpicIds.map((epicId) => getGetEpicGraphQueryOptions(projectId, epicId)),
  });

  const allGoalGraphsLoaded =
    goals !== undefined &&
    goalGraphQueries.length === goals.length &&
    goalGraphQueries.every((q) => q.isSuccess);
  const anyGraphLoading = goalGraphQueries.some((q) => q.isPending);
  const anyGraphError = goalGraphQueries.find((q) => q.isError);

  // Merge all epic nodes and dependencies from all goal graphs
  const allEpicNodes = goalGraphQueries.flatMap((q) => q.data?.nodes ?? []);
  const allEpicDeps = goalGraphQueries.flatMap((q) => q.data?.dependencies ?? []);

  // Merge task nodes and dependencies from expanded epics
  const expandedTaskNodes = epicGraphQueries.flatMap((q) => q.data?.nodes ?? []);
  const expandedTaskDeps = epicGraphQueries.flatMap((q) => q.data?.dependencies ?? []);

  // All nodes for the side panel lookup
  const allNodes: GraphNodesItem[] = [...allEpicNodes, ...expandedTaskNodes];

  const handleToggleExpand = useCallback((epicId: string) => {
    setExpandedEpics((prev) => {
      const next = new Set(prev);
      if (next.has(epicId)) next.delete(epicId);
      else next.add(epicId);
      return next;
    });
  }, []);

  // Build combined topology key including expanded task data
  const currentTopology = useMemo(() => {
    if (!allGoalGraphsLoaded || !goals) return "";
    const epicKey = topologyKey(allEpicNodes, allEpicDeps);
    const taskKey = expandedEpicIds
      .map((id, i) => {
        const data = epicGraphQueries[i]?.data;
        return data ? `${id}:${topologyKey(data.nodes, data.dependencies)}` : "";
      })
      .join("|");
    return `${epicKey}||${taskKey}`;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allGoalGraphsLoaded, allEpicNodes, allEpicDeps, expandedEpicIds, epicGraphQueries]);

  const layout = useMemo(() => {
    if (!currentTopology) return { nodes: [] as Node[], edges: [] as Edge[] };

    // Start with epic layout
    const epicLayout = computeLayout(allEpicNodes, allEpicDeps);

    // Add task nodes and edges for expanded epics
    const taskNodes: Node[] = [];
    const taskEdges: Edge[] = [];
    const parentChildEdges: Edge[] = [];

    for (let i = 0; i < expandedEpicIds.length; i++) {
      const epicId = expandedEpicIds[i];
      const epicGraph = epicGraphQueries[i]?.data;
      if (!epicGraph) continue;

      // Add task nodes
      for (const task of epicGraph.nodes) {
        taskNodes.push({
          id: task.id,
          type: "graphNode",
          position: { x: 0, y: 0 },
          data: task,
        });
      }

      // Add task dependency edges
      for (const dep of epicGraph.dependencies) {
        taskEdges.push({
          id: dep.id,
          source: dep.prerequisite_id,
          target: dep.dependent_id,
          markerEnd: { type: MarkerType.ArrowClosed },
          data: dep as unknown as Record<string, unknown>,
        });
      }

      // Parent-child edges: epic → each task (dashed style)
      for (const task of epicGraph.nodes) {
        parentChildEdges.push({
          id: `__parent_${epicId}_${task.id}`,
          source: epicId,
          target: task.id,
          style: { strokeDasharray: "5 3", opacity: 0.5 },
          markerEnd: { type: MarkerType.ArrowClosed },
        });
      }
    }

    const combinedNodes = [...epicLayout.nodes, ...taskNodes];
    const combinedEdges = [...epicLayout.edges, ...taskEdges, ...parentChildEdges];

    // Add start/end endpoints and compute full layout
    const withEndpoints = addDagEndpoints(combinedNodes, combinedEdges);
    return computeLayoutWithEndpoints(withEndpoints.nodes, withEndpoints.edges);

    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentTopology]);

  const neighbors = useMemo(() => {
    if (!selectedNodeId) return new Set<string>();
    const allDeps = [...allEpicDeps, ...expandedTaskDeps];
    const ids = new Set<string>();
    for (const dep of allDeps) {
      if (dep.prerequisite_id === selectedNodeId) ids.add(dep.dependent_id);
      if (dep.dependent_id === selectedNodeId) ids.add(dep.prerequisite_id);
    }
    return ids;
  }, [selectedNodeId, allEpicDeps, expandedTaskDeps]);

  const rfNodes = useMemo(() => {
    const freshData = new Map(allNodes.map((n) => [n.id, n]));
    return layout.nodes.map((n) => {
      if (n.type === "dagEndpoint") return n;
      const data = freshData.get(n.id) ?? (n.data as GraphNodesItem);
      const isEpic = allEpicNodes.some((e) => e.id === n.id);
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
          expandable: isEpic && data.counts.total > 0,
          expanded: expandedEpics.has(n.id),
          onToggleExpand: handleToggleExpand,
          nodeId: n.id,
        },
      };
    });
  }, [
    layout.nodes,
    allNodes,
    allEpicNodes,
    selectedNodeId,
    neighbors,
    expandedEpics,
    handleToggleExpand,
    t,
  ]);

  const rfEdges = layout.edges;

  /* v8 ignore start -- React Flow canvas callbacks; covered by E2E */
  const onNodeClick = useCallback((_event: React.MouseEvent, node: { id: string }) => {
    if (node.id === DAG_START_ID || node.id === DAG_END_ID) return;
    openerRef.current = document.activeElement as HTMLElement | null;
    setSelectedNodeId(node.id);
    setTimeout(() => sidePanelRef.current?.focus(), 0);
  }, []);

  const onNodeDoubleClick = useCallback(
    (_event: React.MouseEvent, node: { id: string }) => {
      if (node.id === DAG_START_ID || node.id === DAG_END_ID) return;
      const isEpic = allEpicNodes.some((e) => e.id === node.id);
      if (isEpic) {
        void navigate(`/projects/${projectId}/epics/${node.id}`);
      } else {
        void navigate(`/projects/${projectId}/tasks/${node.id}`);
      }
    },
    [navigate, projectId, allEpicNodes],
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
      const prereq = allNodes.find((n) => n.id === edge.source);
      const dep = allNodes.find((n) => n.id === edge.target);
      if (!prereq || !dep) return;
      setRemoveEdge({
        dependencyId: edge.id,
        prerequisiteTitle: prereq.title,
        dependentTitle: dep.title,
      });
    },
    [allNodes],
  );

  const handleMutationSuccess = useCallback(() => {
    for (const q of goalGraphQueries) void q.refetch();
    for (const q of epicGraphQueries) void q.refetch();
  }, [goalGraphQueries, epicGraphQueries]);
  /* v8 ignore stop */

  const handleToggleView = useCallback(
    () => setViewMode((v) => (v === "graph" ? "list" : "graph")),
    [],
  );

  const handleAddDependency = useCallback(() => setAddDialogOpen(true), []);

  const selectedNode = selectedNodeId
    ? (allNodes.find((n) => n.id === selectedNodeId) ?? null)
    : null;

  const selectedLevel = selectedNode
    ? allEpicNodes.some((e) => e.id === selectedNodeId)
      ? ("epic" as const)
      : ("task" as const)
    : ("epic" as const);

  useEffect(() => {
    if (selectedNodeId && allNodes.length > 0 && !allNodes.some((n) => n.id === selectedNodeId)) {
      setSelectedNodeId(null);
    }
  }, [selectedNodeId, allNodes]);

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
      ) : anyGraphLoading && !allGoalGraphsLoaded ? (
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
            level={selectedLevel}
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
        onAddDialogClose={() => setAddDialogOpen(false)}
        removeEdge={removeEdge}
        onRemoveDialogClose={() => setRemoveEdge(null)}
        onSuccess={handleMutationSuccess}
      />
    </>
  );
}
