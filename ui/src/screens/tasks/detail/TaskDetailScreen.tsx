import { useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../../api/generated/projects/projects";
import { useGetEpic } from "../../../api/generated/epics/epics";
import { useGetGoal } from "../../../api/generated/goals/goals";
import { useGetTask } from "../../../api/generated/tasks/tasks";
import { Breadcrumbs } from "../../../components/Breadcrumbs";
import { EligibilityPanel } from "../../../components/EligibilityPanel";
import { LinkButton } from "../../../components/LinkButton";
import { PageHeader } from "../../../components/PageHeader";
import { PhaseBadge } from "../../../components/PhaseBadge";
import { StatusBadge } from "../../../components/StatusBadge";
import { ErrorState, LoadingState } from "../../../components/states";
import { TaskActions } from "./TaskActions";
import styles from "./TaskDetailScreen.module.css";

export function TaskDetailScreen() {
  const { t } = useTranslation();
  const { projectId, taskId } = useParams<{ projectId: string; taskId: string }>();
  const project = useGetProject(projectId!);
  const task = useGetTask(projectId!, taskId!);

  const epic = useGetEpic(projectId!, task.data?.epic_id ?? "", {
    query: { enabled: Boolean(task.data?.epic_id) },
  });
  const goal = useGetGoal(projectId!, epic.data?.goal_id ?? "", {
    query: { enabled: Boolean(epic.data?.goal_id) },
  });

  if (project.isPending || task.isPending) return <LoadingState />;
  if (task.isError) {
    return <ErrorState error={task.error} onRetry={() => void task.refetch()} />;
  }
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }

  const taskData = task.data;

  return (
    <>
      <Breadcrumbs
        items={[
          { label: t("common.nav.projects"), to: "/" },
          { label: project.data.name, to: `/projects/${projectId}` },
          ...(goal.data
            ? [{ label: goal.data.title, to: `/projects/${projectId}/goals/${goal.data.id}` }]
            : []),
          ...(epic.data
            ? [{ label: epic.data.title, to: `/projects/${projectId}/epics/${epic.data.id}` }]
            : []),
          { label: taskData.title },
        ]}
      />

      <PageHeader
        title={taskData.title}
        description={taskData.description || undefined}
        actions={
          <LinkButton to={`/projects/${projectId}/tasks/${taskId}/edit`}>
            {t("task.detail.edit")}
          </LinkButton>
        }
      >
        <div className={styles.badges}>
          <StatusBadge status={taskData.status} />
          <PhaseBadge phase={taskData.phase} />
        </div>
      </PageHeader>

      <section className={styles.metadata} aria-label={t("task.detail.metadata")}>
        <dl className={styles.metaList}>
          <div className={styles.metaItem}>
            <dt>{t("task.detail.type")}</dt>
            <dd>{taskData.type_key}</dd>
          </div>
          <div className={styles.metaItem}>
            <dt>{t("task.detail.planningRequired")}</dt>
            <dd>
              {taskData.planning_required
                ? t("task.detail.planningRequiredYes")
                : t("task.detail.planningRequiredNo")}
            </dd>
          </div>
          <div className={styles.metaItem}>
            <dt>{t("task.detail.planReview")}</dt>
            <dd>{taskData.plan_review}</dd>
          </div>
          <div className={styles.metaItem}>
            <dt>{t("task.detail.workReview")}</dt>
            <dd>{taskData.work_review}</dd>
          </div>
        </dl>
      </section>

      <EligibilityPanel eligibility={taskData.eligibility} projectId={projectId!} />

      <TaskActions
        projectId={projectId!}
        taskId={taskId!}
        revision={taskData.revision}
        eligibility={taskData.eligibility}
      />
    </>
  );
}
