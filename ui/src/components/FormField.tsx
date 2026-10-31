import { useId, type ReactNode } from "react";
import styles from "./FormField.module.css";

interface FormFieldProps {
  label: string;
  error?: string;
  hint?: string;
  /** Render label inline beside the control (for checkboxes/toggles). */
  inline?: boolean;
  children: (props: {
    id: string;
    "aria-invalid": boolean | undefined;
    "aria-describedby": string | undefined;
  }) => ReactNode;
}

export function FormField({ label, error, hint, inline, children }: FormFieldProps) {
  const id = useId();
  const errorId = `${id}-error`;
  const hintId = `${id}-hint`;
  const childProps = {
    id,
    "aria-invalid": error ? (true as const) : undefined,
    "aria-describedby": error ? errorId : hint ? hintId : undefined,
  };

  if (inline) {
    return (
      <div className={styles.fieldInline}>
        {children(childProps)}
        <label className={styles.labelInline} htmlFor={id}>
          {label}
        </label>
        {error ? (
          <p className={styles.error} id={errorId}>
            {error}
          </p>
        ) : null}
      </div>
    );
  }

  return (
    <div className={styles.field}>
      <label className={styles.label} htmlFor={id}>
        {label}
      </label>
      {children(childProps)}
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
