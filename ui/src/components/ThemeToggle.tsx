import { useEffect, useState } from "react";
import { applyThemePreference, readThemePreference, type ThemePreference } from "../lib/theme";
import { watchSystemTheme } from "../lib/theme";
import styles from "./ThemeToggle.module.css";

const OPTIONS: { value: ThemePreference; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

export function ThemeToggle() {
  const [preference, setPreference] = useState<ThemePreference>(() => readThemePreference());

  useEffect(() => {
    applyThemePreference(preference);
    if (preference !== "system") return;
    // While following the OS, re-resolve when it flips.
    return watchSystemTheme(() => applyThemePreference("system"));
  }, [preference]);

  return (
    <div role="group" aria-label="Theme" className={styles.group}>
      {OPTIONS.map((option) => (
        <button
          key={option.value}
          type="button"
          className={preference === option.value ? styles.active : styles.option}
          aria-pressed={preference === option.value}
          onClick={() => setPreference(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
