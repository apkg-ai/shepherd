import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate } from "react-router";
import type { Task } from "../../../api/generated/model";
import {
  useApproveTask,
  useBlockTask,
  useCancelTask,
  useDeleteTask,
  useRejectTask,
  useUnblockTask,
} from "../../../api/generated/tasks/tasks";
import { BlockTaskBody, RejectTaskBody } from "../../../api/generated/zod/tasks/tasks.zod";
import { invalidatePaths } from "../../../api/invalidate";
import { Button } from "../../../components/Button";
import { ConfirmDialog } from "../../../components/ConfirmDialog";
import { ReasonDialog } from "../../../components/ReasonDialog";
import { useToast } from "../../../components/Toast";
import { zodFieldErrors } from "../../../lib/forms";
import styles from "./TaskActions.module.css";

const TERMINAL = new Set(["done", "cancelled"]);

export function validateReason(
  schema: typeof RejectTaskBody | typeof BlockTaskBody,
): (reason: string) => string | null {
  return (reason) => {
    const parsed = schema.safeParse({ reason });
    if (parsed.success) return null;
    return zodFieldErrors(parsed.error)["reason"] ?? "Invalid reason";
  };
}

export function TaskActions({ projectId, task }: { projectId: string; task: Task }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { toast } = useToast();
  const [dialog, setDialog] = useState<"block" | "reject" | "cancel" | "delete" | null>(null);

  const refresh = () => {
    invalidatePaths(
      queryClient,
      `/api/v1/projects/${projectId}/tasks`,
      `/api/v1/projects/${projectId}/tasks/${task.id}`,
    );
  };
  const done = (message: string) => {
    refresh();
    setDialog(null);
    toast(message, "success");
  };

  const approve = useApproveTask();
  const reject = useRejectTask();
  const block = useBlockTask();
  const unblock = useUnblockTask();
  const cancel = useCancelTask();
  const deleteTask = useDeleteTask();

  const ids = { projectId, taskId: task.id };
  const terminal = TERMINAL.has(task.status);

  return (
    <div className={styles.actions}>
      {task.status === "proposed" || task.status === "in_review" ? (
        <Button
          variant="primary"
          busy={approve.isPending}
          onClick={() =>
            approve.mutate(
              { ...ids, data: {} },
              {
                onSuccess: () =>
                  done(
                    task.status === "proposed"
                      ? "Proposal approved."
                      : "Work approved — task is done.",
                  ),
              },
            )
          }
        >
          Approve
        </Button>
      ) : null}

      {task.status === "in_review" ? (
        <Button variant="danger" onClick={() => setDialog("reject")}>
          Reject
        </Button>
      ) : null}

      {task.status === "blocked" ? (
        <Button
          busy={unblock.isPending}
          onClick={() => unblock.mutate({ ...ids }, { onSuccess: () => done("Task unblocked.") })}
        >
          Unblock
        </Button>
      ) : null}

      {!terminal && task.status !== "blocked" ? (
        <Button onClick={() => setDialog("block")}>Block</Button>
      ) : null}

      {!terminal ? <Button onClick={() => setDialog("cancel")}>Cancel task</Button> : null}

      <Link className={styles.editLink} to={`/projects/${projectId}/tasks/${task.id}/edit`}>
        Edit
      </Link>

      <Button variant="danger" onClick={() => setDialog("delete")}>
        Delete
      </Button>

      <ReasonDialog
        open={dialog === "reject"}
        title="Reject work"
        label="Why is this being rejected? (kept in the attempt history)"
        confirmLabel="Reject"
        danger
        busy={reject.isPending}
        validate={validateReason(RejectTaskBody)}
        onSubmit={(reason) =>
          reject.mutate(
            { ...ids, data: { reason } },
            { onSuccess: () => done("Work rejected — task is claimable again.") },
          )
        }
        onCancel={() => setDialog(null)}
      />

      <ReasonDialog
        open={dialog === "block"}
        title="Block task"
        label="Why is this task blocked?"
        confirmLabel="Block"
        busy={block.isPending}
        validate={validateReason(BlockTaskBody)}
        onSubmit={(reason) =>
          block.mutate({ ...ids, data: { reason } }, { onSuccess: () => done("Task blocked.") })
        }
        onCancel={() => setDialog(null)}
      />

      <ConfirmDialog
        open={dialog === "cancel"}
        title="Cancel task"
        confirmLabel="Cancel task"
        danger
        busy={cancel.isPending}
        onConfirm={() => cancel.mutate({ ...ids }, { onSuccess: () => done("Task cancelled.") })}
        onCancel={() => setDialog(null)}
      >
        Cancelling is terminal — the task cannot be reactivated. Continue?
      </ConfirmDialog>

      <ConfirmDialog
        open={dialog === "delete"}
        title="Delete task"
        confirmLabel="Delete"
        danger
        busy={deleteTask.isPending}
        onConfirm={() =>
          deleteTask.mutate(
            { ...ids },
            {
              onSuccess: () => {
                invalidatePaths(queryClient, `/api/v1/projects/${projectId}/tasks`);
                toast("Task deleted.", "success");
                void navigate(`/projects/${projectId}`);
              },
            },
          )
        }
        onCancel={() => setDialog(null)}
      >
        Delete “{task.title}” and its relations, sessions, and knowledge?
      </ConfirmDialog>
    </div>
  );
}
