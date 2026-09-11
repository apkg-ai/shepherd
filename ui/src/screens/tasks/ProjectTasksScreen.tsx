import { Link, useParams, useSearchParams } from "react-router";
import { useListTasksInfinite } from "../../api/generated/tasks/tasks";
import type { StatusFilterParameter, TypeFilterParameter } from "../../api/generated/model";
import { cursorPaging, flattenPages } from "../../api/paging";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { LoadMore } from "../../components/LoadMore";
import { StatusBadge } from "../../components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { formatDateTime } from "../../lib/format";
import styles from "./ProjectTasksScreen.module.css";

const STATUSES: StatusFilterParameter[] = [
  "proposed",
  "approved",
  "ready",
  "in_progress",
  "in_review",
  "done",
  "blocked",
  "cancelled",
];
const TYPES: TypeFilterParameter[] = ["code", "question", "refactor", "review", "research"];

export function ProjectTasksScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <TaskList projectId={projectId} />;
}

function TaskList({ projectId }: { projectId: string }) {
  const [searchParams, setSearchParams] = useSearchParams();

  const rawStatus = searchParams.get("status");
  const rawType = searchParams.get("type");
  const status = STATUSES.find((s) => s === rawStatus);
  const type = TYPES.find((t) => t === rawType);

  const query = useListTasksInfinite(
    projectId,
    {
      ...(status && { status }),
      ...(type && { type }),
    },
    { query: cursorPaging() },
  );
  const tasks = flattenPages(query.data?.pages);

  const setFilter = (key: "status" | "type", value: string) => {
    setSearchParams((params) => {
      if (value === "") params.delete(key);
      else params.set(key, value);
      return params;
    });
  };

  return (
    <section>
      <header className={styles.header}>
        <h2>Tasks</h2>
        <Link to={`/projects/${projectId}/tasks/new`} className={styles.newTask}>
          New task
        </Link>
      </header>

      <div className={styles.filters}>
        <label className={styles.filter}>
          Status
          <select value={status ?? ""} onChange={(e) => setFilter("status", e.target.value)}>
            <option value="">All</option>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s.replace("_", " ")}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.filter}>
          Type
          <select value={type ?? ""} onChange={(e) => setFilter("type", e.target.value)}>
            <option value="">All</option>
            {TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>
      </div>

      {query.isPending ? (
        <LoadingState label="Loading tasks…" />
      ) : query.isError ? (
        <ErrorState error={query.error} onRetry={() => void query.refetch()} />
      ) : tasks.length === 0 ? (
        <EmptyState title={status || type ? "No tasks match these filters" : "No tasks yet"}>
          <Link to={`/projects/${projectId}/tasks/new`}>Create the first task</Link>
        </EmptyState>
      ) : (
        <>
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Status</th>
                <th>Title</th>
                <th>Type</th>
                <th>Assignee</th>
                <th>Attempts</th>
                <th>Updated</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => (
                <tr key={task.id}>
                  <td>
                    <StatusBadge status={task.status} />
                  </td>
                  <td>
                    <Link to={`/projects/${projectId}/tasks/${task.id}`}>{task.title}</Link>
                  </td>
                  <td className={styles.muted}>{task.type}</td>
                  <td>
                    {task.status === "in_progress" && task.assignee ? (
                      <IdentityChip identity={task.assignee} />
                    ) : null}
                  </td>
                  <td>
                    <AttemptBadge attempts={task.attempt_count} />
                  </td>
                  <td className={styles.muted}>{formatDateTime(task.updated_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <LoadMore
            hasNextPage={query.hasNextPage}
            isFetchingNextPage={query.isFetchingNextPage}
            onLoadMore={() => void query.fetchNextPage()}
          />
        </>
      )}
    </section>
  );
}
