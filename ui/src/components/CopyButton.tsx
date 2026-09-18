import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import styles from "./CopyButton.module.css";

export function CopyButton({ value, label }: { value: string; label: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => () => clearTimeout(resetTimer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      clearTimeout(resetTimer.current);
      resetTimer.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard unavailable — stay quiet rather than fake success.
    }
  };

  return (
    <button type="button" className={styles.copy} aria-label={label} onClick={() => void copy()}>
      {copied ? t("common.action.copied") : t("common.action.copy")}
    </button>
  );
}
