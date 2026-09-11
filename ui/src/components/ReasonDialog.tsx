import { useState, type FormEvent } from "react";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { FormField } from "./FormField";
import styles from "./ReasonDialog.module.css";

interface ReasonDialogProps {
  open: boolean;
  title: string;
  label: string;
  confirmLabel: string;
  danger?: boolean;
  busy?: boolean;
  /** Returns an error message to display, or null when the reason is valid. */
  validate: (reason: string) => string | null;
  onSubmit: (reason: string) => void;
  onCancel: () => void;
}

/** Modal asking for a required reason (reject, block). */
export function ReasonDialog({
  open,
  title,
  label,
  confirmLabel,
  danger = false,
  busy = false,
  validate,
  onSubmit,
  onCancel,
}: ReasonDialogProps) {
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | undefined>(undefined);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const problem = validate(reason);
    if (problem) {
      setError(problem);
      return;
    }
    setError(undefined);
    onSubmit(reason);
  };

  return (
    <Dialog open={open} title={title} onClose={onCancel}>
      <form onSubmit={submit} noValidate>
        <FormField label={label} error={error}>
          {(props) => (
            <textarea
              {...props}
              rows={3}
              value={reason}
              onChange={(e) => setReason(e.target.value)}
            />
          )}
        </FormField>
        <div className={styles.actions}>
          <Button onClick={onCancel}>Cancel</Button>
          <Button type="submit" variant={danger ? "danger" : "primary"} busy={busy}>
            {confirmLabel}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
