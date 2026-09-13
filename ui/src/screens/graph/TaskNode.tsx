import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useContext } from "react";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { StatusBadge } from "../../components/StatusBadge";
import { NODE_HEIGHT, NODE_WIDTH, type TaskNodeType } from "../../lib/graphLayout";
import { GraphActionsContext } from "./GraphActions";
import styles from "./TaskNode.module.css";

/**
 * A task on the graph canvas, speaking the status language from
 * docs/04-ui.md via the same --status-* tokens as StatusBadge: in_progress
 * is the only solid-filled node (accent, + claimant chip), cancelled the
 * only grey with a strikethrough. The title is a button whose click bubbles
 * to the canvas' onNodeClick — selection opens the side panel in place
 * (full detail is a link inside it), with keyboard access for free.
 *
 * When `childCount > 0` (decomposition lens), an expand/collapse chip
 * renders to toggle progressive disclosure. Its click stops propagation so
 * it doesn't trigger onNodeClick → selection.
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
  const { task, childCount, expanded, faded } = data;
  const { toggleExpand, lens } = useContext(GraphActionsContext);

  // "start" AND "end" is the derived default for tasks with no dependency
  // edges at all — as chips it's noise, so only real boundaries get marked.
  const roles =
    task.graph_role.includes("start") && task.graph_role.includes("end")
      ? task.graph_role.filter((role) => role === "milestone")
      : task.graph_role;
  const handleExpandClick = (event: React.MouseEvent) => {
    if (lens === "tree") {
      // Tree lens: toggle subtree, don't also select.
      event.stopPropagation();
      toggleExpand(task.id);
    }
    // Flow lens: let click bubble to onNodeClick → selection → side panel
    // with the rich subtask list. Inline expansion tracked in #52.
  };

  return (
    <article
      className={styles.node}
      style={{ width: NODE_WIDTH, height: NODE_HEIGHT }}
      data-status={task.status}
      data-selected={selected ? "true" : undefined}
      data-faded={faded ? "true" : undefined}
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
        {childCount > 0 ? (
          <button
            type="button"
            className={`${styles.expandChip} nodrag`}
            onClick={handleExpandClick}
            aria-expanded={expanded}
            aria-label={`${expanded ? "Collapse" : "Expand"} ${childCount} subtask${childCount === 1 ? "" : "s"}`}
          >
            <span aria-hidden="true">{expanded ? "▾" : "▸"}</span> {childCount}
          </button>
        ) : null}
      </div>
      <Handle type="source" position={sourcePosition ?? Position.Bottom} isConnectable={false} />
    </article>
  );
}
