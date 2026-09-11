import styles from "./AttemptBadge.module.css";

/**
 * Failure-count language from docs/04-ui.md: repeated failures must be
 * visible at a glance. Renders nothing until there has been an attempt.
 */
export function AttemptBadge({
  attempts,
  failures = 0,
  atLeast = false,
}: {
  attempts: number;
  failures?: number;
  /** True when `failures` was counted from partially loaded data. */
  atLeast?: boolean;
}) {
  if (attempts <= 0) return null;
  const label =
    failures > 0
      ? `${attempts} attempt${attempts === 1 ? "" : "s"}, ${atLeast ? "≥" : ""}${failures} failed`
      : `${attempts} attempt${attempts === 1 ? "" : "s"}`;
  return (
    <span className={failures > 0 ? styles.failed : styles.badge} title={label}>
      {label}
    </span>
  );
}
