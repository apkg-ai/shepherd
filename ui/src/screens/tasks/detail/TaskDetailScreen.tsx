import { useParams } from "react-router";
import { useListTaskSessionsInfinite } from "../../../api/generated/sessions/sessions";
import { useGetTask } from "../../../api/generated/tasks/tasks";
import { cursorPaging, flattenPages } from "../../../api/paging";
import { AttemptBadge } from "../../../components/AttemptBadge";
import { IdentityChip } from "../../../components/IdentityChip";
import { LoadMore } from "../../../components/LoadMore";
import { StatusBadge } from "../../../components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "../../../components/states";
import { formatDateTime } from "../../../lib/format";
import { RelationsPanel } from "./RelationsPanel";
import { SessionsTimeline } from "./SessionsTimeline";
import { TaskActions } from "./TaskActions";
import { TaskKnowledgePanel } from "./TaskKnowledgePanel";
import styles from "./TaskDetailScreen.module.css";

export function TaskDetailScreen() {
  const { projectId, taskId } = useParams();
  if (!projectId || !taskId) return <EmptyState title="No task selected" />;
  return <TaskDetail key={taskId} projectId={projectId} taskId={taskId} />;
}

function TaskDetail({ projectId, taskId }: { projectId: string; taskId: string }) {
  const taskQuery = useGetTask(projectId, taskId);
  const sessionsQuery = useListTaskSessionsInfinite(projectId, taskId, undefined, {
    query: cursorPaging(),
  });

  if (taskQuery.isPending) return <LoadingState label="Loading task…" />;
  if (taskQuery.isError) {
    return <ErrorState error={taskQuery.error} onRetry={() => void taskQuery.refetch()} />;
  }

  const task = taskQuery.data;
  const sessions = flattenPages(sessionsQuery.data?.pages);
  const failedAttempts = sessions.filter((s) => s.outcome === "failed").length;
  const metadataEntries = Object.keys(task.metadata).length;

  return (
    <article className={styles.detail}>
      <header className={styles.header}>
        <div className={styles.titleRow}>
          <StatusBadge status={task.status} />
          <h2 className={styles.title}>{task.title}</h2>
        </div>
        <div className={styles.meta}>
          <span className={styles.metaItem}>{task.type}</span>
          {task.graph_role.map((role) => (
            <span key={role} className={styles.metaItem}>
              {role}
            </span>
          ))}
          {task.assignee ? <IdentityChip identity={task.assignee} /> : null}
          <AttemptBadge
            attempts={task.attempt_count}
            failures={failedAttempts}
            // Failures are counted from loaded session pages only — say "≥"
            // rather than understate while more pages exist.
            atLeast={sessionsQuery.hasNextPage === true}
          />
          <span className={styles.muted}>
            Created {formatDateTime(task.created_at)} · updated {formatDateTime(task.updated_at)}
          </span>
        </div>
        <TaskActions projectId={projectId} task={task} />
      </header>

      <section className={styles.section}>
        <h3>Description</h3>
        {task.description ? (
          <p className={styles.description}>{task.description}</p>
        ) : (
          <p className={styles.muted}>No description.</p>
        )}
      </section>

      {metadataEntries > 0 ? (
        <section className={styles.section}>
          <h3>Metadata</h3>
          <pre className={styles.metadata}>{JSON.stringify(task.metadata, null, 2)}</pre>
        </section>
      ) : null}

      <RelationsPanel projectId={projectId} taskId={taskId} />

      <section className={styles.section}>
        <h3>Sessions</h3>
        {sessionsQuery.isPending ? (
          <LoadingState label="Loading sessions…" />
        ) : sessionsQuery.isError ? (
          <ErrorState error={sessionsQuery.error} onRetry={() => void sessionsQuery.refetch()} />
        ) : (
          <>
            <SessionsTimeline sessions={sessions} />
            <LoadMore
              hasNextPage={sessionsQuery.hasNextPage}
              isFetchingNextPage={sessionsQuery.isFetchingNextPage}
              onLoadMore={() => void sessionsQuery.fetchNextPage()}
            />
          </>
        )}
      </section>

      <TaskKnowledgePanel projectId={projectId} taskId={taskId} />
    </article>
  );
}
