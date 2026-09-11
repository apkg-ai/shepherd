import { useQueries } from "@tanstack/react-query";
import { Fragment, useState, type CSSProperties, type ReactNode } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import type {
  StatusFilterParameter,
  TaskStatus,
  TypeFilterParameter,
} from "../../api/generated/model";
import { getListTaskRelationsQueryOptions } from "../../api/generated/relations/relations";
import { getGetTaskQueryOptions, useListTasksInfinite } from "../../api/generated/tasks/tasks";
import { cursorPaging, flattenPages } from "../../api/paging";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { LoadMore } from "../../components/LoadMore";
import { PageHeader } from "../../components/PageHeader";
import { StatusBadge } from "../../components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { formatDateTime } from "../../lib/format";
import { buildTaskTree, describeDependencies, type TaskTreeNode } from "../../lib/taskTree";
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
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const rawStatus = searchParams.get("status");
  const rawType = searchParams.get("type");
  const status = STATUSES.find((s) => s === rawStatus);
  const type = TYPES.find((t) => t === rawType);
  const filtered = status !== undefined || type !== undefined;

  const query = useListTasksInfinite(
    projectId,
    {
      ...(status && { status }),
      ...(type && { type }),
    },
    { query: cursorPaging() },
  );
  const tasks = flattenPages(query.data?.pages);

  // Hierarchy costs one relations fetch per visible task — there is no bulk
  // relations endpoint in v1 (S8 candidate). Bounded by page size and cached,
  // so task-detail visits reuse the same entries.
  const relationQueries = useQueries({
    queries: tasks.map((t) => ({
      ...getListTaskRelationsQueryOptions(projectId, t.id),
      staleTime: 30_000,
    })),
  });
  // Plain per-render computation: page-bounded n keeps this cheap, and it
  // avoids useMemo dependency arrays whose length changes with the page.
  const relationsByTask = new Map(
    tasks.map((t, index) => [t.id, relationQueries[index]?.data?.items]),
  );

  const tree = buildTaskTree(tasks, relationsByTask);

  // Resolve titles/statuses for parents that aren't in the loaded pages.
  const nodesById = new Map<string, TaskTreeNode>();
  const walk = (nodes: TaskTreeNode[]) => {
    for (const node of nodes) {
      nodesById.set(node.task.id, node);
      walk(node.children);
    }
  };
  walk(tree);
  const outOfViewParentIds = [
    ...new Set(
      [...nodesById.values()]
        .filter((n) => n.parentId !== null && !n.parentInView)
        .map((n) => n.parentId!),
    ),
  ];
  const parentQueries = useQueries({
    queries: outOfViewParentIds.map((id) => ({
      ...getGetTaskQueryOptions(projectId, id),
      staleTime: 30_000,
    })),
  });
  const outOfViewParents = new Map(
    outOfViewParentIds.map((id, index) => [id, parentQueries[index]?.data]),
  );

  const statusById = new Map<string, TaskStatus>();
  for (const t of tasks) statusById.set(t.id, t.status);
  for (const [id, t] of outOfViewParents) if (t) statusById.set(id, t.status);

  const setFilter = (key: "status" | "type", value: string) => {
    setSearchParams((params) => {
      if (value === "") params.delete(key);
      else params.set(key, value);
      return params;
    });
  };

  const toggleCollapse = (taskId: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  };

  const parentContext = (node: TaskTreeNode, force = false) => {
    if (node.parentId === null || (node.parentInView && !force)) return null;
    const parent = nodesById.get(node.parentId)?.task ?? outOfViewParents.get(node.parentId);
    return (
      <Link
        to={`/projects/${projectId}/tasks/${node.parentId}`}
        className={styles.parentCrumb}
        title="Parent task"
      >
        ↳ {parent?.title ?? "…"}
      </Link>
    );
  };

  const renderRow = (node: TaskTreeNode, depth: number, flat = false): ReactNode => {
    const { task } = node;
    const hasChildren = !flat && node.children.length > 0;
    const isCollapsed = collapsed.has(task.id);
    const dependencyHint = describeDependencies(node, statusById);
    return (
      <Fragment key={task.id}>
        <tr data-status={task.status} className={styles.row}>
          <td className={styles.statusCell}>
            <StatusBadge status={task.status} />
          </td>
          <td>
            <div className={styles.titleCell} style={{ "--depth": depth } as CSSProperties}>
              {hasChildren ? (
                <button
                  type="button"
                  className={styles.collapse}
                  aria-expanded={!isCollapsed}
                  aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${task.title}`}
                  onClick={() => toggleCollapse(task.id)}
                >
                  {isCollapsed ? "▸" : "▾"}
                </button>
              ) : (
                <span className={styles.collapseSpacer} />
              )}
              <Link to={`/projects/${projectId}/tasks/${task.id}`}>{task.title}</Link>
              {node.subtaskCount > 0 ? (
                <span className={styles.chip}>
                  {node.subtaskCount} subtask{node.subtaskCount === 1 ? "" : "s"}
                </span>
              ) : null}
              {dependencyHint ? (
                <span className={styles.chipWaiting}>
                  {dependencyHint.kind === "waits_on"
                    ? `waits on ${dependencyHint.count}`
                    : `${dependencyHint.count} dependenc${dependencyHint.count === 1 ? "y" : "ies"}`}
                </span>
              ) : null}
              {parentContext(node, flat)}
            </div>
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
        {hasChildren && !isCollapsed
          ? node.children.map((child) => renderRow(child, depth + 1))
          : null}
      </Fragment>
    );
  };

  // Filtered views stay flat: a filter can match a child whose parent
  // doesn't match, so nesting would lie — context badges carry the hierarchy.
  const flatRows = (nodes: TaskTreeNode[]): TaskTreeNode[] =>
    nodes.flatMap((node) => [node, ...flatRows(node.children)]);

  return (
    <section>
      <PageHeader
        title="Tasks"
        actions={
          <Link to={`/projects/${projectId}/tasks/new`} className={styles.newTask}>
            New task
          </Link>
        }
      >
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
      </PageHeader>

      {query.isPending ? (
        <LoadingState label="Loading tasks…" />
      ) : query.isError ? (
        <ErrorState error={query.error} onRetry={() => void query.refetch()} />
      ) : tasks.length === 0 ? (
        <EmptyState title={filtered ? "No tasks match these filters" : "No tasks yet"}>
          <Link to={`/projects/${projectId}/tasks/new`}>Create the first task</Link>
        </EmptyState>
      ) : (
        <>
          <div className={styles.card}>
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
                {filtered
                  ? flatRows(tree).map((node) => renderRow(node, 0, true))
                  : tree.map((node) => renderRow(node, 0))}
              </tbody>
            </table>
          </div>
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
