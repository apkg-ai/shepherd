import { useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetGoal } from "../../api/generated/goals/goals";
import { useGetEpic } from "../../api/generated/epics/epics";
import { useListTasks } from "../../api/generated/tasks/tasks";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Card, CardGrid } from "../../components/CardGrid";
import { EligibilityPanel } from "../../components/EligibilityPanel";
import { LinkButton } from "../../components/LinkButton";
import { PageHeader } from "../../components/PageHeader";
import { PhaseBadge } from "../../components/PhaseBadge";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import styles from "./EpicDetailScreen.module.css";

export function EpicDetailScreen() {
  const { t } = useTranslation();
  const { projectId, epicId } = useParams<{ projectId: string; epicId: string }>();
  const project = useGetProject(projectId!);
  const epic = useGetEpic(projectId!, epicId!);
  const goal = useGetGoal(projectId!, epic.data?.goal_id ?? "", {
    query: { enabled: Boolean(epic.data?.goal_id) },
  });
  const tasks = useListTasks(projectId!, { epic_id: epicId! });

  // Check errors on independent queries before gated ones (goal depends on epic)
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
        description={epic.data.description || undefined}
        actions={
          <LinkButton to={`/projects/${projectId}/epics/${epicId}/edit`}>
            {t("epic.detail.edit")}
          </LinkButton>
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

      <section aria-label={t("epic.detail.tasksTitle")}>
        <div className={styles.sectionHeader}>
          <h3 className={styles.sectionTitle}>{t("epic.detail.tasksTitle")}</h3>
          <LinkButton to={`/projects/${projectId}/epics/${epicId}/tasks/new`} variant="primary">
            {t("epic.detail.newTask")}
          </LinkButton>
        </div>
        {allTasks.length === 0 ? (
          <EmptyState title={t("epic.detail.emptyTasksTitle")} icon="📋">
            <p className={styles.emptyHint}>{t("epic.detail.emptyTasksCta")}</p>
          </EmptyState>
        ) : (
          <CardGrid>
            {allTasks.map((task) => (
              <Card
                key={task.id}
                to={`/projects/${projectId}/tasks/${task.id}`}
                title={task.title}
                badge={
                  <span className={styles.badges}>
                    <StatusBadge status={task.status} />
                    <PhaseBadge phase={task.phase} />
                  </span>
                }
                meta={task.type_key}
              />
            ))}
          </CardGrid>
        )}
      </section>
    </>
  );
}
