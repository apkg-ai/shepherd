import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useNavigate, useParams } from "react-router";
import type { Task, TaskCreate, TaskCreateType, TaskUpdate } from "../../api/generated/model";
import { useCreateTask, useGetTask, useUpdateTask } from "../../api/generated/tasks/tasks";
import { CreateTaskBody, UpdateTaskBody } from "../../api/generated/zod/tasks/tasks.zod";
import { invalidatePaths } from "../../api/invalidate";
import { fieldErrors, isShepherdError } from "../../api/problem";
import { Button } from "../../components/Button";
import { FormField } from "../../components/FormField";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { PageHeader } from "../../components/PageHeader";
import { zodFieldErrors } from "../../lib/forms";
import { MetadataEditor, validateMetadata } from "./MetadataEditor";
import styles from "./TaskFormScreen.module.css";

const TYPES: TaskCreateType[] = ["code", "question", "refactor", "review", "research"];

export function TaskFormScreen() {
  const { projectId, taskId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  // Remount the form per task so edit state never bleeds between routes.
  return <TaskForm key={taskId ?? "new"} projectId={projectId} taskId={taskId} />;
}

function TaskForm({ projectId, taskId }: { projectId: string; taskId: string | undefined }) {
  const isEdit = taskId !== undefined;
  const existingQuery = useGetTask(projectId, taskId ?? "", {
    query: { enabled: isEdit },
  });

  // Edit mode mounts the inner form only once the task is loaded, so field
  // state initializes from data directly — no effect, and a background
  // refetch never clobbers in-progress edits.
  if (isEdit && existingQuery.isPending) {
    return <LoadingState label="Loading task…" />;
  }
  if (isEdit && existingQuery.isError) {
    return <ErrorState error={existingQuery.error} onRetry={() => void existingQuery.refetch()} />;
  }
  return (
    <TaskFormFields projectId={projectId} existing={isEdit ? existingQuery.data : undefined} />
  );
}

function TaskFormFields({
  projectId,
  existing,
}: {
  projectId: string;
  existing: Task | undefined;
}) {
  const isEdit = existing !== undefined;
  const taskId = existing?.id;
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [title, setTitle] = useState(existing?.title ?? "");
  const [description, setDescription] = useState(existing?.description ?? "");
  const [type, setType] = useState<TaskCreateType>(existing?.type ?? "code");
  const [asProposal, setAsProposal] = useState(false);
  const [metadataText, setMetadataText] = useState(() =>
    existing && Object.keys(existing.metadata).length > 0
      ? JSON.stringify(existing.metadata, null, 2)
      : "",
  );
  const [errors, setErrors] = useState<Record<string, string>>({});

  const createTask = useCreateTask({ mutation: { meta: { silent: true } } });
  const updateTask = useUpdateTask({ mutation: { meta: { silent: true } } });

  const onProblem = (error: unknown) => {
    if (isShepherdError(error) && error.status === 422) {
      setErrors(fieldErrors(error));
    } else {
      setErrors({
        _form: isShepherdError(error) ? error.message : "Request failed",
      });
    }
  };

  const afterWrite = (id: string) => {
    invalidatePaths(
      queryClient,
      `/api/v1/projects/${projectId}/tasks`,
      `/api/v1/projects/${projectId}/tasks/${id}`,
    );
    void navigate(`/projects/${projectId}/tasks/${id}`);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const metadata = validateMetadata(metadataText);
    if (!metadata.ok) {
      setErrors({ metadata: metadata.error });
      return;
    }

    if (existing) {
      // PATCH semantics: send only what changed (status never travels here —
      // transitions go through the action endpoints).
      const patch: TaskUpdate = {};
      if (title !== existing.title) patch.title = title;
      if (description !== existing.description) patch.description = description;
      if (type !== existing.type) patch.type = type;
      if (JSON.stringify(metadata.value) !== JSON.stringify(existing.metadata)) {
        patch.metadata = metadata.value;
      }
      if (Object.keys(patch).length === 0) {
        void navigate(`/projects/${projectId}/tasks/${existing.id}`);
        return;
      }
      const parsed = UpdateTaskBody.safeParse(patch);
      if (!parsed.success) {
        setErrors(zodFieldErrors(parsed.error));
        return;
      }
      setErrors({});
      updateTask.mutate(
        { projectId, taskId: existing.id, data: parsed.data },
        {
          onSuccess: (updated) => afterWrite(updated.id),
          onError: onProblem,
        },
      );
      return;
    }

    const body: TaskCreate = {
      title,
      description,
      type,
      // Humans author work directly; proposals are the agent-suggestion lane
      // (docs/agent-guide.md) — offered here as an explicit opt-in.
      status: asProposal ? "proposed" : "approved",
      metadata: metadata.value,
    };
    const parsed = CreateTaskBody.safeParse(body);
    if (!parsed.success) {
      setErrors(zodFieldErrors(parsed.error));
      return;
    }
    setErrors({});
    createTask.mutate(
      { projectId, data: parsed.data },
      {
        onSuccess: (created) => afterWrite(created.id),
        onError: onProblem,
      },
    );
  };

  return (
    <section className={styles.wrap}>
      <PageHeader title={isEdit ? "Edit task" : "New task"} />
      <form onSubmit={submit} noValidate>
        <FormField label="Title" error={errors["title"]}>
          {(props) => (
            <input
              {...props}
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
          )}
        </FormField>
        <FormField label="Description" error={errors["description"]}>
          {(props) => (
            <textarea
              {...props}
              rows={5}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          )}
        </FormField>
        <FormField label="Type" error={errors["type"]}>
          {(props) => (
            <select
              {...props}
              value={type}
              onChange={(e) => setType(e.target.value as TaskCreateType)}
            >
              {TYPES.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          )}
        </FormField>

        {!isEdit ? (
          <fieldset className={styles.statusChoice}>
            <legend>Initial status</legend>
            <label>
              <input
                type="radio"
                name="initial-status"
                checked={!asProposal}
                onChange={() => setAsProposal(false)}
              />
              Approved — ready to work once dependencies are done
            </label>
            <label>
              <input
                type="radio"
                name="initial-status"
                checked={asProposal}
                onChange={() => setAsProposal(true)}
              />
              Proposed — needs human approval first
            </label>
          </fieldset>
        ) : null}

        <MetadataEditor
          value={metadataText}
          onChange={setMetadataText}
          error={errors["metadata"]}
        />

        {errors["_form"] ? (
          <p className={styles.formError} role="alert">
            {errors["_form"]}
          </p>
        ) : null}

        <div className={styles.actions}>
          <Button
            onClick={() =>
              void navigate(
                isEdit ? `/projects/${projectId}/tasks/${taskId}` : `/projects/${projectId}`,
              )
            }
          >
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={createTask.isPending || updateTask.isPending}
          >
            {isEdit ? "Save changes" : "Create task"}
          </Button>
        </div>
      </form>
    </section>
  );
}
