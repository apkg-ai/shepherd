import type { ReactNode } from "react";
import { isShepherdError } from "../api/problem";
import { Button } from "./Button";
import styles from "./states.module.css";

export function LoadingState({ label = "Loading…" }: { label?: string }) {
  return (
    <p className={styles.state} role="status">
      {label}
    </p>
  );
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className={styles.state}>
      <p className={styles.emptyTitle}>{title}</p>
      {children}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const title = isShepherdError(error) ? error.title : "Something went wrong";
  const detail = isShepherdError(error) ? error.detail : undefined;
  return (
    <div className={styles.state} role="alert">
      <p className={styles.errorTitle}>{title}</p>
      {detail ? <p className={styles.errorDetail}>{detail}</p> : null}
      {onRetry ? <Button onClick={onRetry}>Retry</Button> : null}
    </div>
  );
}
