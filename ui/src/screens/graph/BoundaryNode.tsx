import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BOUNDARY_SIZE, type BoundaryNodeType } from "../../lib/graphLayout";
import styles from "./BoundaryNode.module.css";

/**
 * Virtual Start/End node anchoring the epic dependency flow.
 * A simple circle with a label — not a real task, just a visual boundary
 * that makes the graph read as a complete traversal from entry to exit.
 */
export function BoundaryNode({ data }: NodeProps<BoundaryNodeType>) {
  return (
    <div className={styles.boundary} style={{ width: BOUNDARY_SIZE, height: BOUNDARY_SIZE }}>
      <Handle type="target" position={Position.Top} isConnectable={false} />
      <span className={styles.label}>{data.label}</span>
      <Handle type="source" position={Position.Bottom} isConnectable={false} />
    </div>
  );
}
