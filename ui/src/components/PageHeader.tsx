import type { ReactNode } from "react";
import styles from "./PageHeader.module.css";

interface PageHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  /** Right-aligned action buttons/links. */
  actions?: ReactNode;
  /** Slot under the title row (filters, tabs, badges). */
  children?: ReactNode;
}

export function PageHeader({ title, actions, children }: PageHeaderProps) {
  return (
    <header className={styles.header}>
      <div className={styles.titleRow}>
        <h2 className={styles.title}>{title}</h2>
        {actions ? <div className={styles.actions}>{actions}</div> : null}
      </div>
      {children}
    </header>
  );
}
