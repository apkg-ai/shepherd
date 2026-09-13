import { Link } from "react-router";
import type { Relation, Task } from "../../api/generated/model";
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
 */
export function GraphSidePanel({
  task,
  projectId,
  tasks,
  relations,
  onSelect,
  onClose,
}: {
  task: Task;
  projectId: string;
  tasks: readonly Task[];
  relations: readonly Relation[];
  onSelect: (taskId: string) => void;
  onClose: () => void;
}) {
  const titleById = new Map(tasks.map((t) => [t.id, t.title]));

  const parent = relations.find(
    (r) =>
      r.type === "decomposition" && r.target_task_id === task.id && r.source_task_id !== task.id,
  )?.source_task_id;
  const subtasks = relations
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
      {group("Subtasks", subtasks)}
      {group("Depends on", dependsOn)}
      {group("Needed by", neededBy)}

      <footer className={styles.footer}>
        <Link to={`/projects/${projectId}/tasks/${task.id}`}>Open full detail</Link>
        <span className={styles.muted}>Updated {formatDateTime(task.updated_at)}</span>
      </footer>
    </aside>
  );
}
