import { useTranslation } from "react-i18next";
import type { Counts } from "../api/generated/model/counts";
import styles from "./ProgressCounts.module.css";

interface ProgressCountsProps {
  counts: Counts;
  label?: string;
}

export function ProgressCounts({ counts, label }: ProgressCountsProps) {
  const { t } = useTranslation();
  const parts: string[] = [];

  parts.push(`${counts.done} ${t("counts.progress.of")} ${counts.total}`);

  if (counts.cancelled > 0) {
    parts.push(t("counts.progress.cancelled", { count: counts.cancelled }));
  }
  if (counts.waived > 0) {
    parts.push(t("counts.progress.waived", { count: counts.waived }));
  }

  return (
    <span className={styles.counts} aria-label={label}>
      {parts.join(" · ")}
    </span>
  );
}
