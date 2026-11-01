import { useTranslation } from "react-i18next";
import styles from "./GraphToolbar.module.css";

interface GraphToolbarProps {
  onFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetLayout: () => void;
  onAddDependency: () => void;
  hasSelection: boolean;
}

export function GraphToolbar({
  onFit,
  onZoomIn,
  onZoomOut,
  onResetLayout,
  onAddDependency,
  hasSelection,
}: GraphToolbarProps) {
  const { t } = useTranslation();

  return (
    <div role="group" aria-label={t("graph.toolbar.label")} className={styles.toolbar}>
      <button
        className={styles.iconBtn}
        onClick={onZoomIn}
        aria-label={t("graph.action.zoomIn")}
        title={t("graph.action.zoomIn")}
      >
        ⊕
      </button>
      <button
        className={styles.iconBtn}
        onClick={onZoomOut}
        aria-label={t("graph.action.zoomOut")}
        title={t("graph.action.zoomOut")}
      >
        ⊖
      </button>
      <button
        className={styles.iconBtn}
        onClick={onFit}
        aria-label={t("graph.action.fit")}
        title={t("graph.action.fit")}
      >
        ⊡
      </button>
      <button
        className={styles.iconBtn}
        onClick={onResetLayout}
        aria-label={t("graph.action.resetLayout")}
        title={t("graph.action.resetLayout")}
      >
        ↺
      </button>
      <div className={styles.separator} />
      <button
        className={styles.iconBtn}
        onClick={onAddDependency}
        disabled={!hasSelection}
        aria-label={t("graph.action.addDependency")}
        title={t("graph.action.addDependency")}
      >
        +
      </button>
    </div>
  );
}
