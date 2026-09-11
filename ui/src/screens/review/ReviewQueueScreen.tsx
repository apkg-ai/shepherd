import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import type { Task } from "../../api/generated/model";
import {
  useApproveTask,
  useCancelTask,
  useListTasksInfinite,
  useRejectTask,
} from "../../api/generated/tasks/tasks";
import { RejectTaskBody } from "../../api/generated/zod/tasks/tasks.zod";
import { invalidatePaths } from "../../api/invalidate";
import { cursorPaging, flattenPages } from "../../api/paging";
import { AttemptBadge } from "../../components/AttemptBadge";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { IdentityChip } from "../../components/IdentityChip";
import { LoadMore } from "../../components/LoadMore";
import { ReasonDialog } from "../../components/ReasonDialog";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { useToast } from "../../components/Toast";
import { validateReason } from "../tasks/detail/TaskActions";
import styles from "./ReviewQueueScreen.module.css";

type Tab = "in_review" | "proposals";

/**
 * Everything waiting on a human, in one place: work in review
 * (approve → done / reject → ready) and agent proposals
 * (approve → approved / cancel).
 */
export function ReviewQueueScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <ReviewQueue projectId={projectId} />;
}

function ReviewQueue({ projectId }: { projectId: string }) {
  const [searchParams, setSearchParams] = useSearchParams();
  const tab: Tab = searchParams.get("tab") === "proposals" ? "proposals" : "in_review";

  const setTab = (next: Tab) => {
    setSearchParams((params) => {
      params.set("tab", next);
      return params;
    });
  };

  return (
    <section>
      <h2 className={styles.heading}>Review</h2>
      <div role="tablist" className={styles.tabs}>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "in_review"}
          className={tab === "in_review" ? styles.tabActive : styles.tab}
          onClick={() => setTab("in_review")}
        >
          In review
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "proposals"}
          className={tab === "proposals" ? styles.tabActive : styles.tab}
          onClick={() => setTab("proposals")}
        >
          Proposals
        </button>
      </div>
      {tab === "in_review" ? (
        <InReviewTab projectId={projectId} />
      ) : (
        <ProposalsTab projectId={projectId} />
      )}
    </section>
  );
}

function useReviewList(projectId: string, status: "in_review" | "proposed") {
  return useListTasksInfinite(projectId, { status }, { query: cursorPaging() });
}

function useRefreshQueues(projectId: string) {
  const queryClient = useQueryClient();
  return (taskId: string) =>
    invalidatePaths(
      queryClient,
      `/api/v1/projects/${projectId}/tasks`,
      `/api/v1/projects/${projectId}/tasks/${taskId}`,
    );
}

function InReviewTab({ projectId }: { projectId: string }) {
  const { toast } = useToast();
  const query = useReviewList(projectId, "in_review");
  const tasks = flattenPages(query.data?.pages);
  const refresh = useRefreshQueues(projectId);
  const [rejecting, setRejecting] = useState<Task | null>(null);

  const approve = useApproveTask();
  const reject = useRejectTask();

  if (query.isPending) return <LoadingState label="Loading review queue…" />;
  if (query.isError) {
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }
  if (tasks.length === 0) {
    return (
      <EmptyState title="Nothing waiting on you">
        <p>Agent work that needs review will land here.</p>
      </EmptyState>
    );
  }

  return (
    <>
      <ul className={styles.queue}>
        {tasks.map((task) => (
          <li key={task.id} className={styles.row}>
            <div className={styles.rowMain}>
              <Link to={`/projects/${projectId}/tasks/${task.id}`}>{task.title}</Link>
              <span className={styles.rowMeta}>
                <span className={styles.type}>{task.type}</span>
                {task.assignee ? <IdentityChip identity={task.assignee} /> : null}
                <AttemptBadge attempts={task.attempt_count} />
              </span>
            </div>
            <div className={styles.rowActions}>
              <Button
                variant="primary"
                busy={approve.isPending && approve.variables?.taskId === task.id}
                onClick={() =>
                  approve.mutate(
                    { projectId, taskId: task.id, data: {} },
                    {
                      onSuccess: () => {
                        refresh(task.id);
                        toast(`Approved “${task.title}” — done.`, "success");
                      },
                      onError: () => refresh(task.id),
                    },
                  )
                }
              >
                Approve
              </Button>
              <Button variant="danger" onClick={() => setRejecting(task)}>
                Reject
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <LoadMore
        hasNextPage={query.hasNextPage}
        isFetchingNextPage={query.isFetchingNextPage}
        onLoadMore={() => void query.fetchNextPage()}
      />

      <ReasonDialog
        open={rejecting !== null}
        title={`Reject “${rejecting?.title ?? ""}”`}
        label="Why is this being rejected? (kept in the attempt history)"
        confirmLabel="Reject"
        danger
        busy={reject.isPending}
        validate={validateReason(RejectTaskBody)}
        onSubmit={(reason) => {
          if (!rejecting) return;
          reject.mutate(
            { projectId, taskId: rejecting.id, data: { reason } },
            {
              onSuccess: () => {
                refresh(rejecting.id);
                setRejecting(null);
                toast(`Rejected “${rejecting.title}” — back to ready.`, "success");
              },
              onError: () => refresh(rejecting.id),
            },
          );
        }}
        onCancel={() => setRejecting(null)}
      />
    </>
  );
}

function ProposalsTab({ projectId }: { projectId: string }) {
  const { toast } = useToast();
  const query = useReviewList(projectId, "proposed");
  const tasks = flattenPages(query.data?.pages);
  const refresh = useRefreshQueues(projectId);
  const [cancelling, setCancelling] = useState<Task | null>(null);

  const approve = useApproveTask();
  const cancel = useCancelTask();

  if (query.isPending) return <LoadingState label="Loading proposals…" />;
  if (query.isError) {
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }
  if (tasks.length === 0) {
    return (
      <EmptyState title="No proposals waiting">
        <p>Tasks agents propose will show up here for approval.</p>
      </EmptyState>
    );
  }

  return (
    <>
      <ul className={styles.queue}>
        {tasks.map((task) => (
          <li key={task.id} className={styles.row}>
            <div className={styles.rowMain}>
              <Link to={`/projects/${projectId}/tasks/${task.id}`}>{task.title}</Link>
              <span className={styles.rowMeta}>
                <span className={styles.type}>{task.type}</span>
              </span>
            </div>
            <div className={styles.rowActions}>
              <Button
                variant="primary"
                busy={approve.isPending && approve.variables?.taskId === task.id}
                onClick={() =>
                  approve.mutate(
                    { projectId, taskId: task.id, data: {} },
                    {
                      onSuccess: () => {
                        refresh(task.id);
                        toast(`Approved proposal “${task.title}”.`, "success");
                      },
                      onError: () => refresh(task.id),
                    },
                  )
                }
              >
                Approve
              </Button>
              <Button variant="danger" onClick={() => setCancelling(task)}>
                Cancel task
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <LoadMore
        hasNextPage={query.hasNextPage}
        isFetchingNextPage={query.isFetchingNextPage}
        onLoadMore={() => void query.fetchNextPage()}
      />

      <ConfirmDialog
        open={cancelling !== null}
        title="Cancel proposal"
        confirmLabel="Cancel task"
        danger
        busy={cancel.isPending}
        onConfirm={() => {
          if (!cancelling) return;
          cancel.mutate(
            { projectId, taskId: cancelling.id },
            {
              onSuccess: () => {
                refresh(cancelling.id);
                setCancelling(null);
                toast(`Cancelled “${cancelling.title}”.`, "success");
              },
              onError: () => refresh(cancelling.id),
            },
          );
        }}
        onCancel={() => setCancelling(null)}
      >
        Cancelling is terminal — the proposal cannot be reactivated. Continue?
      </ConfirmDialog>
    </>
  );
}
