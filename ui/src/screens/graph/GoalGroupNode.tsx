import { Handle, Position } from "@xyflow/react";
import type { NodeProps, Node } from "@xyflow/react";
import type { Counts } from "../../api/generated/model/counts";
import { NodeContent } from "./NodeContent";
import styles from "./GoalGroupNode.module.css";

interface GoalGroupData extends Record<string, unknown> {
  title: string;
  completed: boolean;
  epicCounts: Counts;
  expanded: boolean;
  onToggleExpand?: (goalId: string) => void;
  goalId: string;
}

export type GoalGroupNodeType = Node<GoalGroupData, "goalGroup">;

/* v8 ignore start -- rendered inside React Flow; covered by E2E */
export function GoalGroupNode({ id, data }: NodeProps<GoalGroupNodeType>) {
  const handleExpandClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    data.onToggleExpand?.(data.goalId);
  };

  return (
    <div className={styles.wrapper} data-testid={`goal-group-${id}`} data-expanded={data.expanded}>
      <Handle type="target" position={Position.Top} className={styles.handle} />
      <NodeContent
        typeIcon={"\u{1F3AF}"}
        title={data.title}
        status={data.completed ? "done" : "open"}
        counts={data.epicCounts}
        expandable={true}
        expanded={data.expanded}
        selected={false}
        dimmed={false}
        onExpandClick={handleExpandClick}
      />
      <Handle type="source" position={Position.Bottom} className={styles.handle} />
    </div>
  );
}
/* v8 ignore stop */
