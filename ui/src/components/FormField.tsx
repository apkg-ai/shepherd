import { useId, type ReactNode } from "react";
import styles from "./FormField.module.css";

interface FormFieldProps {
  label: string;
  error?: string;
  hint?: string;
  /** Render prop so the control can wire up the generated id + aria. */
  children: (props: {
    id: string;
    "aria-invalid": boolean | undefined;
    "aria-describedby": string | undefined;
  }) => ReactNode;
}

export function FormField({ label, error, hint, children }: FormFieldProps) {
  const id = useId();
  const errorId = `${id}-error`;
  const hintId = `${id}-hint`;
  return (
    <div className={styles.field}>
      <label className={styles.label} htmlFor={id}>
        {label}
      </label>
      {children({
        id,
        "aria-invalid": error ? true : undefined,
        // The hint is real guidance — expose it to AT, not just sighted users.
        "aria-describedby": error ? errorId : hint ? hintId : undefined,
      })}
      {hint && !error ? (
        <p className={styles.hint} id={hintId}>
          {hint}
        </p>
      ) : null}
      {error ? (
        <p className={styles.error} id={errorId}>
          {error}
        </p>
      ) : null}
    </div>
  );
}
