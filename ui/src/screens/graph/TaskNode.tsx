import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Link } from "react-router";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { StatusBadge } from "../../components/StatusBadge";
import type { TaskNodeType } from "../../lib/graphLayout";
import styles from "./TaskNode.module.css";

/**
 * A task on the graph canvas, speaking the status language from
 * docs/04-ui.md via the same --status-* tokens as StatusBadge: in_progress
 * is the only solid-filled node (accent, + claimant chip), cancelled the
 * only grey with a strikethrough. The title links to task detail —
 * "selecting a node opens task detail" with native keyboard semantics.
 *
 * Handles follow the lens orientation the layout assigned (top/bottom in
 * the tree, left/right in the flow); the graph is read-only, so nothing
 * is connectable.
 */
export function TaskNode({
  data,
  selected,
  sourcePosition,
  targetPosition,
}: NodeProps<TaskNodeType>) {
  const { task } = data;
  return (
    <article
      className={styles.node}
      data-status={task.status}
      data-selected={selected ? "true" : undefined}
    >
      <Handle type="target" position={targetPosition ?? Position.Top} isConnectable={false} />
      <Link
        to={`/projects/${task.project_id}/tasks/${task.id}`}
        className={`${styles.title} nodrag`}
        title={task.title}
      >
        {task.title}
      </Link>
      <div className={styles.meta}>
        <StatusBadge status={task.status} />
        {task.status === "in_progress" && task.assignee ? (
          <IdentityChip identity={task.assignee} />
        ) : null}
        <AttemptBadge attempts={task.attempt_count} />
        {task.graph_role.map((role) => (
          <span key={role} className={styles.role}>
            {role}
            <span className="sr-only"> of the dependency flow</span>
          </span>
        ))}
      </div>
      <Handle type="source" position={sourcePosition ?? Position.Bottom} isConnectable={false} />
    </article>
  );
}
