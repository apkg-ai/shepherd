import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router";
import { useImportProject } from "../../api/generated/export-import/export-import";
import type { ExportDocument } from "../../api/generated/model";
import { invalidatePaths } from "../../api/invalidate";
import { isShepherdError } from "../../api/problem";
import { useToast } from "../../components/Toast";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { FormField } from "../../components/FormField";
import styles from "./dialogs.module.css";

/**
 * Imports an export document as a NEW project (no merge semantics in v1,
 * docs/02-domain-model.md).
 */
export function ImportProjectDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} title="Import project" onClose={onClose}>
      {/* Dialog unmounts children when closed, so the picked file/error reset
          on every open — a stale File must never be imported silently. */}
      <ImportForm onClose={onClose} />
    </Dialog>
  );
}

function ImportForm({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { toast } = useToast();
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);

  const importProject = useImportProject({
    mutation: { meta: { silent: true } },
  });

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!file) {
      setError("Choose an export file first.");
      return;
    }
    let document: ExportDocument;
    try {
      document = JSON.parse(await file.text()) as ExportDocument;
    } catch {
      setError("That file is not valid JSON.");
      return;
    }
    setError(null);
    importProject.mutate(
      { data: document },
      {
        onSuccess: (result) => {
          invalidatePaths(queryClient, "/api/v1/projects");
          toast(
            `Imported ${result.task_count} tasks, ${result.relation_count} relations, ` +
              `${result.session_count} sessions, ${result.knowledge_count} knowledge items.`,
            "success",
          );
          onClose();
          void navigate(`/projects/${result.project_id}`);
        },
        onError: (err) => {
          setError(isShepherdError(err) ? err.message : "Import failed");
        },
      },
    );
  };

  return (
    <form onSubmit={(e) => void submit(e)} noValidate>
      <FormField
        label="Export file"
        hint="A shepherd export document (.json). Import always creates a new project."
      >
        {(props) => (
          <input
            {...props}
            type="file"
            accept=".json,application/json"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
          />
        )}
      </FormField>
      {error ? (
        <p className={styles.formError} role="alert">
          {error}
        </p>
      ) : null}
      <div className={styles.actions}>
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" busy={importProject.isPending}>
          Import
        </Button>
      </div>
    </form>
  );
}
