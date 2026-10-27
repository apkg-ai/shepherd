import { type FormEvent, useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  useListProjects,
  createProject,
  getListProjectsQueryKey,
} from "../../api/generated/projects/projects";
import type { Project } from "../../api/generated/model/project";
import { isShepherdError, fieldErrors } from "../../api/problem";
import { newIdempotencyKey } from "../../lib/idempotency";
import { Button } from "../../components/Button";
import { Card, CardGrid } from "../../components/CardGrid";
import { Dialog } from "../../components/Dialog";
import { FormField } from "../../components/FormField";
import inputStyles from "../../components/Input.module.css";
import { PageHeader } from "../../components/PageHeader";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import styles from "./ProjectListScreen.module.css";

export function ProjectListScreen() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const projects = useListProjects();
  const [dialogOpen, setDialogOpen] = useState(false);

  if (projects.isPending) return <LoadingState />;
  if (projects.isError) {
    return <ErrorState error={projects.error} onRetry={() => void projects.refetch()} />;
  }

  const allItems = projects.data.items;

  return (
    <>
      <PageHeader
        title={t("project.list.title")}
        description={t("project.list.description")}
        actions={
          <Button variant="primary" onClick={() => setDialogOpen(true)}>
            {t("project.list.create")}
          </Button>
        }
      />

      {allItems.length === 0 ? (
        <EmptyState title={t("project.list.emptyTitle")} icon="📁">
          <Button variant="primary" onClick={() => setDialogOpen(true)}>
            {t("project.list.emptyCta")}
          </Button>
        </EmptyState>
      ) : (
        <CardGrid>
          {allItems.map((project) => (
            <Card
              key={project.id}
              to={`/projects/${project.id}`}
              title={project.name}
              description={project.description || undefined}
              meta={
                project.epic_counts.total > 0
                  ? t("project.overview.epicCount", { count: project.epic_counts.total })
                  : undefined
              }
            />
          ))}
        </CardGrid>
      )}

      <CreateProjectDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onCreated={(id) => void navigate(`/projects/${id}`)}
      />
    </>
  );
}

function CreateProjectDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [idempotencyKey, setIdempotencyKey] = useState(newIdempotencyKey);

  const create = useMutation<Project, unknown, { name: string; description?: string }>({
    mutationFn: (vars) =>
      createProject(vars, {
        headers: { "Idempotency-Key": idempotencyKey },
      }),
    meta: { silent: true },
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: getListProjectsQueryKey() });
      resetForm();
      onClose();
      onCreated(data.id);
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

  const resetForm = () => {
    setName("");
    setDescription("");
    setErrors({});
    setIdempotencyKey(newIdempotencyKey());
  };

  const handleClose = () => {
    resetForm();
    onClose();
  };

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    setErrors({});
    create.mutate({ name: name.trim(), description: description.trim() || undefined });
  };

  return (
    <Dialog open={open} title={t("project.form.createTitle")} onClose={handleClose}>
      <form onSubmit={handleSubmit} noValidate>
        <FormField label={t("project.form.nameLabel")} error={errors.name}>
          {(props) => (
            <input
              {...props}
              type="text"
              className={inputStyles.input}
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
            />
          )}
        </FormField>
        <FormField label={t("project.form.descriptionLabel")} error={errors.description}>
          {(props) => (
            <textarea
              {...props}
              className={inputStyles.textarea}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
            />
          )}
        </FormField>
        {errors._form ? <p className={styles.formError}>{errors._form}</p> : null}
        <div className={styles.dialogActions}>
          <Button onClick={handleClose}>{t("common.action.cancel")}</Button>
          <Button type="submit" variant="primary" busy={create.isPending} disabled={!name.trim()}>
            {t("project.form.createSubmit")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
