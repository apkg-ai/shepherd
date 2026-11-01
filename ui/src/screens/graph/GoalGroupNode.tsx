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
}

export type GoalGroupNodeType = Node<GoalGroupData, "goalGroup">;

export function GoalGroupNode({ id, data }: NodeProps<GoalGroupNodeType>) {
  const { t } = useTranslation();

  return (
    <div
      className={styles.group}
      data-testid={`goal-group-${id}`}
      data-completed={data.completed}
      aria-label={t("graph.group.label", { title: data.title })}
    >
      <div className={styles.header}>
        <span className={styles.title}>{data.title}</span>
        {data.completed && <StatusBadge status="done" />}
        {data.epicCounts.total > 0 && <ProgressCounts counts={data.epicCounts} />}
      </div>
    </div>
  );
}
