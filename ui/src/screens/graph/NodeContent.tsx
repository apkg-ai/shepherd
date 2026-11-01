import { useTranslation } from "react-i18next";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import type { Counts } from "../../api/generated/model/counts";
import styles from "./NodeContent.module.css";

export interface NodeContentProps {
  title: string;
  status: string;
  counts: Counts;
  expandable: boolean;
  expanded: boolean;
  selected: boolean;
  dimmed: boolean;
  onExpandClick?: (e: React.MouseEvent) => void;
}

/* v8 ignore start -- rendered inside React Flow; covered by E2E */
export function NodeContent({
  title,
  status,
  counts,
  expandable,
  expanded,
  selected,
  dimmed,
  onExpandClick,
}: NodeContentProps) {
  const { t } = useTranslation();

  return (
    <div
      className={styles.node}
      data-status={status}
      data-selected={selected || false}
      data-dimmed={dimmed || false}
      data-expanded={expanded || false}
    >
      <div className={styles.content}>
        <div className={styles.title}>{title}</div>
        <div className={styles.tags}>
          <StatusBadge status={status} />
          {counts.total > 0 && <ProgressCounts counts={counts} />}
        </div>
      </div>
      {expandable && onExpandClick && (
        <button
          type="button"
          className={styles.expandToggle}
          onClick={onExpandClick}
          aria-label={expanded ? t("graph.action.collapseTasks") : t("graph.action.expandTasks")}
        >
          {expanded ? "▲" : "▼"}
        </button>
      )}
    </div>
  );
}
/* v8 ignore stop */
