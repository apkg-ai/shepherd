import { useRef, type MouseEvent } from "react";
import { Link, Outlet } from "react-router";
import { ThemeToggle } from "../components/ThemeToggle";
import styles from "./AppShell.module.css";

export function AppShell() {
  const mainRef = useRef<HTMLElement>(null);

  const skipToContent = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    mainRef.current?.focus();
  };

  return (
    <div className={styles.shell}>
      <a href="#main" className={styles.skipLink} onClick={skipToContent}>
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
      <main id="main" ref={mainRef} tabIndex={-1} className={styles.main}>
        <Outlet />
      </main>
    </div>
  );
}
