import { type FormEvent } from "react";
import { useNavigate, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useGetProject } from "../../api/generated/projects/projects";
import {
  useGetGoal,
  getGoal,
  createGoal,
  updateGoal,
  getGetGoalQueryKey,
  getListGoalsQueryKey,
} from "../../api/generated/goals/goals";
import type { Goal } from "../../api/generated/model/goal";
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
import styles from "./GoalFormScreen.module.css";

export function GoalFormScreen() {
  const { t } = useTranslation();
  const { projectId, goalId } = useParams<{ projectId: string; goalId: string }>();
  const isEdit = Boolean(goalId);

  const project = useGetProject(projectId!);
  const goal = useGetGoal(projectId!, goalId!, { query: { enabled: isEdit } });

  if (project.isPending || (isEdit && goal.isPending)) return <LoadingState />;
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }
  if (isEdit && goal.isError) {
    return <ErrorState error={goal.error} onRetry={() => void goal.refetch()} />;
  }

  const breadcrumbItems: BreadcrumbItem[] = isEdit
    ? [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: goal.data!.title, to: `/projects/${projectId}/goals/${goalId}` },
        { label: t("goal.form.editTitle") },
      ]
    : [
        { label: t("common.nav.projects"), to: "/" },
        { label: project.data.name, to: `/projects/${projectId}` },
        { label: t("goal.form.newTitle") },
      ];

  return (
    <GoalForm
      projectId={projectId!}
      goalId={goalId}
      isEdit={isEdit}
      breadcrumbItems={breadcrumbItems}
      initialTitle={isEdit ? goal.data!.title : ""}
      initialDescription={isEdit ? goal.data!.description : ""}
      initialRevision={isEdit ? goal.data!.revision : 0}
      key={`${projectId}:${goalId ?? "new"}`}
    />
  );
}

function GoalForm({
  projectId,
  goalId,
  isEdit,
  breadcrumbItems,
  initialTitle,
  initialDescription,
  initialRevision,
}: {
  projectId: string;
  goalId?: string;
  isEdit: boolean;
  breadcrumbItems: BreadcrumbItem[];
  initialTitle: string;
  initialDescription: string;
  initialRevision: number;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();

  const form = useEntityForm<Goal>({
    entityId: goalId,
    initialTitle,
    initialDescription,
    initialRevision,
    createFn: (data, opts) =>
      createGoal(projectId, data as { title: string; description?: string }, opts),
    onCreateSuccess: (data) => {
      void navigate(`/projects/${projectId}/goals/${data.id}`, { replace: true });
    },
    updateFn: (data, opts) =>
      updateGoal(projectId, goalId!, data as { title?: string; description?: string }, opts),
    refetchFn: () => getGoal(projectId, goalId!),
    onUpdateSuccess: () => {
      void navigate(`/projects/${projectId}/goals/${goalId}`, { replace: true });
    },
    getTitle: (g) => g.title,
    getDescription: (g) => g.description,
    getRevision: (g) => g.revision,
    listQueryKey: getListGoalsQueryKey(projectId),
    detailQueryKey: getGetGoalQueryKey(projectId, goalId!),
    createBackPath: `/projects/${projectId}`,
    editBackPath: `/projects/${projectId}/goals/${goalId}`,
    conflictInspectKey: "goal.form.conflictPreviousSubmission",
    conflictInspectPath: `/projects/${projectId}`,
  });

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (isEdit) {
      form.submitUpdate({ title: form.title.trim(), description: form.description.trim() });
    } else {
      form.submitCreate({
        title: form.title.trim(),
        description: form.description.trim() || undefined,
      });
    }
  };

  if (form.conflict) {
    return (
      <>
        <Breadcrumbs items={breadcrumbItems} />
        <PageHeader title={t("goal.conflict.title")} description={t("goal.conflict.description")} />
        <ConflictView
          fields={[
            {
              label: t("goal.form.titleLabel"),
              local: form.conflict.localTitle,
              remote: form.conflict.remoteTitle,
            },
            {
              label: t("goal.form.descriptionLabel"),
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
      <PageHeader title={isEdit ? t("goal.form.editTitle") : t("goal.form.newTitle")} />

      <form className={styles.form} onSubmit={handleSubmit} noValidate>
        <FormField label={t("goal.form.titleLabel")} error={form.errors.title}>
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
        <FormField label={t("goal.form.descriptionLabel")} error={form.errors.description}>
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
            <LinkButton to={form.conflictInspectPath}>{t("goal.form.inspectGoals")}</LinkButton>
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
            {isEdit ? t("goal.form.updateSubmit") : t("goal.form.createSubmit")}
          </Button>
        </div>
      </form>

      <ConfirmDialog
        open={form.confirmDiscard}
        title={t("goal.form.discardTitle")}
        confirmLabel={t("goal.form.discardConfirm")}
        onConfirm={form.navigateBack}
        onCancel={() => form.setConfirmDiscard(false)}
      >
        <p>{t("goal.form.discardDescription")}</p>
      </ConfirmDialog>
    </>
  );
}
