import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import {
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type NodeMouseHandler,
  type OnMoveEnd,
  type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { UseQueryResult } from "@tanstack/react-query";
import type { Graph } from "../../api/generated/model/graph";
import type { Problem } from "../../api/generated/model/problem";
import { isShepherdError, errorSlug } from "../../api/problem";
import { computeLayout, topologyKey } from "../../lib/graphLayout";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { GraphNode, type GraphNodeType } from "./GraphNode";
import { GraphSidePanel } from "./GraphSidePanel";
import { GraphToolbar } from "./GraphToolbar";
import { GraphActions } from "./GraphActions";
import styles from "./GraphScreen.module.css";
import type { ReactNode } from "react";

const nodeTypes = { graphNode: GraphNode };

const MAX_VIEWPORT_CACHE = 50;
const viewportCache = new Map<string, Viewport>();

interface GraphScreenProps {
  projectId: string;
  level: "epic" | "task";
  scopeId: string;
  graphQuery: UseQueryResult<Graph, Problem>;
  detailPath: (nodeId: string) => string;
  listContent: ReactNode;
}

export function GraphScreen(props: GraphScreenProps) {
  return (
    <ReactFlowProvider>
      <GraphScreenInner {...props} />
    </ReactFlowProvider>
  );
}

function GraphScreenInner({
  projectId,
  level,
  scopeId,
  graphQuery,
  detailPath,
  listContent,
}: GraphScreenProps) {
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

  const isTooLarge =
    graphQuery.isError &&
    isShepherdError(graphQuery.error) &&
    errorSlug(graphQuery.error) === "graph_too_large";

  const cacheKey = `${level}:${scopeId}`;

  const graphData = graphQuery.data;
  const nodes = useMemo(() => graphData?.nodes ?? [], [graphData]);
  const dependencies = useMemo(() => graphData?.dependencies ?? [], [graphData]);

  const currentTopology = useMemo(() => topologyKey(nodes, dependencies), [nodes, dependencies]);

  const layout = useMemo(
    () => computeLayout(nodes, dependencies),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- relayout only on topology change, not status updates
    [currentTopology],
  );

  const neighbors = useMemo(() => {
    if (!selectedNodeId) return new Set<string>();
    /* v8 ignore start -- selection requires React Flow canvas click; covered by E2E */
    const ids = new Set<string>();
    for (const dep of dependencies) {
      if (dep.prerequisite_id === selectedNodeId) ids.add(dep.dependent_id);
      if (dep.dependent_id === selectedNodeId) ids.add(dep.prerequisite_id);
    }
    return ids;
    /* v8 ignore stop */
  }, [selectedNodeId, dependencies]);

  const rfNodes: GraphNodeType[] = useMemo(() => {
    const freshData = new Map(nodes.map((n) => [n.id, n]));
    return layout.nodes.map((n) => {
      const data = freshData.get(n.id) ?? n.data;
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
        },
      };
    }) as GraphNodeType[];
  }, [layout.nodes, nodes, selectedNodeId, neighbors, t]);

  const rfEdges = layout.edges;

  const cachedViewport = viewportCache.get(cacheKey);
  /* v8 ignore start -- LRU promotion; covered by E2E */
  if (cachedViewport) {
    viewportCache.delete(cacheKey);
    viewportCache.set(cacheKey, cachedViewport);
  }
  /* v8 ignore stop */

  /* v8 ignore start -- React Flow canvas callbacks; covered by E2E */
  const onNodeClick: NodeMouseHandler<GraphNodeType> = useCallback((_event, node) => {
    openerRef.current = document.activeElement as HTMLElement | null;
    setSelectedNodeId(node.id);
    setTimeout(() => sidePanelRef.current?.focus(), 0);
  }, []);

  const onNodeDoubleClick: NodeMouseHandler<GraphNodeType> = useCallback(
    (_event, node) => void navigate(detailPath(node.id)),
    [navigate, detailPath],
  );

  const handleClosePanel = useCallback(() => {
    setSelectedNodeId(null);
    openerRef.current?.focus();
    openerRef.current = null;
  }, []);

  const onPaneClick = useCallback(() => handleClosePanel(), [handleClosePanel]);

  const onMoveEnd: OnMoveEnd = useCallback(
    (_event, viewport) => {
      viewportCache.set(cacheKey, viewport);
      if (viewportCache.size > MAX_VIEWPORT_CACHE) {
        const oldest = viewportCache.keys().next().value!;
        viewportCache.delete(oldest);
      }
    },
    [cacheKey],
  );

  const handleFit = useCallback(() => void fitView({ duration: 200 }), [fitView]);
  const handleZoomIn = useCallback(() => void zoomIn({ duration: 200 }), [zoomIn]);
  const handleZoomOut = useCallback(() => void zoomOut({ duration: 200 }), [zoomOut]);

  const handleResetLayout = useCallback(() => {
    viewportCache.delete(cacheKey);
    void fitView({ duration: 200 });
  }, [cacheKey, fitView]);
  /* v8 ignore stop */

  const handleToggleView = useCallback(
    () => setViewMode((v) => (v === "graph" ? "list" : "graph")),
    [],
  );

  const handleAddDependency = useCallback(() => setAddDialogOpen(true), []);

  /* v8 ignore start -- fires from React Flow canvas; covered by E2E */
  const onEdgeClick = useCallback(
    (_event: React.MouseEvent, edge: { id: string; source: string; target: string }) => {
      const prereq = nodes.find((n) => n.id === edge.source);
      const dep = nodes.find((n) => n.id === edge.target);
      if (!prereq || !dep) return;
      setRemoveEdge({
        dependencyId: edge.id,
        prerequisiteTitle: prereq.title,
        dependentTitle: dep.title,
      });
    },
    [nodes],
  );
  /* v8 ignore stop */

  /* v8 ignore start -- mutation callback; covered by E2E */
  const handleMutationSuccess = useCallback(() => {
    void graphQuery.refetch();
  }, [graphQuery.refetch]);
  /* v8 ignore stop */

  const selectedNode = selectedNodeId ? (nodes.find((n) => n.id === selectedNodeId) ?? null) : null;

  /* v8 ignore start -- stale selection guard; covered by E2E */
  useEffect(() => {
    if (selectedNodeId && nodes.length > 0 && !nodes.some((n) => n.id === selectedNodeId)) {
      setSelectedNodeId(null);
    }
  }, [selectedNodeId, nodes]);
  /* v8 ignore stop */

  if (graphQuery.isPending && viewMode === "graph") {
    /* v8 ignore next -- loading flash; covered by E2E */
    return <LoadingState />;
  }

  if (isTooLarge) {
    return (
      <>
        <div role="alert" className={styles.tooLarge}>
          {t("graph.state.tooLarge")}
        </div>
        {listContent}
      </>
    );
  }

  if (graphQuery.isError && viewMode === "graph") {
    return <ErrorState error={graphQuery.error} onRetry={() => void graphQuery.refetch()} />;
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
      ) : nodes.length === 0 ? (
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
              onMoveEnd={onMoveEnd}
              defaultViewport={cachedViewport}
              fitView={!cachedViewport}
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
            level={level}
            projectId={projectId}
            onClose={handleClosePanel}
          />
        </div>
      )}

      <GraphActions
        projectId={projectId}
        level={level}
        nodes={nodes}
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
