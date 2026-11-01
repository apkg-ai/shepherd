import { forwardRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { GraphNodesItem } from "../../api/generated/model/graphNodesItem";
import { useGetTask } from "../../api/generated/tasks/tasks";
import { EligibilityPanel } from "../../components/EligibilityPanel";
import { LinkButton } from "../../components/LinkButton";
import { PhaseBadge } from "../../components/PhaseBadge";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import styles from "./GraphSidePanel.module.css";

interface GraphSidePanelProps {
  node: GraphNodesItem | null;
  level: "epic" | "task";
  projectId: string;
  onClose: () => void;
}

export const GraphSidePanel = forwardRef<HTMLElement, GraphSidePanelProps>(function GraphSidePanel(
  { node, level, projectId, onClose },
  ref,
) {
  const { t } = useTranslation();

  const { data: task, isLoading: taskLoading } = useGetTask(projectId, node?.id ?? "", {
    query: { enabled: level === "task" && node?.id !== undefined },
  });

  useEffect(() => {
    if (!node) return;
    const handler = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [node, onClose]);

  if (!node) return null;

  const detailPath =
    level === "epic"
      ? `/projects/${projectId}/epics/${node.id}`
      : `/projects/${projectId}/tasks/${node.id}`;

  return (
    <aside aria-label={t("graph.panel.label")} className={styles.panel} ref={ref} tabIndex={-1}>
      <header className={styles.header}>
        <h3 className={styles.title}>{node.title}</h3>
        <button
          type="button"
          className={styles.close}
          aria-label={t("graph.panel.close")}
          onClick={onClose}
        >
          &times;
        </button>
      </header>
      <div className={styles.content}>
        <div className={styles.meta}>
          <StatusBadge status={node.status} />
          {level === "task" && taskLoading && <span aria-busy="true" />}
          {level === "task" && task?.phase && <PhaseBadge phase={task.phase} />}
        </div>
        {node.counts.total > 0 && <ProgressCounts counts={node.counts} />}
        <EligibilityPanel eligibility={node.eligibility} projectId={projectId} />
        <LinkButton to={detailPath}>{t("graph.panel.openDetails")}</LinkButton>
      </div>
    </aside>
  );
});
