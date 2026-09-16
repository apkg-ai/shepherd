import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { applyThemePreference, readThemePreference, type ThemePreference } from "../lib/theme";
import { watchSystemTheme } from "../lib/theme";
import styles from "./ThemeToggle.module.css";

const OPTIONS: { value: ThemePreference; labelKey: string }[] = [
  { value: "light", labelKey: "theme.toggle.light" },
  { value: "dark", labelKey: "theme.toggle.dark" },
  { value: "system", labelKey: "theme.toggle.system" },
];

export function ThemeToggle() {
  const { t } = useTranslation();
  const [preference, setPreference] = useState<ThemePreference>(() => readThemePreference());

  useEffect(() => {
    applyThemePreference(preference);
    if (preference !== "system") return;
    return watchSystemTheme(() => applyThemePreference("system"));
  }, [preference]);

  return (
    <div role="group" aria-label={t("theme.toggle.label")} className={styles.group}>
      {OPTIONS.map((option) => (
        <button
          key={option.value}
          type="button"
          className={preference === option.value ? styles.active : styles.option}
          aria-pressed={preference === option.value}
          onClick={() => setPreference(option.value)}
        >
          {t(option.labelKey)}
        </button>
      ))}
    </div>
  );
}
