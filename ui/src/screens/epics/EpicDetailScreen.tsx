import { useState } from "react";
import { lazy, Suspense } from "react";
import { useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetGoal } from "../../api/generated/goals/goals";
import { useGetEpic, useGetEpicGraph } from "../../api/generated/epics/epics";
import { useListTasks } from "../../api/generated/tasks/tasks";
import type { Task } from "../../api/generated/model/task";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Card, CardGrid } from "../../components/CardGrid";
import { EligibilityPanel } from "../../components/EligibilityPanel";
import { LinkButton } from "../../components/LinkButton";
import { PageHeader } from "../../components/PageHeader";
import { PhaseBadge } from "../../components/PhaseBadge";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import { StatusFilter } from "../../components/StatusFilter";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import styles from "./EpicDetailScreen.module.css";

// Lazy so the React Flow + dagre stack loads only when a graph actually renders.
const GraphScreen = lazy(() =>
  import("../graph/GraphScreen").then((m) => ({ default: m.GraphScreen })),
);

const ACTIVE_STATUSES = new Set(["proposed", "open", "active"]);
const DONE_STATUSES = new Set(["done", "cancelled"]);

function splitTasks(tasks: Task[]) {
  const active = tasks.filter((t) => ACTIVE_STATUSES.has(t.status));
  const done = tasks.filter((t) => DONE_STATUSES.has(t.status));
  return { active, done };
}

export function EpicDetailScreen() {
  const { t } = useTranslation();
  const { projectId, epicId } = useParams<{ projectId: string; epicId: string }>();
  const project = useGetProject(projectId!);
  const epic = useGetEpic(projectId!, epicId!);
  const goal = useGetGoal(projectId!, epic.data?.goal_id ?? "", {
    query: { enabled: Boolean(epic.data?.goal_id) },
  });
  const tasks = useListTasks(projectId!, { epic_id: epicId! });
  // Only fetched when a graph will render — an epic without tasks shows the empty state.
  const graphQuery = useGetEpicGraph(projectId!, epicId!, {
    query: { enabled: (tasks.data?.items.length ?? 0) > 0 },
  });
  const [filter, setFilter] = useState<"active" | "done" | "all">("active");

  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (epic.isError) {
    return <ErrorState error={epic.error} onRetry={() => void epic.refetch()} />;
  }
  if (project.isPending || epic.isPending || goal.isPending || tasks.isPending) {
    return <LoadingState />;
  }
  if (goal.isError) {
    return <ErrorState error={goal.error} onRetry={() => void goal.refetch()} />;
  }
  if (tasks.isError) {
    return <ErrorState error={tasks.error} onRetry={() => void tasks.refetch()} />;
  }

  const allTasks = tasks.data.items;
  const { active, done } = splitTasks(allTasks);
  const visible = filter === "active" ? active : filter === "done" ? done : allTasks;

  return (
    <>
      <Breadcrumbs
        items={[
          { label: t("common.nav.projects"), to: "/" },
          { label: project.data.name, to: `/projects/${projectId}` },
          { label: goal.data.title, to: `/projects/${projectId}/goals/${epic.data.goal_id}` },
          { label: epic.data.title },
        ]}
      />

      <PageHeader
        title={epic.data.title}
        actions={
          <>
            <LinkButton to={`/projects/${projectId}/epics/${epicId}/tasks/new`} variant="primary">
              {t("epic.detail.newTask")}
            </LinkButton>
            <LinkButton to={`/projects/${projectId}/epics/${epicId}/edit`}>
              {t("epic.detail.edit")}
            </LinkButton>
          </>
        }
      >
        <div className={styles.meta}>
          <StatusBadge status={epic.data.status} />
          {epic.data.task_counts.total > 0 ? (
            <ProgressCounts counts={epic.data.task_counts} />
          ) : null}
        </div>
      </PageHeader>

      <EligibilityPanel eligibility={epic.data.eligibility} projectId={projectId!} />

      {allTasks.length === 0 ? (
        <EmptyState title={t("epic.detail.emptyTasksTitle")} icon="📋">
          <p className={styles.emptyHint}>{t("epic.detail.emptyTasksCta")}</p>
        </EmptyState>
      ) : (
        <Suspense fallback={<LoadingState />}>
          <GraphScreen
            projectId={projectId!}
            level="task"
            scopeId={epicId!}
            graphQuery={graphQuery}
            detailPath={(nodeId) => `/projects/${projectId}/tasks/${nodeId}`}
            listContent={
              <>
                <StatusFilter
                  value={filter}
                  onChange={setFilter}
                  counts={{ active: active.length, done: done.length, total: allTasks.length }}
                />

                {visible.length === 0 ? (
                  <p className={styles.noResults}>{t("filter.status.noResults")}</p>
                ) : (
                  <CardGrid>
                    {visible.map((task) => (
                      <Card
                        key={task.id}
                        to={`/projects/${projectId}/tasks/${task.id}`}
                        title={task.title}
                        status={task.status}
                        tags={
                          <>
                            <StatusBadge status={task.status} />
                            <PhaseBadge phase={task.phase} />
                          </>
                        }
                        meta={task.type_key}
                      />
                    ))}
                  </CardGrid>
                )}
              </>
            }
          />
        </Suspense>
      )}
    </>
  );
}
