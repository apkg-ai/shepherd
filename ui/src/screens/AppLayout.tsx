import { Outlet, useParams } from "react-router";
import { useGetProject } from "../api/generated/projects/projects";
import { ErrorState, LoadingState } from "../components/states";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const { projectId } = useParams<{ projectId: string }>();
  const project = useGetProject(projectId!);

  if (project.isPending) return <LoadingState />;
  if (project.isError) {
    return <ErrorState error={project.error} onRetry={() => void project.refetch()} />;
  }

  return (
    <div className={styles.layout}>
      {/* SSE subscription will be wired in a future step */}
      <Outlet />
    </div>
  );
}
