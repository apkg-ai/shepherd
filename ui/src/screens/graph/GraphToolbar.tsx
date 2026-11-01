import { useTranslation } from "react-i18next";
import styles from "./GraphToolbar.module.css";

interface GraphToolbarProps {
  onFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetLayout?: () => void;
  onAddDependency: () => void;
  hasSelection: boolean;
  viewMode?: "graph" | "list" | "table";
  onToggleView?: () => void;
  onToggleTable?: () => void;
  hideCompleted?: boolean;
  onToggleHideCompleted?: () => void;
  floating?: boolean;
}

export function GraphToolbar({
  onFit,
  onZoomIn,
  onZoomOut,
  onResetLayout,
  onAddDependency,
  hasSelection,
  viewMode,
  onToggleView,
  onToggleTable,
  hideCompleted,
  onToggleHideCompleted,
  floating,
}: GraphToolbarProps) {
  const { t } = useTranslation();

  const hasViewControls = onToggleView || onToggleTable || onToggleHideCompleted;

  return (
    <div
      role="group"
      aria-label={t("graph.toolbar.label")}
      className={`${styles.toolbar} ${floating ? styles.floating : ""}`}
    >
      {floating && (
        <>
          <button
            type="button"
            className={styles.iconBtn}
            onClick={onZoomIn}
            aria-label={t("graph.action.zoomIn")}
            title={t("graph.action.zoomIn")}
          >
            ⊕
          </button>
          <button
            type="button"
            className={styles.iconBtn}
            onClick={onZoomOut}
            aria-label={t("graph.action.zoomOut")}
            title={t("graph.action.zoomOut")}
          >
            ⊖
          </button>
          <button
            type="button"
            className={styles.iconBtn}
            onClick={onFit}
            aria-label={t("graph.action.fit")}
            title={t("graph.action.fit")}
          >
            ⊡
          </button>
          {onResetLayout && (
            <button
              type="button"
              className={styles.iconBtn}
              onClick={onResetLayout}
              aria-label={t("graph.action.resetLayout")}
              title={t("graph.action.resetLayout")}
            >
              ↺
            </button>
          )}
          <div className={styles.separator} />
          <button
            type="button"
            className={styles.iconBtn}
            onClick={onAddDependency}
            disabled={!hasSelection}
            aria-label={t("graph.action.addDependency")}
            title={t("graph.action.addDependency")}
          >
            +
          </button>
        </>
      )}
      {hasViewControls && (
        <>
          {floating && <div className={styles.separator} />}
          {onToggleView && (
            <button
              type="button"
              className={styles.iconBtn}
              onClick={onToggleView}
              aria-label={
                viewMode === "list" ? t("graph.action.graphView") : t("graph.action.listView")
              }
              title={viewMode === "list" ? t("graph.action.graphView") : t("graph.action.listView")}
            >
              {viewMode === "list" ? "⊞" : "☰"}
            </button>
          )}
          {onToggleTable && (
            <button
              type="button"
              className={styles.iconBtn}
              onClick={onToggleTable}
              aria-label={
                viewMode === "table" ? t("graph.action.graphView") : t("graph.action.tabsView")
              }
              title={
                viewMode === "table" ? t("graph.action.graphView") : t("graph.action.tabsView")
              }
            >
              {viewMode === "table" ? "⊞" : "▤"}
            </button>
          )}
          {onToggleHideCompleted && (
            <button
              type="button"
              className={styles.iconBtn}
              onClick={onToggleHideCompleted}
              aria-label={
                hideCompleted ? t("graph.action.showCompleted") : t("graph.action.hideCompleted")
              }
              title={
                hideCompleted ? t("graph.action.showCompleted") : t("graph.action.hideCompleted")
              }
              aria-pressed={hideCompleted}
            >
              ✓
            </button>
          )}
        </>
      )}
    </div>
  );
}
