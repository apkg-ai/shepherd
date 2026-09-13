import type { TaskType } from "../api/generated/model";
import styles from "./TypeBadge.module.css";

const TYPE_META: Record<TaskType, { symbol: string; label: string }> = {
  epic: { symbol: "◆", label: "Epic" },
  code: { symbol: "⌘", label: "Code" },
  question: { symbol: "?", label: "Question" },
  refactor: { symbol: "⚙", label: "Refactor" },
  review: { symbol: "⚑", label: "Review" },
  research: { symbol: "◎", label: "Research" },
};

/**
 * Type badge with a per-type symbol and color, driven by --type-* tokens.
 * Follows the same data-attribute pattern as StatusBadge.
 */
export function TypeBadge({ type }: { type: TaskType }) {
  const { symbol, label } = TYPE_META[type];
  return (
    <span className={styles.badge} data-type={type}>
      <span aria-hidden="true">{symbol} </span>
      {label}
    </span>
  );
}
