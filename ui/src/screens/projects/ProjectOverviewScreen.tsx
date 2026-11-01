import { useState } from "react";
import { useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useListGoals } from "../../api/generated/goals/goals";
import type { Goal } from "../../api/generated/model/goal";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Card, CardGrid } from "../../components/CardGrid";
import { LinkButton } from "../../components/LinkButton";
import { PageHeader } from "../../components/PageHeader";
import { ProgressCounts } from "../../components/ProgressCounts";
import { StatusBadge } from "../../components/StatusBadge";
import { StatusFilter } from "../../components/StatusFilter";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { ProjectGraphView } from "../graph/ProjectGraphView";
import styles from "./ProjectOverviewScreen.module.css";

function splitGoals(goals: Goal[]) {
  const active = goals.filter((g) => !g.completed);
  const done = goals.filter((g) => g.completed);
  return { active, done };
}

export function ProjectOverviewScreen() {
  const { t } = useTranslation();
  const { projectId } = useParams<{ projectId: string }>();
  const project = useGetProject(projectId!);
  const goals = useListGoals(projectId!);
  const [filter, setFilter] = useState<"active" | "done" | "all">("active");

  if (project.isPending || goals.isPending) return <LoadingState />;
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (goals.isError) {
    return <ErrorState error={goals.error} onRetry={() => void goals.refetch()} />;
  }

  const allGoals = goals.data.items;
  const { active, done } = splitGoals(allGoals);
  const visible = filter === "active" ? active : filter === "done" ? done : allGoals;

  return (
    <>
      <Breadcrumbs
        items={[{ label: t("common.nav.projects"), to: "/" }, { label: project.data.name }]}
      />
      <PageHeader
        title={project.data.name}
        description={project.data.description || undefined}
        actions={
          <LinkButton to={`/projects/${projectId}/goals/new`} variant="primary">
            {t("project.overview.newGoal")}
          </LinkButton>
        }
      >
        {allGoals.length > 0 || project.data.epic_counts.total > 0 ? (
          <div className={styles.summary}>
            <span className={styles.summaryItem}>
              {t("project.overview.goalCount", { count: allGoals.length })}
            </span>
            {project.data.epic_counts.total > 0 ? (
              <>
                <span className={styles.summaryDot}>·</span>
                <ProgressCounts counts={project.data.epic_counts} />
              </>
            ) : null}
          </div>
        ) : null}
      </PageHeader>

      {allGoals.length === 0 ? (
        <EmptyState title={t("project.overview.emptyGoalsTitle")} icon="🎯">
          <LinkButton to={`/projects/${projectId}/goals/new`} variant="primary">
            {t("project.overview.emptyGoalsCta")}
          </LinkButton>
        </EmptyState>
      ) : (
        <ProjectGraphView
          projectId={projectId!}
          listContent={
            <>
              <StatusFilter
                value={filter}
                onChange={setFilter}
                counts={{ active: active.length, done: done.length, total: allGoals.length }}
              />
              {visible.length === 0 ? (
                <p className={styles.noResults}>{t("filter.status.noResults")}</p>
              ) : (
                <CardGrid>
                  {visible.map((goal) => (
                    <Card
                      key={goal.id}
                      to={`/projects/${projectId}/goals/${goal.id}`}
                      title={goal.title}
                      description={goal.description || undefined}
                      status={goal.completed ? "done" : undefined}
                      badge={goal.completed ? <StatusBadge status="done" /> : undefined}
                      meta={
                        goal.epic_counts.total > 0
                          ? t("project.overview.epicCount", { count: goal.epic_counts.total })
                          : undefined
                      }
                    />
                  ))}
                </CardGrid>
              )}
            </>
          }
        />
      )}
    </>
  );
}
