import { useTranslation } from "react-i18next";
import type { EntityStatus } from "../api/generated/model/entityStatus";
import styles from "./StatusBadge.module.css";

const statusKeys: Record<EntityStatus, string> = {
  proposed: "common.status.proposed",
  open: "common.status.open",
  active: "common.status.active",
  done: "common.status.done",
  cancelled: "common.status.cancelled",
};

interface StatusBadgeProps {
  status: string;
}

export function StatusBadge({ status }: StatusBadgeProps) {
  const { t } = useTranslation();
  const key = status in statusKeys ? statusKeys[status as EntityStatus] : undefined;
  return (
    <span className={styles.badge} data-status={status}>
      {key ? t(key) : status}
    </span>
  );
}
