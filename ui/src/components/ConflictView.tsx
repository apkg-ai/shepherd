import { useTranslation } from "react-i18next";
import { Button } from "./Button";
import styles from "./ConflictView.module.css";

interface ConflictField {
  label: string;
  local: string;
  remote: string;
}

interface ConflictViewProps {
  fields: ConflictField[];
  onRetry: () => void;
  onDiscard: () => void;
}

export function ConflictView({ fields, onRetry, onDiscard }: ConflictViewProps) {
  const { t } = useTranslation();
  const emptyLabel = t("conflict.view.none");

  return (
    <>
      <div className={styles.grid}>
        <div className={styles.pane}>
          <h3 className={styles.label}>{t("conflict.view.yours")}</h3>
          <dl className={styles.fields}>
            {fields.map((f) => (
              <div key={f.label}>
                <dt>{f.label}</dt>
                <dd>{f.local || emptyLabel}</dd>
              </div>
            ))}
          </dl>
        </div>
        <div className={styles.pane}>
          <h3 className={styles.label}>{t("conflict.view.theirs")}</h3>
          <dl className={styles.fields}>
            {fields.map((f) => (
              <div key={f.label}>
                <dt>{f.label}</dt>
                <dd>{f.remote || emptyLabel}</dd>
              </div>
            ))}
          </dl>
        </div>
      </div>
      <div className={styles.actions}>
        <Button onClick={onDiscard}>{t("conflict.view.discard")}</Button>
        <Button variant="primary" onClick={onRetry}>
          {t("conflict.view.retry")}
        </Button>
      </div>
    </>
  );
}
