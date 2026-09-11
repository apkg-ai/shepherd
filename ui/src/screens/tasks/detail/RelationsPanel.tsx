import { useQueries, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { Link } from "react-router";
import type { Relation, RelationCreateType } from "../../../api/generated/model";
import {
  useCreateTaskRelation,
  useDeleteTaskRelation,
  useListTaskRelations,
} from "../../../api/generated/relations/relations";
import { getGetTaskQueryOptions, useListTasks } from "../../../api/generated/tasks/tasks";
import { invalidatePaths } from "../../../api/invalidate";
import { isShepherdError } from "../../../api/problem";
import { Button } from "../../../components/Button";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import { Dialog } from "../../../components/Dialog";
import { FormField } from "../../../components/FormField";
import { ErrorState, LoadingState } from "../../../components/states";
import { useToast } from "../../../components/Toast";
import styles from "./RelationsPanel.module.css";

/**
 * Relation semantics (docs/02-domain-model.md): decomposition source is the
 * parent; depends_on source is the task that depends on the target.
 */
type RelationKind = "depends_on" | "subtask" | "parent";

export function RelationsPanel({ projectId, taskId }: { projectId: string; taskId: string }) {
  const queryClient = useQueryClient();
  const { toast } = useToast();
  const relationsQuery = useListTaskRelations(projectId, taskId);
  const [addOpen, setAddOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<Relation | null>(null);

  const relations = relationsQuery.data?.items ?? [];
  const parent = relations.find((r) => r.type === "decomposition" && r.target_task_id === taskId);
  const subtasks = relations.filter(
    (r) => r.type === "decomposition" && r.source_task_id === taskId,
  );
  const dependsOn = relations.filter((r) => r.type === "depends_on" && r.source_task_id === taskId);
  const dependedBy = relations.filter(
    (r) => r.type === "depends_on" && r.target_task_id === taskId,
  );

  // Resolve related task titles through the query cache (deduped per task).
  const otherIds = [
    ...new Set(
      relations.flatMap((r) => [r.source_task_id, r.target_task_id].filter((id) => id !== taskId)),
    ),
  ];
  const titleQueries = useQueries({
    queries: otherIds.map((id) => getGetTaskQueryOptions(projectId, id)),
  });
  const titles = new Map(otherIds.map((id, index) => [id, titleQueries[index]?.data?.title]));

  const deleteRelation = useDeleteTaskRelation();
  const invalidate = () =>
    invalidatePaths(
      queryClient,
      `/api/v1/projects/${projectId}/tasks/${taskId}/relations`,
      `/api/v1/projects/${projectId}/tasks`,
    );

  if (relationsQuery.isPending) return <LoadingState label="Loading relations…" />;
  if (relationsQuery.isError) {
    return (
      <ErrorState error={relationsQuery.error} onRetry={() => void relationsQuery.refetch()} />
    );
  }

  const renderItem = (relation: Relation, otherId: string) => (
    <li key={relation.id} className={styles.item}>
      <Link to={`/projects/${projectId}/tasks/${otherId}`}>{titles.get(otherId) ?? otherId}</Link>
      <button
        type="button"
        className={styles.remove}
        aria-label="Remove relation"
        onClick={() => setPendingDelete(relation)}
      >
        ×
      </button>
    </li>
  );

  return (
    <section className={styles.panel}>
      <header className={styles.header}>
        <h3>Relations</h3>
        <Button variant="ghost" onClick={() => setAddOpen(true)}>
          Add relation
        </Button>
      </header>

      {relations.length === 0 ? (
        <p className={styles.muted}>No relations yet.</p>
      ) : (
        <dl className={styles.groups}>
          {parent ? (
            <>
              <dt>Parent</dt>
              <dd>
                <ul>{renderItem(parent, parent.source_task_id)}</ul>
              </dd>
            </>
          ) : null}
          {subtasks.length > 0 ? (
            <>
              <dt>Subtasks</dt>
              <dd>
                <ul>{subtasks.map((r) => renderItem(r, r.target_task_id))}</ul>
              </dd>
            </>
          ) : null}
          {dependsOn.length > 0 ? (
            <>
              <dt>Depends on</dt>
              <dd>
                <ul>{dependsOn.map((r) => renderItem(r, r.target_task_id))}</ul>
              </dd>
            </>
          ) : null}
          {dependedBy.length > 0 ? (
            <>
              <dt>Depended on by</dt>
              <dd>
                <ul>{dependedBy.map((r) => renderItem(r, r.source_task_id))}</ul>
              </dd>
            </>
          ) : null}
        </dl>
      )}

      <AddRelationDialog
        open={addOpen}
        projectId={projectId}
        taskId={taskId}
        hasParent={parent !== undefined}
        onClose={() => setAddOpen(false)}
        onAdded={invalidate}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Remove relation"
        confirmLabel="Remove"
        danger
        busy={deleteRelation.isPending}
        onConfirm={() => {
          if (!pendingDelete) return;
          deleteRelation.mutate(
            { projectId, taskId, relationId: pendingDelete.id },
            {
              onSuccess: () => {
                invalidate();
                setPendingDelete(null);
                toast("Relation removed.", "success");
              },
            },
          );
        }}
        onCancel={() => setPendingDelete(null)}
      >
        Remove this relation? The tasks themselves are untouched.
      </ConfirmDialog>
    </section>
  );
}

function AddRelationDialog({
  open,
  projectId,
  taskId,
  hasParent,
  onClose,
  onAdded,
}: {
  open: boolean;
  projectId: string;
  taskId: string;
  hasParent: boolean;
  onClose: () => void;
  onAdded: () => void;
}) {
  return (
    <Dialog open={open} title="Add relation" onClose={onClose}>
      {/* Unmounts on close (state resets) and remounts when the option set
          changes, so a stale "parent" kind can't be submitted after the
          "Set parent…" option disappeared. */}
      <AddRelationForm
        key={String(hasParent)}
        projectId={projectId}
        taskId={taskId}
        hasParent={hasParent}
        onClose={onClose}
        onAdded={onAdded}
      />
    </Dialog>
  );
}

function AddRelationForm({
  projectId,
  taskId,
  hasParent,
  onClose,
  onAdded,
}: {
  projectId: string;
  taskId: string;
  hasParent: boolean;
  onClose: () => void;
  onAdded: () => void;
}) {
  const { toast } = useToast();
  const [kind, setKind] = useState<RelationKind>("depends_on");
  const [targetId, setTargetId] = useState("");
  const [error, setError] = useState<string | null>(null);

  // First page of project tasks as the picker source — v1-honest.
  const candidatesQuery = useListTasks(projectId, { limit: 100 });
  const candidates = (candidatesQuery.data?.items ?? []).filter((t) => t.id !== taskId);

  const createRelation = useCreateTaskRelation({
    mutation: { meta: { silent: true } },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (targetId === "") {
      setError("Pick a task first.");
      return;
    }
    setError(null);
    // decomposition source is the parent, so "set parent" posts on the
    // selected task with this task as target; the other kinds post on this
    // task (the path task is always the relation source).
    const type: RelationCreateType = kind === "depends_on" ? "depends_on" : "decomposition";
    const sourceTaskId = kind === "parent" ? targetId : taskId;
    const targetTaskId = kind === "parent" ? taskId : targetId;
    createRelation.mutate(
      {
        projectId,
        taskId: sourceTaskId,
        data: { type, target_task_id: targetTaskId },
      },
      {
        onSuccess: () => {
          onAdded();
          onClose();
          setTargetId("");
          toast("Relation added.", "success");
        },
        onError: (err) => {
          setError(isShepherdError(err) ? err.message : "Request failed");
        },
      },
    );
  };

  return (
    <form onSubmit={submit} noValidate>
      <FormField label="Kind">
        {(props) => (
          <select {...props} value={kind} onChange={(e) => setKind(e.target.value as RelationKind)}>
            <option value="depends_on">This task depends on…</option>
            <option value="subtask">Add subtask…</option>
            {!hasParent ? <option value="parent">Set parent…</option> : null}
          </select>
        )}
      </FormField>
      <FormField label="Task">
        {(props) => (
          <select {...props} value={targetId} onChange={(e) => setTargetId(e.target.value)}>
            <option value="">{candidatesQuery.isPending ? "Loading tasks…" : "Pick a task"}</option>
            {candidates.map((t) => (
              <option key={t.id} value={t.id}>
                {t.title}
              </option>
            ))}
          </select>
        )}
      </FormField>
      {error ? (
        <p className={styles.formError} role="alert">
          {error}
        </p>
      ) : null}
      <div className={styles.dialogActions}>
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" busy={createRelation.isPending}>
          Add relation
        </Button>
      </div>
    </form>
  );
}
