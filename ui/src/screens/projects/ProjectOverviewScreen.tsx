import { Link, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useListGoals } from "../../api/generated/goals/goals";
import { Breadcrumbs } from "../../components/Breadcrumbs";
import { Button } from "../../components/Button";
import { Card, CardGrid } from "../../components/CardGrid";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";

export function ProjectOverviewScreen() {
  const { t } = useTranslation();
  const { projectId } = useParams<{ projectId: string }>();
  const project = useGetProject(projectId!);
  const goals = useListGoals(projectId!);

  if (project.isPending || goals.isPending) return <LoadingState />;
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (goals.isError) {
    return <ErrorState error={goals.error} onRetry={() => void goals.refetch()} />;
  }

  const allGoals = goals.data.items;

  return (
    <>
      <Breadcrumbs
        items={[{ label: t("common.nav.projects"), to: "/" }, { label: project.data.name }]}
      />
      <PageHeader
        title={project.data.name}
        description={project.data.description || undefined}
        actions={
          <Link to={`/projects/${projectId}/goals/new`}>
            <Button variant="primary">{t("project.overview.newGoal")}</Button>
          </Link>
        }
      />

      {allGoals.length === 0 ? (
        <EmptyState title={t("project.overview.emptyGoalsTitle")} icon="🎯">
          <Link to={`/projects/${projectId}/goals/new`}>
            <Button variant="primary">{t("project.overview.emptyGoalsCta")}</Button>
          </Link>
        </EmptyState>
      ) : (
        <CardGrid>
          {allGoals.map((goal) => (
            <Card
              key={goal.id}
              to={`/projects/${projectId}/goals/${goal.id}`}
              title={goal.title}
              description={goal.description || undefined}
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
  );
}
