import { type FormEvent } from "react";
import { useNavigate, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import { useGetGoal } from "../../api/generated/goals/goals";
import {
  useGetEpic,
  getEpic,
  createEpic,
  updateEpic,
  getGetEpicQueryKey,
  getListEpicsQueryKey,
} from "../../api/generated/epics/epics";
import type { Epic } from "../../api/generated/model/epic";
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
import styles from "./EpicFormScreen.module.css";

export function EpicFormScreen() {
  const { t } = useTranslation();
  const { projectId, goalId, epicId } = useParams<{
    projectId: string;
    goalId: string;
    epicId: string;
  }>();
  const isEdit = Boolean(epicId);

  const project = useGetProject(projectId!);
  const epic = useGetEpic(projectId!, epicId!, { query: { enabled: isEdit } });
  // In edit mode, get goal_id from the loaded epic; in create mode, from the URL
  const resolvedGoalId = isEdit ? epic.data?.goal_id : goalId;
  const goal = useGetGoal(projectId!, resolvedGoalId!, {
    query: { enabled: Boolean(resolvedGoalId) },
  });

  if (project.isPending || (isEdit && epic.isPending) || (!resolvedGoalId && isEdit)) {
    return <LoadingState />;
  }
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (isEdit && epic.isError) {
    return <ErrorState error={epic.error} onRetry={() => void epic.refetch()} />;
  }
  if (goal.isPending) return <LoadingState />;
  if (goal.isError) {
    return <ErrorState error={goal.error} onRetry={() => void goal.refetch()} />;
  }

  const breadcrumbItems: BreadcrumbItem[] = isEdit
    ? [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: goal.data.title, to: `/projects/${projectId}/goals/${resolvedGoalId}` },
        { label: epic.data!.title, to: `/projects/${projectId}/epics/${epicId}` },
        { label: t("epic.form.editTitle") },
      ]
    : [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: goal.data.title, to: `/projects/${projectId}/goals/${goalId}` },
        { label: t("epic.form.newTitle") },
      ];

  return (
    <EpicForm
      projectId={projectId!}
      goalId={resolvedGoalId!}
      epicId={epicId}
      isEdit={isEdit}
      breadcrumbItems={breadcrumbItems}
      initialTitle={isEdit ? epic.data!.title : ""}
      initialDescription={isEdit ? epic.data!.description : ""}
      initialRevision={isEdit ? epic.data!.revision : 0}
      key={`${projectId}:${epicId ?? "new"}`}
    />
  );
}

function EpicForm({
  projectId,
  goalId,
  epicId,
  isEdit,
  breadcrumbItems,
  initialTitle,
  initialDescription,
  initialRevision,
}: {
  projectId: string;
  goalId: string;
  epicId?: string;
  isEdit: boolean;
  breadcrumbItems: BreadcrumbItem[];
  initialTitle: string;
  initialDescription: string;
  initialRevision: number;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const form = useEntityForm<Epic>({
    entityId: epicId,
    initialTitle,
    initialDescription,
    initialRevision,
    createFn: (data, opts) =>
      createEpic(projectId, data as { goal_id: string; title: string; description?: string }, opts),
    onCreateSuccess: (data) => {
      void navigate(`/projects/${projectId}/epics/${data.id}`, { replace: true });
    },
    updateFn: (data, opts) =>
      updateEpic(projectId, epicId!, data as { title?: string; description?: string }, opts),
    refetchFn: () => getEpic(projectId, epicId!),
    onUpdateSuccess: () => {
      void navigate(`/projects/${projectId}/epics/${epicId}`, { replace: true });
    },
    getTitle: (e) => e.title,
    getDescription: (e) => e.description,
    getRevision: (e) => e.revision,
    listQueryKey: getListEpicsQueryKey(projectId),
    detailQueryKey: getGetEpicQueryKey(projectId, epicId!),
    createBackPath: `/projects/${projectId}/goals/${goalId}`,
    editBackPath: `/projects/${projectId}/epics/${epicId}`,
    conflictInspectKey: "epic.form.conflictPreviousSubmission",
    conflictInspectPath: `/projects/${projectId}/goals/${goalId}`,
  });

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (isEdit) {
      form.submitUpdate({ title: form.title.trim(), description: form.description.trim() });
    } else {
      form.submitCreate({
        goal_id: goalId,
        title: form.title.trim(),
        description: form.description.trim() || undefined,
      });
    }
  };

  if (form.conflict) {
    return (
      <>
        <Breadcrumbs items={breadcrumbItems} />
        <PageHeader title={t("epic.conflict.title")} description={t("epic.conflict.description")} />
        <ConflictView
          fields={[
            {
              label: t("epic.form.titleLabel"),
              local: form.conflict.localTitle,
              remote: form.conflict.remoteTitle,
            },
            {
              label: t("epic.form.descriptionLabel"),
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
      <PageHeader title={isEdit ? t("epic.form.editTitle") : t("epic.form.newTitle")} />

      <form className={styles.form} onSubmit={handleSubmit} noValidate>
        <FormField label={t("epic.form.titleLabel")} error={form.errors.title}>
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
        <FormField label={t("epic.form.descriptionLabel")} error={form.errors.description}>
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
        {form.errors._form ? <p className={styles.formError}>{form.errors._form}</p> : null}
        {form.createConflict && form.conflictInspectPath ? (
          <p className={styles.formLink}>
            <LinkButton to={form.conflictInspectPath}>{t("epic.form.inspectEpics")}</LinkButton>
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
            disabled={!form.title.trim()}
          >
            {isEdit ? t("epic.form.updateSubmit") : t("epic.form.createSubmit")}
          </Button>
        </div>
      </form>

      <ConfirmDialog
        open={form.confirmDiscard}
        title={t("epic.form.discardTitle")}
        confirmLabel={t("epic.form.discardConfirm")}
        onConfirm={form.navigateBack}
        onCancel={() => form.setConfirmDiscard(false)}
      >
        <p>{t("epic.form.discardDescription")}</p>
      </ConfirmDialog>
    </>
  );
}
