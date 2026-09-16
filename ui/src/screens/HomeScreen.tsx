import { useTranslation } from "react-i18next";
import { useGetHealth } from "../api/generated/system/system";
import { PageHeader } from "../components/PageHeader";
import { ErrorState, LoadingState } from "../components/states";
import styles from "./HomeScreen.module.css";

export function HomeScreen() {
  const { t } = useTranslation();
  const health = useGetHealth();

  if (health.isPending) return <LoadingState />;
  if (health.isError) {
    return <ErrorState error={health.error} onRetry={() => void health.refetch()} />;
  }

  return (
    <>
      <PageHeader title={t("home.screen.title")} description={t("home.screen.description")} />
      <dl className={styles.facts}>
        <div className={styles.fact}>
          <dt className={styles.factLabel}>{t("home.fact.status")}</dt>
          <dd className={styles.factValue}>{health.data.status}</dd>
        </div>
        <div className={styles.fact}>
          <dt className={styles.factLabel}>{t("home.fact.version")}</dt>
          <dd className={styles.factValue}>{health.data.version}</dd>
        </div>
        {health.data.description ? (
          <div className={styles.fact}>
            <dt className={styles.factLabel}>{t("home.fact.service")}</dt>
            <dd className={styles.factValue}>{health.data.description}</dd>
          </div>
        ) : null}
      </dl>
    </>
  );
}
