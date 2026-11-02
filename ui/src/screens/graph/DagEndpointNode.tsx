import { Handle, Position } from "@xyflow/react";
import type { NodeProps, Node } from "@xyflow/react";
import { useTranslation } from "react-i18next";
import styles from "./DagEndpointNode.module.css";

interface DagEndpointData extends Record<string, unknown> {
  label: string;
}

export type DagEndpointNodeType = Node<DagEndpointData, "dagEndpoint">;

export function DagEndpointNode({ data }: NodeProps<DagEndpointNodeType>) {
  const { t } = useTranslation();
  const isStart = data.label === "start";

  return (
    <div className={styles.endpoint} data-kind={data.label}>
      {!isStart && <Handle type="target" position={Position.Top} className={styles.handle} />}
      <span className={styles.label}>{t(`graph.endpoint.${data.label}`)}</span>
      {isStart && <Handle type="source" position={Position.Bottom} className={styles.handle} />}
    </div>
  );
}
