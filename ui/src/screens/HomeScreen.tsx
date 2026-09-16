import { useGetHealth } from "../api/generated/system/system";
import { PageHeader } from "../components/PageHeader";
import { ErrorState, LoadingState } from "../components/states";
import styles from "./HomeScreen.module.css";

export function HomeScreen() {
  const health = useGetHealth();

  if (health.isPending) return <LoadingState />;
  if (health.isError) {
    return <ErrorState error={health.error} onRetry={() => void health.refetch()} />;
  }

  return (
    <>
      <PageHeader
        title="Shepherd"
        description="v1 scaffold — the daemon serves this shell and a health endpoint."
      />
      <dl className={styles.facts}>
        <div className={styles.fact}>
          <dt className={styles.factLabel}>Status</dt>
          <dd className={styles.factValue}>{health.data.status}</dd>
        </div>
        <div className={styles.fact}>
          <dt className={styles.factLabel}>Version</dt>
          <dd className={styles.factValue}>{health.data.version}</dd>
        </div>
        {health.data.description ? (
          <div className={styles.fact}>
            <dt className={styles.factLabel}>Service</dt>
            <dd className={styles.factValue}>{health.data.description}</dd>
          </div>
        ) : null}
      </dl>
    </>
  );
}
