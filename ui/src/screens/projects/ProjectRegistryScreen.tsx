import { useState } from "react";
import { Link } from "react-router";
import { useListProjectsInfinite } from "../../api/generated/projects/projects";
import { cursorPaging, flattenPages } from "../../api/paging";
import { Button } from "../../components/Button";
import { LoadMore } from "../../components/LoadMore";
import { EmptyState, ErrorState, LoadingState } from "../../components/states";
import { formatDateTime } from "../../lib/format";
import { PageHeader } from "../../components/PageHeader";
import { ImportProjectDialog } from "./ImportProjectDialog";
import { ProjectCreateDialog } from "./ProjectCreateDialog";
import styles from "./ProjectRegistryScreen.module.css";

export function ProjectRegistryScreen() {
  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const query = useListProjectsInfinite(undefined, {
    query: cursorPaging(),
  });

  const projects = flattenPages(query.data?.pages);

  return (
    <section>
      <PageHeader
        title="Projects"
        description="Each project is a typed task graph shared between you and your agents."
        actions={
          <>
            <Button onClick={() => setImportOpen(true)}>Import</Button>
            <Button variant="primary" onClick={() => setCreateOpen(true)}>
              Register project
            </Button>
          </>
        }
      />

      {query.isPending ? (
        <LoadingState label="Loading projects…" />
      ) : query.isError ? (
        <ErrorState error={query.error} onRetry={() => void query.refetch()} />
      ) : projects.length === 0 ? (
        <EmptyState title="Register your first project">
          <p>Projects map work as typed task graphs shared with agents.</p>
        </EmptyState>
      ) : (
        <>
          <div className={styles.card}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Description</th>
                  <th>Review gate</th>
                  <th>Created</th>
                </tr>
              </thead>
              <tbody>
                {projects.map((project) => (
                  <tr key={project.id}>
                    <td>
                      <Link to={`/projects/${project.id}`}>{project.name}</Link>
                    </td>
                    <td className={styles.description}>{project.description}</td>
                    <td>{project.settings.review_gate ? "On" : "Off"}</td>
                    <td>{formatDateTime(project.created_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <LoadMore
            hasNextPage={query.hasNextPage}
            isFetchingNextPage={query.isFetchingNextPage}
            onLoadMore={() => void query.fetchNextPage()}
          />
        </>
      )}

      <ProjectCreateDialog open={createOpen} onClose={() => setCreateOpen(false)} />
      <ImportProjectDialog open={importOpen} onClose={() => setImportOpen(false)} />
    </section>
  );
}
