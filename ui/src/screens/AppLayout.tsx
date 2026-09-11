import { Link, NavLink, Outlet, useParams } from "react-router";
import { useGetProject, useListProjects } from "../api/generated/projects/projects";
import { useListTasks } from "../api/generated/tasks/tasks";
import { ThemeToggle } from "../components/ThemeToggle";
import styles from "./AppLayout.module.css";

function ProjectSwitcher({ activeProjectId }: { activeProjectId?: string }) {
  // First page is plenty for a local tool's sidebar; the registry screen
  // remains the full, paginated list.
  const { data } = useListProjects();
  const projects = data?.items ?? [];
  return (
    <nav aria-label="Projects" className={styles.switcher}>
      <div className={styles.sectionLabel}>Projects</div>
      <ul className={styles.switcherList}>
        {projects.map((project) => (
          <li key={project.id}>
            <Link
              to={`/projects/${project.id}`}
              className={
                project.id === activeProjectId ? styles.switcherLinkActive : styles.switcherLink
              }
              aria-current={project.id === activeProjectId ? "true" : undefined}
            >
              {project.name}
            </Link>
          </li>
        ))}
      </ul>
      <Link to="/" className={styles.allProjects}>
        All projects
      </Link>
    </nav>
  );
}

/**
 * Everything waiting on a human: work in review plus agent proposals.
 * First-page honest — cursor pagination can't give exact totals, so past
 * one page the count gets a "+". Query keys match the review screen's tab
 * counts, so both refresh together off the existing task invalidations.
 */
function ReviewBadge({ projectId }: { projectId: string }) {
  const inReview = useListTasks(projectId, { status: "in_review", limit: 25 });
  const proposed = useListTasks(projectId, { status: "proposed", limit: 25 });
  const count = (inReview.data?.items.length ?? 0) + (proposed.data?.items.length ?? 0);
  if (count === 0) return null;
  const hedged = inReview.data?.has_more === true || proposed.data?.has_more === true;
  return (
    <>
      <span className={styles.badge} aria-hidden="true">
        {count}
        {hedged ? "+" : ""}
      </span>
      <span className="sr-only">
        , {count}
        {hedged ? " or more" : ""} waiting
      </span>
    </>
  );
}

function ProjectNav({ projectId }: { projectId: string }) {
  const { data: project } = useGetProject(projectId);
  const linkClass = ({ isActive }: { isActive: boolean }) =>
    isActive ? styles.navLinkActive : styles.navLink;
  return (
    <nav className={styles.projectNav} aria-label="Project">
      <div className={styles.projectName}>{project?.name ?? "…"}</div>
      <NavLink to={`/projects/${projectId}`} end className={linkClass}>
        Tasks
      </NavLink>
      <NavLink to={`/projects/${projectId}/review`} className={linkClass}>
        Review
        <ReviewBadge projectId={projectId} />
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
    <div className={styles.shell}>
      <aside className={styles.sidebar}>
        <h1 className={styles.brand}>
          <Link to="/" className={styles.brandLink}>
            Shepherd
          </Link>
        </h1>
        <ProjectSwitcher activeProjectId={projectId} />
        {projectId ? <ProjectNav projectId={projectId} /> : null}
        <div className={styles.sidebarFooter}>
          <ThemeToggle />
        </div>
      </aside>
      <main className={styles.main}>
        <Outlet />
      </main>
    </div>
  );
}
