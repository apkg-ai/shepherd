import type { TaskStatus } from "../api/generated/model";
import styles from "./StatusBadge.module.css";

const LABELS: Record<TaskStatus, string> = {
  proposed: "Proposed",
  approved: "Approved",
  ready: "Ready",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
  blocked: "Blocked",
  cancelled: "Cancelled",
};

/**
 * The status visual language from docs/04-ui.md, driven entirely by
 * --status-* tokens so the S8 graph can reuse them.
 */
export function StatusBadge({ status }: { status: TaskStatus }) {
  return (
    <span className={styles.badge} data-status={status}>
      {status === "in_review" ? <span aria-hidden="true">⚑ </span> : null}
      {LABELS[status]}
    </span>
  );
}
