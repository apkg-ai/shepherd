import type { ReactNode } from "react";
import { Link } from "react-router";
import styles from "./CardGrid.module.css";

export function CardGrid({ children }: { children: ReactNode }) {
  return <ul className={styles.grid}>{children}</ul>;
}

interface CardProps {
  to: string;
  title: string;
  description?: string;
  meta?: string;
  /** Inline badge next to the title (single badge, short content). */
  badge?: ReactNode;
  /** Tag row above the title (multiple badges, always on its own line). */
  tags?: ReactNode;
  /** Sets data-status on the card for status-colored left border. */
  status?: string;
}

export function Card({ to, title, description, meta, badge, tags, status }: CardProps) {
  return (
    <li className={styles.card} data-status={status}>
      <Link to={to} className={styles.cardLink}>
        {tags ? <div className={styles.cardTags}>{tags}</div> : null}
        <div className={styles.cardHeader}>
          <span className={styles.cardTitle}>{title}</span>
          {badge ? <span className={styles.cardBadge}>{badge}</span> : null}
        </div>
        {description ? <span className={styles.cardDescription}>{description}</span> : null}
        {meta ? <span className={styles.cardMeta}>{meta}</span> : null}
      </Link>
    </li>
  );
}

interface StaticCardProps {
  title: string;
  meta?: string;
  badge?: ReactNode;
  children?: ReactNode;
}

export function StaticCard({ title, meta, badge, children }: StaticCardProps) {
  return (
    <li className={styles.card}>
      <div className={styles.cardContent}>
        <div className={styles.cardHeader}>
          <span className={styles.cardTitle}>{title}</span>
          {badge ? <span className={styles.cardBadge}>{badge}</span> : null}
        </div>
        {children}
        {meta ? <span className={styles.cardMeta}>{meta}</span> : null}
      </div>
    </li>
  );
}
