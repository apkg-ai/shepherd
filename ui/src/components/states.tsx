import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { isShepherdError } from "../api/problem";
import { Button } from "./Button";
import styles from "./states.module.css";

export function LoadingState({ label }: { label?: string }) {
  const { t } = useTranslation();
  return (
    <p className={styles.state} role="status">
      {label ?? t("common.state.loading")}
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
  const { t } = useTranslation();
  const title = isShepherdError(error) ? error.title : t("common.error.title");
  const detail = isShepherdError(error) ? error.detail : undefined;
  return (
    <div className={styles.state} role="alert">
      <p className={styles.errorTitle}>{title}</p>
      {detail ? <p className={styles.errorDetail}>{detail}</p> : null}
      {onRetry ? <Button onClick={onRetry}>{t("common.action.retry")}</Button> : null}
    </div>
  );
}
