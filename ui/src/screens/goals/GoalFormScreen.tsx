import { type FormEvent, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import { isShepherdError, fieldErrors } from "../../api/problem";
import { newIdempotencyKey } from "../../lib/idempotency";
import { Breadcrumbs, type BreadcrumbItem } from "../../components/Breadcrumbs";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { FormField } from "../../components/FormField";
import inputStyles from "../../components/Input.module.css";
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
      key={goalId ?? "new"}
    />
  );
}

type ConflictState = {
  localTitle: string;
  localDescription: string;
  remoteTitle: string;
  remoteDescription: string;
  remoteRevision: number;
};

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
  const queryClient = useQueryClient();

  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState(initialDescription);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [idempotencyKey, setIdempotencyKey] = useState(newIdempotencyKey);
  const [revision, setRevision] = useState(initialRevision);
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState<ConflictState | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  const create = useMutation<Goal, unknown, { title: string; description?: string }>({
    mutationFn: (vars) =>
      createGoal(projectId, vars, {
        headers: { "Idempotency-Key": idempotencyKey },
      }),
    meta: { silent: true },
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: getListGoalsQueryKey(projectId) });
      void navigate(`/projects/${projectId}/goals/${data.id}`, { replace: true });
    },
    onError: (error) => {
      setIdempotencyKey(newIdempotencyKey());
      if (isShepherdError(error) && error.status === 422) {
        setErrors(fieldErrors(error));
      } else {
        setErrors({ _form: isShepherdError(error) ? error.title : t("common.error.unexpected") });
      }
    },
  });

  const update = useMutation<Goal, unknown, { title: string; description: string; rev: number }>({
    mutationFn: (vars) =>
      updateGoal(
        projectId,
        goalId!,
        { title: vars.title, description: vars.description },
        {
          headers: {
            "If-Match": `"${vars.rev}"`,
            "Idempotency-Key": idempotencyKey,
          },
        },
      ),
    meta: { silent: true },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: getGetGoalQueryKey(projectId, goalId!) });
      void queryClient.invalidateQueries({ queryKey: getListGoalsQueryKey(projectId) });
      void navigate(`/projects/${projectId}/goals/${goalId}`, { replace: true });
    },
    onError: async (error) => {
      setIdempotencyKey(newIdempotencyKey());
      if (isShepherdError(error) && error.status === 412) {
        try {
          // Direct call: a failed refetch must not poison the mounted goal query.
          const fresh = await getGoal(projectId, goalId!);
          setConflict({
            localTitle: title,
            localDescription: description,
            remoteTitle: fresh.title,
            remoteDescription: fresh.description,
            remoteRevision: fresh.revision,
          });
        } catch {
          setErrors({ _form: t("common.error.unexpected") });
        }
      } else if (isShepherdError(error) && error.status === 422) {
        setErrors(fieldErrors(error));
      } else {
        setErrors({ _form: isShepherdError(error) ? error.title : t("common.error.unexpected") });
      }
    },
  });

  const isPending = create.isPending || update.isPending;

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    setErrors({});
    if (isEdit) {
      update.mutate({ title: title.trim(), description: description.trim(), rev: revision });
    } else {
      create.mutate({ title: title.trim(), description: description.trim() || undefined });
    }
  };

  const handleConflictRetry = () => {
    if (!conflict) return;
    setRevision(conflict.remoteRevision);
    setConflict(null);
  };

  const handleConflictDiscard = () => {
    setConflict(null);
    void navigate(`/projects/${projectId}/goals/${goalId}`, { replace: true });
  };

  const handleCancel = () => {
    if (dirty) {
      setConfirmDiscard(true);
    } else {
      navigateBack();
    }
  };

  const navigateBack = () => {
    if (isEdit) {
      void navigate(`/projects/${projectId}/goals/${goalId}`);
    } else {
      void navigate(`/projects/${projectId}`);
    }
  };

  if (conflict) {
    return (
      <>
        <Breadcrumbs items={breadcrumbItems} />
        <PageHeader title={t("goal.conflict.title")} description={t("goal.conflict.description")} />
        <div className={styles.conflictGrid}>
          <div className={styles.conflictPane}>
            <h3 className={styles.conflictLabel}>{t("goal.conflict.yours")}</h3>
            <dl className={styles.conflictFields}>
              <dt>{t("goal.form.titleLabel")}</dt>
              <dd>{conflict.localTitle}</dd>
              <dt>{t("goal.form.descriptionLabel")}</dt>
              <dd>{conflict.localDescription || t("goal.conflict.none")}</dd>
            </dl>
          </div>
          <div className={styles.conflictPane}>
            <h3 className={styles.conflictLabel}>{t("goal.conflict.theirs")}</h3>
            <dl className={styles.conflictFields}>
              <dt>{t("goal.form.titleLabel")}</dt>
              <dd>{conflict.remoteTitle}</dd>
              <dt>{t("goal.form.descriptionLabel")}</dt>
              <dd>{conflict.remoteDescription || t("goal.conflict.none")}</dd>
            </dl>
          </div>
        </div>
        <div className={styles.conflictActions}>
          <Button onClick={handleConflictDiscard}>{t("goal.conflict.discard")}</Button>
          <Button variant="primary" onClick={handleConflictRetry}>
            {t("goal.conflict.retry")}
          </Button>
        </div>
      </>
    );
  }

  return (
    <>
      <Breadcrumbs items={breadcrumbItems} />
      <PageHeader title={isEdit ? t("goal.form.editTitle") : t("goal.form.newTitle")} />

      <form className={styles.form} onSubmit={handleSubmit} noValidate>
        <FormField label={t("goal.form.titleLabel")} error={errors.title}>
          {(props) => (
            <input
              {...props}
              type="text"
              className={inputStyles.input}
              value={title}
              onChange={(e) => {
                setTitle(e.target.value);
                setDirty(true);
              }}
              required
            />
          )}
        </FormField>
        <FormField label={t("goal.form.descriptionLabel")} error={errors.description}>
          {(props) => (
            <textarea
              {...props}
              className={inputStyles.textarea}
              value={description}
              onChange={(e) => {
                setDescription(e.target.value);
                setDirty(true);
              }}
              rows={6}
            />
          )}
        </FormField>
        {errors._form ? <p className={styles.formError}>{errors._form}</p> : null}
        <div className={styles.actions}>
          <Button type="button" onClick={handleCancel}>
            {t("common.action.cancel")}
          </Button>
          <Button type="submit" variant="primary" busy={isPending} disabled={!title.trim()}>
            {isEdit ? t("goal.form.updateSubmit") : t("goal.form.createSubmit")}
          </Button>
        </div>
      </form>

      <ConfirmDialog
        open={confirmDiscard}
        title={t("goal.form.discardTitle")}
        confirmLabel={t("goal.form.discardConfirm")}
        onConfirm={navigateBack}
        onCancel={() => setConfirmDiscard(false)}
      >
        <p>{t("goal.form.discardDescription")}</p>
      </ConfirmDialog>
    </>
  );
}
