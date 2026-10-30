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
  badge?: ReactNode;
}

export function Card({ to, title, description, meta, badge }: CardProps) {
  return (
    <li className={styles.card}>
      <Link to={to} className={styles.cardLink}>
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
