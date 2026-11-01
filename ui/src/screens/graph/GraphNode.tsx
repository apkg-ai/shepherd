import { Handle, Position } from "@xyflow/react";
import type { NodeProps, Node } from "@xyflow/react";
import { useTranslation } from "react-i18next";
import type { Counts } from "../../api/generated/model/counts";
import type { Eligibility } from "../../api/generated/model/eligibility";
import type { EntityStatus } from "../../api/generated/model/entityStatus";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import styles from "./GraphNode.module.css";

export type GraphNodeData = {
  title: string;
  status: EntityStatus;
  eligibility: Eligibility;
  counts: Counts;
  selected: boolean;
  dimmed: boolean;
  root: boolean;
  leaf: boolean;
  expandable: boolean;
  expanded: boolean;
  onToggleExpand?: (nodeId: string) => void;
  nodeId: string;
  [key: string]: unknown;
};

export type GraphNodeType = Node<GraphNodeData, "graphNode">;

export function GraphNode({ id, data }: NodeProps<GraphNodeType>) {
  const { t } = useTranslation();

  const handleExpandClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    data.onToggleExpand?.(data.nodeId);
  };

  return (
    <div
      className={styles.node}
      data-testid={`graph-node-${id}`}
      data-status={data.status}
      data-selected={data.selected || false}
      data-dimmed={data.dimmed || false}
      data-expanded={data.expanded || false}
    >
      <Handle type="target" position={Position.Top} className={styles.handle} />
      <div className={styles.content}>
        <div className={styles.title}>{data.title}</div>
        <div className={styles.tags}>
          <StatusBadge status={data.status} />
          {data.counts.total > 0 && <ProgressCounts counts={data.counts} />}
        </div>
      </div>
      {data.expandable && (
        <button
          type="button"
          className={styles.expandToggle}
          onClick={handleExpandClick}
          aria-label={
            data.expanded ? t("graph.action.collapseTasks") : t("graph.action.expandTasks")
          }
        >
          {data.expanded ? "▲" : "▼"}
        </button>
      )}
      <Handle type="source" position={Position.Bottom} className={styles.handle} />
    </div>
  );
}
