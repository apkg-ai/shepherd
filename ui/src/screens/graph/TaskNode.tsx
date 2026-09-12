import { Handle, Position, type NodeProps } from "@xyflow/react";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { StatusBadge } from "../../components/StatusBadge";
import type { TaskNodeType } from "../../lib/graphLayout";
import styles from "./TaskNode.module.css";

/**
 * A task on the graph canvas, speaking the status language from
 * docs/04-ui.md via the same --status-* tokens as StatusBadge: in_progress
 * is the only solid-filled node (accent, + claimant chip), cancelled the
 * only grey with a strikethrough. The title is a button whose click bubbles
 * to the canvas' onNodeClick — selection opens the side panel in place
 * (full detail is a link inside it), with keyboard access for free.
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
  // "start" AND "end" is the derived default for tasks with no dependency
  // edges at all — as chips it's noise, so only real boundaries get marked.
  const roles =
    task.graph_role.includes("start") && task.graph_role.includes("end")
      ? task.graph_role.filter((role) => role === "milestone")
      : task.graph_role;
  return (
    <article
      className={styles.node}
      data-status={task.status}
      data-selected={selected ? "true" : undefined}
    >
      <Handle type="target" position={targetPosition ?? Position.Top} isConnectable={false} />
      {/* No handler: the click bubbles to React Flow's node wrapper, which
          fires onNodeClick → selection. The button exists for keyboard and
          assistive-tech reach. */}
      <button type="button" className={`${styles.title} nodrag`} title={task.title}>
        {task.title}
      </button>
      <div className={styles.meta}>
        <StatusBadge status={task.status} />
        {task.status === "in_progress" && task.assignee ? (
          <IdentityChip identity={task.assignee} />
        ) : null}
        <AttemptBadge attempts={task.attempt_count} />
        {roles.map((role) => (
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
