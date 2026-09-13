import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { Link } from "react-router";
import type { Relation, RelationCreateType, Task } from "../../api/generated/model";
import { useCreateTaskRelation } from "../../api/generated/relations/relations";
import { useListTasks } from "../../api/generated/tasks/tasks";
import { invalidatePaths } from "../../api/invalidate";
import { isShepherdError } from "../../api/problem";
import { AttemptBadge } from "../../components/AttemptBadge";
import { IdentityChip } from "../../components/IdentityChip";
import { StatusBadge } from "../../components/StatusBadge";
import { formatDateTime } from "../../lib/format";
import styles from "./GraphSidePanel.module.css";

/**
 * The selected node's context, without leaving the graph: core fields plus
 * its edges, each a jump to that node's own panel. Full task detail stays
 * one link away. Data comes from the graph's already-loaded feeds, so the
 * panel is live off the same SSE invalidations for free.
 *
 * In the tree lens, clicking a subtask triggers expand-and-focus: the
 * current task's children expand, and the camera pans to the target.
 */
export function GraphSidePanel({
  task,
  projectId,
  tasks,
  relations,
  lens,
  onSelect,
  onExpandAndFocus,
  onClose,
}: {
  task: Task;
  projectId: string;
  tasks: readonly Task[];
  relations: readonly Relation[];
  lens: "tree" | "flow";
  onSelect: (taskId: string) => void;
  onExpandAndFocus: (parentId: string, childId: string) => void;
  onClose: () => void;
}) {
  const taskById = new Map(tasks.map((t) => [t.id, t]));
  const titleById = new Map(tasks.map((t) => [t.id, t.title]));

  const parent = relations.find(
    (r) =>
      r.type === "decomposition" && r.target_task_id === task.id && r.source_task_id !== task.id,
  )?.source_task_id;
  const subtaskIds = relations
    .filter((r) => r.type === "decomposition" && r.source_task_id === task.id)
    .map((r) => r.target_task_id);
  const dependsOn = relations
    .filter((r) => r.type === "depends_on" && r.source_task_id === task.id)
    .map((r) => r.target_task_id);
  const neededBy = relations
    .filter((r) => r.type === "depends_on" && r.target_task_id === task.id)
    .map((r) => r.source_task_id);

  const group = (label: string, ids: readonly string[]) => {
    const loaded = ids.filter((id) => titleById.has(id));
    if (loaded.length === 0) return null;
    return (
      <div className={styles.group}>
        <div className={styles.groupLabel}>{label}</div>
        <ul className={styles.groupList}>
          {loaded.map((id) => (
            <li key={id}>
              <button type="button" className={styles.jump} onClick={() => onSelect(id)}>
                {titleById.get(id)}
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  };

  /** Rich subtask list with status badges and progress summary. */
  const subtaskGroup = () => {
    const subs = subtaskIds.map((id) => taskById.get(id)).filter(Boolean) as Task[];
    if (subs.length === 0) return null;
    const done = subs.filter((s) => s.status === "done").length;
    return (
      <div className={styles.group}>
        <div className={styles.groupLabel}>
          Subtasks — {done}/{subs.length} done
        </div>
        <div className={styles.progressTrack}>
          <div
            className={styles.progressFill}
            style={{ width: `${(done / subs.length) * 100}%` }}
          />
        </div>
        <ul className={styles.subtaskList}>
          {subs.map((s) => (
            <li key={s.id} className={styles.subtaskRow}>
              <StatusBadge status={s.status} />
              <button
                type="button"
                className={styles.jump}
                onClick={() => (lens === "tree" ? onExpandAndFocus(task.id, s.id) : onSelect(s.id))}
              >
                {s.title}
              </button>
              {s.assignee ? <IdentityChip identity={s.assignee} /> : null}
            </li>
          ))}
        </ul>
      </div>
    );
  };

  return (
    <aside className={styles.panel} aria-label={`Selected task: ${task.title}`}>
      <header className={styles.header}>
        <h3 className={styles.title}>{task.title}</h3>
        <button type="button" className={styles.close} aria-label="Close panel" onClick={onClose}>
          ×
        </button>
      </header>

      <div className={styles.badges}>
        <StatusBadge status={task.status} />
        <span className={styles.type}>{task.type}</span>
        {task.assignee ? <IdentityChip identity={task.assignee} /> : null}
        <AttemptBadge attempts={task.attempt_count} />
      </div>

      {task.description !== "" ? (
        <p className={styles.description}>{task.description}</p>
      ) : (
        <p className={styles.muted}>No description.</p>
      )}

      {group("Parent", parent !== undefined ? [parent] : [])}
      {subtaskGroup()}
      {group("Depends on", dependsOn)}
      {group("Needed by", neededBy)}

      <GraphSidePanelActions projectId={projectId} task={task} hasParent={parent !== undefined} />

      <footer className={styles.footer}>
        <Link to={`/projects/${projectId}/tasks/${task.id}`}>Open full detail</Link>
        <span className={styles.muted}>Updated {formatDateTime(task.updated_at)}</span>
      </footer>
    </aside>
  );
}

type LinkKind = "depends_on" | "subtask" | "parent";

function GraphSidePanelActions({
  projectId,
  task,
  hasParent,
}: {
  projectId: string;
  task: Task;
  hasParent: boolean;
}) {
  const queryClient = useQueryClient();
  const [showLink, setShowLink] = useState(false);
  const [linkKind, setLinkKind] = useState<LinkKind>("depends_on");
  const [targetId, setTargetId] = useState("");
  const [error, setError] = useState<string | null>(null);

  const candidatesQuery = useListTasks(projectId, { limit: 100 }, { query: { enabled: showLink } });
  const candidates = (candidatesQuery.data?.items ?? []).filter((t) => t.id !== task.id);

  const createRelation = useCreateTaskRelation({ mutation: { meta: { silent: true } } });

  const submitLink = (e: FormEvent) => {
    e.preventDefault();
    if (targetId === "") {
      setError("Pick a task.");
      return;
    }
    setError(null);
    const type: RelationCreateType = linkKind === "depends_on" ? "depends_on" : "decomposition";
    const sourceTaskId = linkKind === "parent" ? targetId : task.id;
    const targetTaskId = linkKind === "parent" ? task.id : targetId;
    createRelation.mutate(
      { projectId, taskId: sourceTaskId, data: { type, target_task_id: targetTaskId } },
      {
        onSuccess: () => {
          invalidatePaths(queryClient, `/api/v1/projects/${projectId}/relations`);
          setShowLink(false);
          setTargetId("");
          setError(null);
        },
        onError: (err) => setError(isShepherdError(err) ? err.message : "Request failed"),
      },
    );
  };

  return (
    <div className={styles.actionsSection}>
      <div className={styles.actions}>
        <Link
          to={`/projects/${projectId}/tasks/new?parent=${task.id}`}
          className={styles.actionLink}
        >
          + New subtask
        </Link>
        <Link
          to={`/projects/${projectId}/tasks/new?depends_on=${task.id}`}
          className={styles.actionLink}
        >
          + New dependency
        </Link>
        <button type="button" className={styles.actionLink} onClick={() => setShowLink((v) => !v)}>
          {showLink ? "Cancel" : "+ Link task"}
        </button>
      </div>

      {showLink ? (
        <form onSubmit={submitLink} className={styles.linkForm}>
          <select
            value={linkKind}
            onChange={(e) => setLinkKind(e.target.value as LinkKind)}
            className={styles.linkSelect}
          >
            <option value="depends_on">This task depends on…</option>
            <option value="subtask">Add subtask…</option>
            {!hasParent ? <option value="parent">Set parent…</option> : null}
          </select>
          <select
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            className={styles.linkSelect}
          >
            <option value="">{candidatesQuery.isPending ? "Loading…" : "Pick a task"}</option>
            {candidates.map((t) => (
              <option key={t.id} value={t.id}>
                {t.title}
              </option>
            ))}
          </select>
          {error ? <p className={styles.linkError}>{error}</p> : null}
          <button type="submit" className={styles.linkSubmit} disabled={createRelation.isPending}>
            {createRelation.isPending ? "Adding…" : "Add"}
          </button>
        </form>
      ) : null}
    </div>
  );
}
