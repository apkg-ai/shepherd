import type { TaskMetadata } from "../../api/generated/model";
import { Button } from "../../components/Button";
import { FormField } from "../../components/FormField";
import styles from "./MetadataEditor.module.css";

const MAX_PROPERTIES = 200; // TaskMetadata maxProperties in the spec

export type MetadataValidation = { ok: true; value: TaskMetadata } | { ok: false; error: string };

/** Validates the metadata textarea: JSON, plain object, ≤ 200 top-level keys. */
export function validateMetadata(text: string): MetadataValidation {
  if (text.trim() === "") return { ok: true, value: {} };
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { ok: false, error: "Metadata must be valid JSON." };
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { ok: false, error: "Metadata must be a JSON object." };
  }
  if (Object.keys(parsed).length > MAX_PROPERTIES) {
    return {
      ok: false,
      error: `Metadata can have at most ${MAX_PROPERTIES} top-level keys.`,
    };
  }
  return { ok: true, value: parsed as TaskMetadata };
}

interface MetadataEditorProps {
  value: string;
  onChange: (value: string) => void;
  error?: string;
}

/** Raw-JSON metadata editor (v1 — no tree editor dependency). */
export function MetadataEditor({ value, onChange, error }: MetadataEditorProps) {
  const format = () => {
    const result = validateMetadata(value);
    if (result.ok) onChange(JSON.stringify(result.value, null, 2));
  };

  return (
    <div className={styles.wrap}>
      <FormField
        label="Metadata"
        error={error}
        hint="Freeform JSON object attached to the task (structured per task-type conventions)."
      >
        {(props) => (
          <textarea
            {...props}
            className={styles.editor}
            rows={6}
            spellCheck={false}
            value={value}
            onChange={(e) => onChange(e.target.value)}
          />
        )}
      </FormField>
      <Button variant="ghost" onClick={format}>
        Format
      </Button>
    </div>
  );
}
