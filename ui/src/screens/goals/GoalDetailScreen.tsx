import { Link, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetGoal } from "../../api/generated/goals/goals";
import { useListEpics } from "../../api/generated/epics/epics";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Button } from "../../components/Button";
import { CardGrid, StaticCard } from "../../components/CardGrid";
import { PageHeader } from "../../components/PageHeader";
import { StatusBadge } from "../../components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import styles from "./GoalDetailScreen.module.css";

export function GoalDetailScreen() {
  const { t } = useTranslation();
  const { projectId, goalId } = useParams<{ projectId: string; goalId: string }>();
  const project = useGetProject(projectId!);
  const goal = useGetGoal(projectId!, goalId!);
  const epics = useListEpics(projectId!, { goal_id: goalId! });

  if (project.isPending || goal.isPending || epics.isPending) {
    return <LoadingState />;
  }
  if (goal.isError) {
    return <ErrorState error={goal.error} onRetry={() => void goal.refetch()} />;
  }
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (epics.isError) {
    return <ErrorState error={epics.error} onRetry={() => void epics.refetch()} />;
  }

  const allEpics = epics.data.items;

  return (
    <>
      <Breadcrumbs
        items={[
          { label: t("common.nav.projects"), to: "/" },
          { label: project.data.name, to: `/projects/${projectId}` },
          { label: goal.data.title },
        ]}
      />

      <PageHeader
        title={goal.data.title}
        description={goal.data.description || undefined}
        actions={
          <Link to={`/projects/${projectId}/goals/${goalId}/edit`}>
            <Button>{t("goal.detail.edit")}</Button>
          </Link>
        }
      >
        {goal.data.epic_counts.total > 0 ? (
          <span className={styles.counts}>
            {t("goal.detail.epicCount", { count: goal.data.epic_counts.total })}
          </span>
        ) : null}
      </PageHeader>

      <section aria-label={t("goal.detail.epicsTitle")}>
        <h3 className={styles.sectionTitle}>{t("goal.detail.epicsTitle")}</h3>
        {allEpics.length === 0 ? (
          <EmptyState title={t("goal.detail.emptyEpicsTitle")} icon="🏗️">
            <p className={styles.emptyHint}>{t("goal.detail.emptyEpicsCta")}</p>
          </EmptyState>
        ) : (
          <CardGrid>
            {allEpics.map((epic) => (
              <StaticCard
                key={epic.id}
                title={epic.title}
                badge={<StatusBadge status={epic.status} />}
                meta={
                  epic.task_counts.total > 0
                    ? t("goal.detail.taskCount", { count: epic.task_counts.total })
                    : undefined
                }
              />
            ))}
          </CardGrid>
        )}
      </section>
    </>
  );
}
