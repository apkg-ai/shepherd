import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useListGoals } from "../../api/generated/goals/goals";
import { useListEpics } from "../../api/generated/epics/epics";
import { useListTasks } from "../../api/generated/tasks/tasks";
import type { Goal } from "../../api/generated/model/goal";
import type { Epic } from "../../api/generated/model/epic";
import { StatusBadge } from "../../components/StatusBadge";
import { PhaseBadge } from "../../components/PhaseBadge";
import { ProgressCounts } from "../../components/ProgressCounts";
import { LoadingState } from "../../components/states";
import styles from "./TabsView.module.css";

interface TabsViewProps {
  projectId: string;
  hideCompleted: boolean;
}

export function TabsView({ projectId, hideCompleted }: TabsViewProps) {
  const { t } = useTranslation();
  const { data, isLoading, isError } = useListGoals(projectId);

  if (isLoading) return <LoadingState />;
  if (isError) return <div role="alert">{t("tabs.view.error")}</div>;

  const goals = (data?.items ?? []).filter((g) => !hideCompleted || !g.completed);

  return (
    <div role="group" aria-label={t("tabs.view.label")}>
      {goals.map((goal) => (
        <GoalRow key={goal.id} goal={goal} projectId={projectId} hideCompleted={hideCompleted} />
      ))}
    </div>
  );
}

function GoalRow({
  goal,
  projectId,
  hideCompleted,
}: {
  goal: Goal;
  projectId: string;
  hideCompleted: boolean;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const label = expanded ? t("tabs.view.collapseGoal") : t("tabs.view.expandGoal");

  return (
    <>
      <div className={`${styles.goalRow}${goal.completed ? ` ${styles.completed}` : ""}`}>
        <button
          className={styles.toggle}
          onClick={() => setExpanded(!expanded)}
          aria-expanded={expanded}
          aria-label={label}
        >
          {expanded ? "▼" : "▶"}
        </button>
        <span className={styles.title}>
          <Link className={styles.titleLink} to={`/projects/${projectId}/goals/${goal.id}`}>
            {goal.title}
          </Link>
        </span>
        <span className={styles.meta}>
          {goal.completed && <StatusBadge status="done" />}
          <ProgressCounts counts={goal.epic_counts} />
        </span>
      </div>
      {expanded && (
        <EpicList projectId={projectId} goalId={goal.id} hideCompleted={hideCompleted} />
      )}
    </>
  );
}

function EpicList({
  projectId,
  goalId,
  hideCompleted,
}: {
  projectId: string;
  goalId: string;
  hideCompleted: boolean;
}) {
  const { t } = useTranslation();
  const { data, isLoading, isError } = useListEpics(projectId, { goal_id: goalId });

  if (isLoading) {
    return <div className={styles.loadingEpic}>{t("tabs.view.loading")}</div>;
  }
  if (isError) {
    return <div role="alert">{t("tabs.view.error")}</div>;
  }

  const epics = (data?.items ?? []).filter(
    (e) => !hideCompleted || (e.status !== "done" && e.status !== "cancelled"),
  );

  return (
    <>
      {epics.map((epic) => (
        <EpicRow key={epic.id} epic={epic} projectId={projectId} hideCompleted={hideCompleted} />
      ))}
    </>
  );
}

function EpicRow({
  epic,
  projectId,
  hideCompleted,
}: {
  epic: Epic;
  projectId: string;
  hideCompleted: boolean;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const isDone = epic.status === "done" || epic.status === "cancelled";
  const label = expanded ? t("tabs.view.collapseEpic") : t("tabs.view.expandEpic");

  return (
    <>
      <div className={`${styles.epicRow}${isDone ? ` ${styles.completed}` : ""}`}>
        <button
          className={styles.toggle}
          onClick={() => setExpanded(!expanded)}
          aria-expanded={expanded}
          aria-label={label}
        >
          {expanded ? "▼" : "▶"}
        </button>
        <span className={styles.title}>
          <Link className={styles.titleLink} to={`/projects/${projectId}/epics/${epic.id}`}>
            {epic.title}
          </Link>
        </span>
        <span className={styles.meta}>
          <StatusBadge status={epic.status} />
          <ProgressCounts counts={epic.task_counts} />
        </span>
      </div>
      {expanded && (
        <TaskList projectId={projectId} epicId={epic.id} hideCompleted={hideCompleted} />
      )}
    </>
  );
}

function TaskList({
  projectId,
  epicId,
  hideCompleted,
}: {
  projectId: string;
  epicId: string;
  hideCompleted: boolean;
}) {
  const { t } = useTranslation();
  const { data, isLoading, isError } = useListTasks(projectId, { epic_id: epicId });

  if (isLoading) {
    return <div className={styles.loadingTask}>{t("tabs.view.loading")}</div>;
  }
  if (isError) {
    return <div role="alert">{t("tabs.view.error")}</div>;
  }

  const tasks = (data?.items ?? []).filter(
    (tk) => !hideCompleted || (tk.status !== "done" && tk.status !== "cancelled"),
  );

  return (
    <>
      {tasks.map((task) => {
        const isDone = task.status === "done" || task.status === "cancelled";
        return (
          <div
            key={task.id}

            className={`${styles.taskRow}${isDone ? ` ${styles.completed}` : ""}`}
          >
            <span className={styles.title}>
              <Link className={styles.titleLink} to={`/projects/${projectId}/tasks/${task.id}`}>
                {task.title}
              </Link>
            </span>
            <span className={styles.meta}>
              <StatusBadge status={task.status} />
              <PhaseBadge phase={task.phase} />
              <span className={styles.typeKey}>{task.type_key}</span>
            </span>
          </div>
        );
      })}
    </>
  );
}
