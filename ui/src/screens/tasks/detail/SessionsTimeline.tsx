import type { Session } from "../../../api/generated/model";
import { IdentityChip } from "../../../components/IdentityChip";
import { formatDateTime, formatDuration } from "../../../lib/format";
import styles from "./SessionsTimeline.module.css";

/** Chronological work sessions — the task's attempt history, failures first-class. */
export function SessionsTimeline({ sessions }: { sessions: Session[] }) {
  if (sessions.length === 0) {
    return <p className={styles.muted}>No sessions reported yet.</p>;
  }
  return (
    <ol className={styles.timeline}>
      {sessions.map((session) => (
        <li key={session.id} className={styles.entry} data-outcome={session.outcome}>
          <div className={styles.entryHeader}>
            <span className={session.outcome === "failed" ? styles.failed : styles.succeeded}>
              {session.outcome === "failed" ? "✗ Failed" : "✓ Succeeded"}
            </span>
            <IdentityChip identity={session.identity} />
            <span className={styles.muted}>
              {formatDateTime(session.started_at)} ·{" "}
              {formatDuration(session.started_at, session.ended_at)}
            </span>
          </div>
          <p className={styles.summary}>{session.summary}</p>
          {session.outcome === "failed" && session.failure_reason ? (
            <p className={styles.failureReason}>{session.failure_reason}</p>
          ) : null}
          {session.decisions.length > 0 ? (
            <ul className={styles.decisions}>
              {session.decisions.map((decision) => (
                <li key={decision}>{decision}</li>
              ))}
            </ul>
          ) : null}
          {session.artifacts.length > 0 ? (
            <p className={styles.artifacts}>
              {session.artifacts.map((artifact) => (
                <a key={artifact} href={artifact} target="_blank" rel="noreferrer">
                  {artifact}
                </a>
              ))}
            </p>
          ) : null}
          {session.knowledge_items.length > 0 ? (
            <ul className={styles.knowledge}>
              {session.knowledge_items.map((item) => (
                <li key={item.id}>
                  <span className={styles.knowledgeType}>{item.type}</span> {item.title}
                </li>
              ))}
            </ul>
          ) : null}
        </li>
      ))}
    </ol>
  );
}
