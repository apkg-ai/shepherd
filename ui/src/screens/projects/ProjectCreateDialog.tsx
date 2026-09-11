import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router";
import { useCreateProject } from "../../api/generated/projects/projects";
import { CreateProjectBody } from "../../api/generated/zod/projects/projects.zod";
import { invalidatePaths } from "../../api/invalidate";
import { fieldErrors, isShepherdError } from "../../api/problem";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { FormField } from "../../components/FormField";
import { zodFieldErrors } from "../../lib/forms";
import styles from "./dialogs.module.css";

export function ProjectCreateDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [reviewGate, setReviewGate] = useState(true);
  const [errors, setErrors] = useState<Record<string, string>>({});

  const createProject = useCreateProject({
    mutation: { meta: { silent: true } },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const parsed = CreateProjectBody.safeParse({
      name,
      description,
      settings: { review_gate: reviewGate },
    });
    if (!parsed.success) {
      setErrors(zodFieldErrors(parsed.error));
      return;
    }
    setErrors({});
    createProject.mutate(
      { data: parsed.data },
      {
        onSuccess: (project) => {
          invalidatePaths(queryClient, "/api/v1/projects");
          onClose();
          void navigate(`/projects/${project.id}`);
        },
        onError: (error) => {
          if (isShepherdError(error) && error.status === 422) {
            setErrors(fieldErrors(error));
          } else {
            setErrors({
              _form: isShepherdError(error) ? error.message : "Request failed",
            });
          }
        },
      },
    );
  };

  return (
    <Dialog open={open} title="Register project" onClose={onClose}>
      <form onSubmit={submit} noValidate>
        <FormField label="Name" error={errors["name"]}>
          {(props) => (
            <input {...props} type="text" value={name} onChange={(e) => setName(e.target.value)} />
          )}
        </FormField>
        <FormField label="Description" error={errors["description"]}>
          {(props) => (
            <textarea
              {...props}
              rows={3}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          )}
        </FormField>
        <label className={styles.checkbox}>
          <input
            type="checkbox"
            checked={reviewGate}
            onChange={(e) => setReviewGate(e.target.checked)}
          />
          Review gate — agent work needs human approval before it counts as done
        </label>
        {errors["_form"] ? (
          <p className={styles.formError} role="alert">
            {errors["_form"]}
          </p>
        ) : null}
        <div className={styles.actions}>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" busy={createProject.isPending}>
            Register
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
