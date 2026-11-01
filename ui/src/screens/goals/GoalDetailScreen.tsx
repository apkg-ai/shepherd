import { useState } from "react";
import { useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetGoal } from "../../api/generated/goals/goals";
import { useListEpics } from "../../api/generated/epics/epics";
import type { Epic } from "../../api/generated/model/epic";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Card, CardGrid } from "../../components/CardGrid";
import { LinkButton } from "../../components/LinkButton";
import { PageHeader } from "../../components/PageHeader";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import { StatusFilter } from "../../components/StatusFilter";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import styles from "./GoalDetailScreen.module.css";

const ACTIVE_STATUSES = new Set(["proposed", "open", "active"]);
const DONE_STATUSES = new Set(["done", "cancelled"]);

function splitEpics(epics: Epic[]) {
  const active = epics.filter((e) => ACTIVE_STATUSES.has(e.status));
  const done = epics.filter((e) => DONE_STATUSES.has(e.status));
  return { active, done };
}

export function GoalDetailScreen() {
  const { t } = useTranslation();
  const { projectId, goalId } = useParams<{ projectId: string; goalId: string }>();
  const project = useGetProject(projectId!);
  const goal = useGetGoal(projectId!, goalId!);
  const epics = useListEpics(projectId!, { goal_id: goalId! });
  const [filter, setFilter] = useState<"active" | "done" | "all">("active");

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
  const { active, done } = splitEpics(allEpics);
  const visible = filter === "active" ? active : filter === "done" ? done : allEpics;

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
          <LinkButton to={`/projects/${projectId}/goals/${goalId}/edit`}>
            {t("goal.detail.edit")}
          </LinkButton>
        }
      >
        <div className={styles.meta}>
          {goal.data.completed ? <StatusBadge status="done" /> : null}
          {goal.data.epic_counts.total > 0 ? (
            <ProgressCounts counts={goal.data.epic_counts} />
          ) : null}
        </div>
      </PageHeader>

      <section aria-label={t("goal.detail.epicsTitle")}>
        <div className={styles.sectionHeader}>
          <h3 className={styles.sectionTitle}>{t("goal.detail.epicsTitle")}</h3>
          <LinkButton to={`/projects/${projectId}/goals/${goalId}/epics/new`} variant="primary">
            {t("goal.detail.newEpic")}
          </LinkButton>
        </div>

        {allEpics.length > 0 ? (
          <StatusFilter
            value={filter}
            onChange={setFilter}
            counts={{ active: active.length, done: done.length, total: allEpics.length }}
          />
        ) : null}

        {allEpics.length === 0 ? (
          <EmptyState title={t("goal.detail.emptyEpicsTitle")} icon="🏗️">
            <p className={styles.emptyHint}>{t("goal.detail.emptyEpicsCta")}</p>
          </EmptyState>
        ) : visible.length === 0 ? (
          <p className={styles.noResults}>{t("filter.status.noResults")}</p>
        ) : (
          <CardGrid>
            {visible.map((epic) => (
              <Card
                key={epic.id}
                to={`/projects/${projectId}/epics/${epic.id}`}
                title={epic.title}
                status={epic.status}
                tags={
                  <>
                    <StatusBadge status={epic.status} />
                    {epic.task_counts.total > 0 ? (
                      <ProgressCounts counts={epic.task_counts} />
                    ) : null}
                  </>
                }
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
