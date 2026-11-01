import { Handle, Position } from "@xyflow/react";
import type { NodeProps, Node } from "@xyflow/react";
import type { Counts } from "../../api/generated/model/counts";
import type { Eligibility } from "../../api/generated/model/eligibility";
import type { EntityStatus } from "../../api/generated/model/entityStatus";
import { NodeContent } from "./NodeContent";
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
  /* v8 ignore next 4 -- expand click only fires in React Flow canvas; covered by E2E */
  const handleExpandClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    data.onToggleExpand?.(data.nodeId);
  };

  return (
    <div className={styles.wrapper} data-testid={`graph-node-${id}`}>
      <Handle type="target" position={Position.Top} className={styles.handle} />
      <NodeContent
        title={data.title}
        status={data.status}
        counts={data.counts}
        expandable={data.expandable}
        expanded={data.expanded}
        selected={data.selected}
        dimmed={data.dimmed}
        onExpandClick={handleExpandClick}
      />
      <Handle type="source" position={Position.Bottom} className={styles.handle} />
    </div>
  );
}
