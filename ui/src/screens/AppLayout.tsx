import { Link, NavLink, Outlet, useParams } from "react-router";
import { useGetProject } from "../api/generated/projects/projects";
import styles from "./AppLayout.module.css";

function ProjectNav({ projectId }: { projectId: string }) {
  const { data: project } = useGetProject(projectId);
  const linkClass = ({ isActive }: { isActive: boolean }) =>
    isActive ? `${styles.navLink} ${styles.navLinkActive}` : styles.navLink;
  return (
    <nav className={styles.projectNav} aria-label="Project">
      <span className={styles.projectName}>{project?.name ?? "…"}</span>
      <NavLink to={`/projects/${projectId}`} end className={linkClass}>
        Tasks
      </NavLink>
      <NavLink to={`/projects/${projectId}/review`} className={linkClass}>
        Review
      </NavLink>
      <NavLink to={`/projects/${projectId}/settings`} className={linkClass}>
        Settings
      </NavLink>
    </nav>
  );
}

export function AppLayout() {
  const { projectId } = useParams();
  return (
    <>
      <header className={styles.topBar}>
        <h1 className={styles.brand}>
          <Link to="/" className={styles.brandLink}>
            Shepherd
          </Link>
        </h1>
        {projectId ? <ProjectNav projectId={projectId} /> : null}
      </header>
      <main className={styles.main}>
        <Outlet />
      </main>
    </>
  );
}
