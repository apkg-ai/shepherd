import type { NodeProps, Node } from "@xyflow/react";
import { useTranslation } from "react-i18next";
import type { Counts } from "../../api/generated/model/counts";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
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
  const { t } = useTranslation();

  const handleExpandClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    data.onToggleExpand?.(data.goalId);
  };

  return (
    <div
      className={styles.group}
      data-testid={`goal-group-${id}`}
      data-completed={data.completed}
      data-expanded={data.expanded}
      aria-label={t("graph.group.label", { title: data.title })}
    >
      <div className={styles.header}>
        <button
          type="button"
          className={styles.expandToggle}
          onClick={handleExpandClick}
          aria-label={
            data.expanded ? t("graph.action.collapseTasks") : t("graph.action.expandTasks")
          }
        >
          {data.expanded ? "▲" : "▼"}
        </button>
        <span className={styles.title}>{data.title}</span>
        {data.completed && <StatusBadge status="done" />}
        {data.epicCounts.total > 0 && <ProgressCounts counts={data.epicCounts} />}
      </div>
    </div>
  );
}
/* v8 ignore stop */
