import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { useNavigate, useParams } from "react-router";
import { exportProject } from "../../api/generated/export-import/export-import";
import type { Project } from "../../api/generated/model";
import {
  useDeleteProject,
  useGetProject,
  useUpdateProject,
} from "../../api/generated/projects/projects";
import { UpdateProjectBody } from "../../api/generated/zod/projects/projects.zod";
import { invalidatePaths } from "../../api/invalidate";
import { fieldErrors, isShepherdError } from "../../api/problem";
import { Button } from "../../components/Button";
import { CopyButton } from "../../components/CopyButton";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { FormField } from "../../components/FormField";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { PageHeader } from "../../components/PageHeader";
import { useToast } from "../../components/Toast";
import { downloadJson, slugify } from "../../lib/download";
import { formatDateTime } from "../../lib/format";
import { zodFieldErrors } from "../../lib/forms";
import styles from "./ProjectSettingsScreen.module.css";

export function ProjectSettingsScreen() {
  const { projectId } = useParams();
  if (!projectId) return <EmptyState title="No project selected" />;
  return <Settings projectId={projectId} />;
}

function Settings({ projectId }: { projectId: string }) {
  const projectQuery = useGetProject(projectId);

  // Mount the form only once the project is loaded so field state initializes
  // from data directly — no prefill effect, and a background refetch never
  // clobbers in-progress edits.
  if (projectQuery.isPending) return <LoadingState label="Loading project…" />;
  if (projectQuery.isError) {
    return <ErrorState error={projectQuery.error} onRetry={() => void projectQuery.refetch()} />;
  }
  // Keyed by project so form state never bleeds when navigating between
  // projects whose data is already cached (isPending stays false).
  return <SettingsForm key={projectId} project={projectQuery.data} />;
}

function SettingsForm({ project }: { project: Project }) {
  const projectId = project.id;
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { toast } = useToast();

  const [name, setName] = useState(project.name);
  const [description, setDescription] = useState(project.description);
  const [reviewGate, setReviewGate] = useState(project.settings.review_gate);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [exporting, setExporting] = useState(false);

  const updateProject = useUpdateProject({ mutation: { meta: { silent: true } } });
  const deleteProject = useDeleteProject();

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const parsed = UpdateProjectBody.safeParse({
      name,
      description,
      settings: { review_gate: reviewGate },
    });
    if (!parsed.success) {
      setErrors(zodFieldErrors(parsed.error));
      return;
    }
    setErrors({});
    updateProject.mutate(
      { projectId, data: parsed.data },
      {
        onSuccess: () => {
          invalidatePaths(queryClient, "/api/v1/projects", `/api/v1/projects/${projectId}`);
          toast("Project settings saved.", "success");
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

  const onExport = async () => {
    setExporting(true);
    try {
      const document = await exportProject(projectId);
      downloadJson(document, `${slugify(name)}-export.json`);
    } catch (error) {
      toast(isShepherdError(error) ? error.message : "Export failed", "error");
    } finally {
      setExporting(false);
    }
  };

  const onDelete = () => {
    deleteProject.mutate(
      { projectId },
      {
        onSuccess: () => {
          invalidatePaths(queryClient, "/api/v1/projects");
          toast("Project deleted.", "success");
          void navigate("/");
        },
        onSettled: () => setDeleteOpen(false),
      },
    );
  };

  return (
    <section className={styles.wrap}>
      <PageHeader
        title="Project settings"
        description="Name, review gate, portability, and the danger zone."
      />

      <div className={styles.columns}>
        <div className={styles.column}>
          <section className={styles.card}>
            <header className={styles.cardHeader}>
              <h3>General</h3>
              <p className={styles.muted}>The project's identity and how agent work completes.</p>
              <p className={styles.metaLine}>
                Created {formatDateTime(project.created_at)} · updated{" "}
                {formatDateTime(project.updated_at)}
              </p>
            </header>
            <form onSubmit={submit} noValidate>
              <FormField label="Name" error={errors["name"]}>
                {(props) => (
                  <input
                    {...props}
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                  />
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
              <label className={styles.gateRow}>
                <input
                  type="checkbox"
                  checked={reviewGate}
                  onChange={(e) => setReviewGate(e.target.checked)}
                />
                <span>
                  <strong>Review gate</strong> — on: successful agent sessions land in review for
                  human approval; off: they complete straight to done.
                </span>
              </label>
              {errors["_form"] ? (
                <p className={styles.formError} role="alert">
                  {errors["_form"]}
                </p>
              ) : null}
              <footer className={styles.cardFooter}>
                <Button type="submit" variant="primary" busy={updateProject.isPending}>
                  Save settings
                </Button>
              </footer>
            </form>
          </section>
        </div>

        <div className={styles.column}>
          <section className={styles.card}>
            <header className={styles.cardHeader}>
              <h3>Agent access</h3>
              <p className={styles.muted}>
                Agents work this project through the REST API (docs/agent-guide.md) — hand them
                these.
              </p>
            </header>
            <dl className={styles.accessList}>
              <dt>Server</dt>
              <dd>
                <code className={styles.mono}>{window.location.origin}</code>
                <CopyButton value={window.location.origin} label="Copy server URL" />
              </dd>
              <dt>Project ID</dt>
              <dd>
                <code className={styles.mono}>{projectId}</code>
                <CopyButton value={projectId} label="Copy project ID" />
              </dd>
              <dt>Connection check</dt>
              <dd>
                <code className={styles.mono}>
                  curl {window.location.origin}/api/v1/projects/{projectId}/next-task
                </code>
                <CopyButton
                  value={`curl ${window.location.origin}/api/v1/projects/${projectId}/next-task`}
                  label="Copy connection check command"
                />
              </dd>
            </dl>
          </section>

          <section className={styles.card}>
            <div className={styles.actionRow}>
              <div>
                <h3>Export</h3>
                <p className={styles.muted}>
                  Download the full project — tasks, relations, sessions, knowledge — as a portable
                  JSON document.
                </p>
              </div>
              <Button busy={exporting} onClick={() => void onExport()}>
                Export project
              </Button>
            </div>
          </section>

          <section className={styles.cardDanger}>
            <div className={styles.actionRow}>
              <div>
                <h3 className={styles.dangerTitle}>Danger zone</h3>
                <p className={styles.muted}>
                  Deleting a project removes its tasks, relations, sessions, and knowledge. This
                  cannot be undone.
                </p>
              </div>
              <Button variant="danger" onClick={() => setDeleteOpen(true)}>
                Delete project
              </Button>
            </div>
          </section>
        </div>
      </div>

      <ConfirmDialog
        open={deleteOpen}
        title="Delete project"
        confirmLabel="Delete project"
        danger
        busy={deleteProject.isPending}
        onConfirm={onDelete}
        onCancel={() => setDeleteOpen(false)}
      >
        Delete “{name}” and everything in it? This cannot be undone.
      </ConfirmDialog>
    </section>
  );
}
