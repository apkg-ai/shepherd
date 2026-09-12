import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import type { KnowledgeItem, KnowledgeItemCreateType } from "../../../api/generated/model";
import {
  useCreateKnowledge,
  useDeleteKnowledge,
  useListKnowledgeInfinite,
} from "../../../api/generated/knowledge/knowledge";
import { CreateKnowledgeBody } from "../../../api/generated/zod/knowledge/knowledge.zod";
import { invalidatePaths } from "../../../api/invalidate";
import { cursorPaging, flattenPages } from "../../../api/paging";
import { fieldErrors, isShepherdError } from "../../../api/problem";
import { Button } from "../../../components/Button";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import { Dialog } from "../../../components/Dialog";
import { FormField } from "../../../components/FormField";
import { LoadMore } from "../../../components/LoadMore";
import { ErrorState, LoadingState } from "../../../components/states";
import { useToast } from "../../../components/Toast";
import { zodFieldErrors } from "../../../lib/forms";
import { isHttpUrl } from "../../../lib/links";
import styles from "./TaskKnowledgePanel.module.css";

const KNOWLEDGE_TYPES: KnowledgeItemCreateType[] = ["note", "link", "decision", "transcript"];

/** Task-scoped knowledge (the project-wide knowledge screen is S8). */
export function TaskKnowledgePanel({ projectId, taskId }: { projectId: string; taskId: string }) {
  const queryClient = useQueryClient();
  const { toast } = useToast();
  const [addOpen, setAddOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<KnowledgeItem | null>(null);

  const query = useListKnowledgeInfinite(projectId, { task_id: taskId }, { query: cursorPaging() });
  const items = flattenPages(query.data?.pages);

  const deleteKnowledge = useDeleteKnowledge();
  const invalidate = () => invalidatePaths(queryClient, `/api/v1/projects/${projectId}/knowledge`);

  return (
    <section className={styles.panel}>
      <header className={styles.header}>
        <h3>Knowledge</h3>
        <Button variant="ghost" onClick={() => setAddOpen(true)}>
          Add knowledge
        </Button>
      </header>

      {query.isPending ? (
        <LoadingState label="Loading knowledge…" />
      ) : query.isError ? (
        <ErrorState error={query.error} onRetry={() => void query.refetch()} />
      ) : items.length === 0 ? (
        <p className={styles.muted}>Nothing recorded for this task yet.</p>
      ) : (
        <>
          <ul className={styles.list}>
            {items.map((item) => (
              <li key={item.id} className={styles.item}>
                <div className={styles.itemHeader}>
                  <span className={styles.type}>{item.type}</span>
                  <strong>{item.title}</strong>
                  <button
                    type="button"
                    className={styles.remove}
                    aria-label={`Delete ${item.title}`}
                    onClick={() => setPendingDelete(item)}
                  >
                    ×
                  </button>
                </div>
                {item.type === "link" && isHttpUrl(item.content) ? (
                  <a href={item.content} target="_blank" rel="noreferrer">
                    {item.content}
                  </a>
                ) : (
                  <p className={styles.content}>{item.content}</p>
                )}
              </li>
            ))}
          </ul>
          <LoadMore
            hasNextPage={query.hasNextPage}
            isFetchingNextPage={query.isFetchingNextPage}
            onLoadMore={() => void query.fetchNextPage()}
          />
        </>
      )}

      <AddKnowledgeDialog
        open={addOpen}
        projectId={projectId}
        taskId={taskId}
        onClose={() => setAddOpen(false)}
        onAdded={invalidate}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Delete knowledge item"
        confirmLabel="Delete"
        danger
        busy={deleteKnowledge.isPending}
        onConfirm={() => {
          if (!pendingDelete) return;
          deleteKnowledge.mutate(
            { projectId, knowledgeId: pendingDelete.id },
            {
              onSuccess: () => {
                invalidate();
                setPendingDelete(null);
                toast("Knowledge item deleted.", "success");
              },
            },
          );
        }}
        onCancel={() => setPendingDelete(null)}
      >
        Delete “{pendingDelete?.title}”?
      </ConfirmDialog>
    </section>
  );
}

function AddKnowledgeDialog({
  open,
  projectId,
  taskId,
  onClose,
  onAdded,
}: {
  open: boolean;
  projectId: string;
  taskId: string;
  onClose: () => void;
  onAdded: () => void;
}) {
  const { toast } = useToast();
  const [type, setType] = useState<KnowledgeItemCreateType>("note");
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [errors, setErrors] = useState<Record<string, string>>({});

  const createKnowledge = useCreateKnowledge({
    mutation: { meta: { silent: true } },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const parsed = CreateKnowledgeBody.safeParse({
      type,
      title,
      content,
      scope: "task",
      task_id: taskId,
    });
    if (!parsed.success) {
      setErrors(zodFieldErrors(parsed.error));
      return;
    }
    setErrors({});
    createKnowledge.mutate(
      { projectId, data: parsed.data },
      {
        onSuccess: () => {
          onAdded();
          onClose();
          setTitle("");
          setContent("");
          toast("Knowledge recorded.", "success");
        },
        onError: (error) => {
          if (isShepherdError(error) && error.status === 422) {
            setErrors(fieldErrors(error));
          } else {
            setErrors({
              _form: isShepherdError(error) ? error.message : "Request failed",
            });
          }
        },
      },
    );
  };

  return (
    <Dialog open={open} title="Add knowledge" onClose={onClose}>
      <form onSubmit={submit} noValidate>
        <FormField label="Type" error={errors["type"]}>
          {(props) => (
            <select
              {...props}
              value={type}
              onChange={(e) => setType(e.target.value as KnowledgeItemCreateType)}
            >
              {KNOWLEDGE_TYPES.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          )}
        </FormField>
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
        <FormField label={type === "link" ? "URL" : "Content"} error={errors["content"]}>
          {(props) => (
            <textarea
              {...props}
              rows={4}
              value={content}
              onChange={(e) => setContent(e.target.value)}
            />
          )}
        </FormField>
        {errors["_form"] ? (
          <p className={styles.formError} role="alert">
            {errors["_form"]}
          </p>
        ) : null}
        <div className={styles.dialogActions}>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" busy={createKnowledge.isPending}>
            Add knowledge
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
