import { type FormEvent, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetEpic } from "../../api/generated/epics/epics";
import { useGetGoal } from "../../api/generated/goals/goals";
import {
  useGetTask,
  getTask,
  createTask,
  updateTask,
  getGetTaskQueryKey,
  getListTasksQueryKey,
} from "../../api/generated/tasks/tasks";
import { useListTaskTypes } from "../../api/generated/task-types/task-types";
import type { Task } from "../../api/generated/model/task";
import type { ReviewPolicy } from "../../api/generated/model/reviewPolicy";
import { useEntityForm } from "../../lib/useEntityForm";
import { Breadcrumbs, type BreadcrumbItem } from "../../components/Breadcrumbs";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { ConflictView } from "../../components/ConflictView";
import { FormField } from "../../components/FormField";
import inputStyles from "../../components/Input.module.css";
import { LinkButton } from "../../components/LinkButton";
import { PageHeader } from "../../components/PageHeader";
import { ErrorState, LoadingState } from "../../components/states";
import styles from "./TaskFormScreen.module.css";

export function TaskFormScreen() {
  const { t } = useTranslation();
  const { projectId, epicId, taskId } = useParams<{
    projectId: string;
    epicId: string;
    taskId: string;
  }>();
  const isEdit = Boolean(taskId);

  const project = useGetProject(projectId!);
  const task = useGetTask(projectId!, taskId!, { query: { enabled: isEdit } });

  // In edit mode, get epicId from the task; in create mode, from the URL
  const resolvedEpicId = isEdit && task.data ? task.data.epic_id : epicId;
  const epic = useGetEpic(projectId!, resolvedEpicId!, {
    query: { enabled: Boolean(resolvedEpicId) },
  });
  const goal = useGetGoal(projectId!, epic.data?.goal_id ?? "", {
    query: { enabled: Boolean(epic.data?.goal_id) },
  });
  const taskTypes = useListTaskTypes(projectId!);

  const isLoading =
    project.isPending ||
    (isEdit && task.isPending) ||
    !resolvedEpicId ||
    epic.isPending ||
    !epic.data?.goal_id ||
    goal.isPending ||
    taskTypes.isPending;

  if (isLoading) return <LoadingState />;
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (isEdit && task.isError) {
    return <ErrorState error={task.error} onRetry={() => void task.refetch()} />;
  }
  if (epic.isError) {
    return <ErrorState error={epic.error} onRetry={() => void epic.refetch()} />;
  }
  if (goal.isError) {
    return <ErrorState error={goal.error} onRetry={() => void goal.refetch()} />;
  }
  if (taskTypes.isError) {
    return <ErrorState error={taskTypes.error} onRetry={() => void taskTypes.refetch()} />;
  }

  const activeTypes = taskTypes.data.items.filter((tt) => !tt.archived);

  const breadcrumbItems: BreadcrumbItem[] = isEdit
    ? [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: goal.data!.title, to: `/projects/${projectId}/goals/${goal.data!.id}` },
        { label: epic.data!.title, to: `/projects/${projectId}/epics/${epic.data!.id}` },
        { label: task.data!.title, to: `/projects/${projectId}/tasks/${taskId}` },
        { label: t("task.form.editTitle") },
      ]
    : [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: goal.data!.title, to: `/projects/${projectId}/goals/${goal.data!.id}` },
        { label: epic.data!.title, to: `/projects/${projectId}/epics/${resolvedEpicId}` },
        { label: t("task.form.newTitle") },
      ];

  return (
    <TaskForm
      projectId={projectId!}
      epicId={resolvedEpicId!}
      taskId={taskId}
      isEdit={isEdit}
      breadcrumbItems={breadcrumbItems}
      initialTitle={isEdit ? task.data!.title : ""}
      initialDescription={isEdit ? task.data!.description : ""}
      initialRevision={isEdit ? task.data!.revision : 0}
      initialTypeKey={isEdit ? task.data!.type_key : (activeTypes[0]?.key ?? "")}
      initialPlanningRequired={
        isEdit ? task.data!.planning_required : project.data.settings.planning_required
      }
      initialPlanReview={isEdit ? task.data!.plan_review : project.data.settings.plan_review}
      initialWorkReview={isEdit ? task.data!.work_review : project.data.settings.work_review}
      taskTypes={activeTypes}
      key={`${projectId}:${taskId ?? "new"}`}
    />
  );
}

function TaskForm({
  projectId,
  epicId,
  taskId,
  isEdit,
  breadcrumbItems,
  initialTitle,
  initialDescription,
  initialRevision,
  initialTypeKey,
  initialPlanningRequired,
  initialPlanReview,
  initialWorkReview,
  taskTypes,
}: {
  projectId: string;
  epicId: string;
  taskId?: string;
  isEdit: boolean;
  breadcrumbItems: BreadcrumbItem[];
  initialTitle: string;
  initialDescription: string;
  initialRevision: number;
  initialTypeKey: string;
  initialPlanningRequired: boolean;
  initialPlanReview: ReviewPolicy;
  initialWorkReview: ReviewPolicy;
  taskTypes: Array<{ key: string; label: string }>;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const [typeKey, setTypeKey] = useState(initialTypeKey);
  const [planningRequired, setPlanningRequired] = useState(initialPlanningRequired);
  const [planReview, setPlanReview] = useState<ReviewPolicy>(initialPlanReview);
  const [workReview, setWorkReview] = useState<ReviewPolicy>(initialWorkReview);

  const form = useEntityForm<Task>({
    entityId: taskId,
    initialTitle,
    initialDescription,
    initialRevision,
    createFn: (data, opts) =>
      createTask(projectId, data as { epic_id: string; title: string; type_key: string }, opts),
    onCreateSuccess: (data) => {
      void navigate(`/projects/${projectId}/tasks/${data.id}`, { replace: true });
    },
    updateFn: (data, opts) => updateTask(projectId, taskId!, data as { title?: string }, opts),
    refetchFn: () => getTask(projectId, taskId!),
    onUpdateSuccess: () => {
      void navigate(`/projects/${projectId}/tasks/${taskId}`, { replace: true });
    },
    getTitle: (tk) => tk.title,
    getDescription: (tk) => tk.description,
    getRevision: (tk) => tk.revision,
    listQueryKey: getListTasksQueryKey(projectId),
    detailQueryKey: getGetTaskQueryKey(projectId, taskId!),
    createBackPath: `/projects/${projectId}/epics/${epicId}`,
    editBackPath: `/projects/${projectId}/tasks/${taskId}`,
    conflictInspectKey: "task.form.conflictPreviousSubmission",
    conflictInspectPath: `/projects/${projectId}/epics/${epicId}`,
  });

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (isEdit) {
      form.submitUpdate({
        title: form.title.trim(),
        description: form.description.trim(),
        type_key: typeKey,
        planning_required: planningRequired,
        plan_review: planReview,
        work_review: workReview,
      });
    } else {
      form.submitCreate({
        epic_id: epicId,
        title: form.title.trim(),
        description: form.description.trim() || undefined,
        type_key: typeKey,
        planning_required: planningRequired,
        plan_review: planReview,
        work_review: workReview,
      });
    }
  };

  if (form.conflict) {
    return (
      <>
        <Breadcrumbs items={breadcrumbItems} />
        <PageHeader title={t("task.conflict.title")} description={t("task.conflict.description")} />
        <ConflictView
          fields={[
            {
              label: t("task.form.titleLabel"),
              local: form.conflict.localTitle,
              remote: form.conflict.remoteTitle,
            },
            {
              label: t("task.form.descriptionLabel"),
              local: form.conflict.localDescription,
              remote: form.conflict.remoteDescription,
            },
          ]}
          onRetry={form.handleConflictRetry}
          onDiscard={form.handleConflictDiscard}
        />
      </>
    );
  }

  return (
    <>
      <Breadcrumbs items={breadcrumbItems} />
      <PageHeader title={isEdit ? t("task.form.editTitle") : t("task.form.newTitle")} />

      <form className={styles.form} onSubmit={handleSubmit} noValidate>
        <FormField label={t("task.form.titleLabel")} error={form.errors.title}>
          {(props) => (
            <input
              {...props}
              type="text"
              className={inputStyles.input}
              value={form.title}
              onChange={(e) => {
                form.setTitle(e.target.value);
                form.setDirty();
              }}
              required
            />
          )}
        </FormField>

        <FormField label={t("task.form.descriptionLabel")} error={form.errors.description}>
          {(props) => (
            <textarea
              {...props}
              className={inputStyles.textarea}
              value={form.description}
              onChange={(e) => {
                form.setDescription(e.target.value);
                form.setDirty();
              }}
              rows={6}
            />
          )}
        </FormField>

        <FormField label={t("task.form.typeLabel")} error={form.errors.type_key}>
          {(props) => (
            <select
              {...props}
              className={inputStyles.input}
              value={typeKey}
              onChange={(e) => {
                setTypeKey(e.target.value);
                form.setDirty();
              }}
            >
              {taskTypes.map((tt) => (
                <option key={tt.key} value={tt.key}>
                  {tt.label}
                </option>
              ))}
            </select>
          )}
        </FormField>

        <FormField label={t("task.form.planningRequiredLabel")}>
          {(props) => (
            <input
              {...props}
              type="checkbox"
              className={inputStyles.checkbox}
              checked={planningRequired}
              onChange={(e) => {
                setPlanningRequired(e.target.checked);
                form.setDirty();
              }}
            />
          )}
        </FormField>

        <FormField label={t("task.form.planReviewLabel")} error={form.errors.plan_review}>
          {(props) => (
            <select
              {...props}
              className={inputStyles.input}
              value={planReview}
              onChange={(e) => {
                setPlanReview(e.target.value as ReviewPolicy);
                form.setDirty();
              }}
            >
              <option value="human">Human</option>
              <option value="agent">Agent</option>
              <option value="none">None</option>
            </select>
          )}
        </FormField>

        <FormField label={t("task.form.workReviewLabel")} error={form.errors.work_review}>
          {(props) => (
            <select
              {...props}
              className={inputStyles.input}
              value={workReview}
              onChange={(e) => {
                setWorkReview(e.target.value as ReviewPolicy);
                form.setDirty();
              }}
            >
              <option value="human">Human</option>
              <option value="agent">Agent</option>
              <option value="none">None</option>
            </select>
          )}
        </FormField>

        {form.errors._form ? <p className={styles.formError}>{form.errors._form}</p> : null}
        {form.createConflict && form.conflictInspectPath ? (
          <p className={styles.formLink}>
            <LinkButton to={form.conflictInspectPath}>{t("task.form.inspectTasks")}</LinkButton>
          </p>
        ) : null}
        <div className={styles.actions}>
          <Button type="button" onClick={form.handleCancel}>
            {t("common.action.cancel")}
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={form.isPending}
            disabled={!form.title.trim() || !typeKey}
          >
            {isEdit ? t("task.form.updateSubmit") : t("task.form.createSubmit")}
          </Button>
        </div>
      </form>

      <ConfirmDialog
        open={form.confirmDiscard}
        title={t("task.form.discardTitle")}
        confirmLabel={t("task.form.discardConfirm")}
        onConfirm={form.navigateBack}
        onCancel={() => form.setConfirmDiscard(false)}
      >
        <p>{t("task.form.discardDescription")}</p>
      </ConfirmDialog>
    </>
  );
}
