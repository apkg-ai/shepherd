import { useTranslation } from "react-i18next";
import styles from "./StatusFilter.module.css";

type FilterValue = "active" | "done" | "all";

interface StatusFilterProps {
  value: FilterValue;
  onChange: (value: FilterValue) => void;
  counts: { active: number; done: number; total: number };
}

const OPTIONS: Array<{ value: FilterValue; key: string }> = [
  { value: "active", key: "filter.status.active" },
  { value: "done", key: "filter.status.done" },
  { value: "all", key: "filter.status.all" },
];

export function StatusFilter({ value, onChange, counts }: StatusFilterProps) {
  const { t } = useTranslation();

  const countMap: Record<FilterValue, number> = {
    active: counts.active,
    done: counts.done,
    all: counts.total,
  };

  return (
    <div className={styles.bar} role="group" aria-label={t("filter.status.label")}>
      {OPTIONS.map((opt) => (
        <button
          key={opt.value}
          type="button"
          className={value === opt.value ? styles.active : styles.option}
          aria-pressed={value === opt.value}
          onClick={() => onChange(opt.value)}
        >
          {t(opt.key)}
          <span className={styles.count}>{countMap[opt.value]}</span>
        </button>
      ))}
    </div>
  );
}
