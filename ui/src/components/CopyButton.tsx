import { useEffect, useRef, useState } from "react";
import styles from "./CopyButton.module.css";

/** Copies `value` to the clipboard with brief visual + SR confirmation. */
export function CopyButton({ value, label }: { value: string; label: string }) {
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
      // Clipboard unavailable (insecure context/permissions) — leave the
      // value selectable next to the button instead of pretending.
    }
  };

  return (
    <button type="button" className={styles.copy} aria-label={label} onClick={() => void copy()}>
      {copied ? "Copied ✓" : "Copy"}
    </button>
  );
}
