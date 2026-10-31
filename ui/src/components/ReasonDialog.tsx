import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { FormField } from "./FormField";
import inputStyles from "./Input.module.css";
import styles from "./ConfirmDialog.module.css";

interface ReasonDialogProps {
  open: boolean;
  title: string;
  busy?: boolean;
  onSubmit: (reason: string) => void;
  onCancel: () => void;
}

export function ReasonDialog({ open, title, busy = false, onSubmit, onCancel }: ReasonDialogProps) {
  const { t } = useTranslation();
  const [reason, setReason] = useState("");

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = reason.trim();
    if (trimmed) {
      onSubmit(trimmed);
      setReason("");
    }
  };

  const handleClose = () => {
    setReason("");
    onCancel();
  };

  return (
    <Dialog open={open} title={title} onClose={handleClose}>
      <form onSubmit={handleSubmit}>
        <div className={styles.body}>
          <FormField label={t("reason.dialog.label")}>
            {(props) => (
              <textarea
                {...props}
                className={inputStyles.textarea}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                placeholder={t("reason.dialog.placeholder")}
                rows={3}
                required
              />
            )}
          </FormField>
        </div>
        <div className={styles.actions}>
          <Button type="button" onClick={handleClose}>
            {t("common.action.cancel")}
          </Button>
          <Button type="submit" variant="primary" busy={busy} disabled={!reason.trim()}>
            {t("reason.dialog.submit")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
