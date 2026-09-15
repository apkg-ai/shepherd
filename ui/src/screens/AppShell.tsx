import { Link, Outlet } from "react-router";
import { ThemeToggle } from "../components/ThemeToggle";
import styles from "./AppShell.module.css";

/**
 * Application shell (v1 step 000): brand header, theme toggle, and the
 * routed content region. Later steps grow navigation here.
 */
export function AppShell() {
  return (
    <div className={styles.shell}>
      <a href="#main" className={styles.skipLink}>
        Skip to content
      </a>
      <header className={styles.header}>
        <h1 className={styles.brand}>
          <Link to="/" className={styles.brandLink}>
            Shepherd
          </Link>
        </h1>
        <ThemeToggle />
      </header>
      <main id="main" className={styles.main}>
        <Outlet />
      </main>
    </div>
  );
}
