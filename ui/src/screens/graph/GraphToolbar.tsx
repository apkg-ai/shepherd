import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import styles from "./GraphToolbar.module.css";

interface GraphToolbarProps {
  viewMode: "graph" | "list" | "table";
  hasSelection: boolean;
  hideCompleted?: boolean;
  onFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetLayout: () => void;
  onToggleView: () => void;
  onToggleTable?: () => void;
  onAddDependency: () => void;
  onToggleHideCompleted?: () => void;
}

export function GraphToolbar({
  viewMode,
  hasSelection,
  hideCompleted,
  onFit,
  onZoomIn,
  onZoomOut,
  onResetLayout,
  onToggleView,
  onToggleTable,
  onAddDependency,
  onToggleHideCompleted,
}: GraphToolbarProps) {
  const { t } = useTranslation();

  return (
    <div role="group" aria-label={t("graph.toolbar.label")} className={styles.toolbar}>
      {viewMode === "graph" && (
        <>
          <Button variant="ghost" onClick={onFit}>
            {t("graph.action.fit")}
          </Button>
          <Button variant="ghost" onClick={onZoomIn}>
            {t("graph.action.zoomIn")}
          </Button>
          <Button variant="ghost" onClick={onZoomOut}>
            {t("graph.action.zoomOut")}
          </Button>
          <Button variant="ghost" onClick={onResetLayout}>
            {t("graph.action.resetLayout")}
          </Button>
          <Button variant="ghost" onClick={onAddDependency} disabled={!hasSelection}>
            {t("graph.action.addDependency")}
          </Button>
        </>
      )}
      {onToggleHideCompleted && (
        <Button variant="ghost" onClick={onToggleHideCompleted} aria-pressed={hideCompleted}>
          {hideCompleted ? t("graph.action.showCompleted") : t("graph.action.hideCompleted")}
        </Button>
      )}
      <div className={styles.spacer} />
      {onToggleTable && viewMode !== "list" && (
        <Button variant="ghost" onClick={onToggleTable}>
          {viewMode === "graph" ? t("graph.action.tabsView") : t("graph.action.graphView")}
        </Button>
      )}
      {!onToggleTable && (
        <Button variant="ghost" onClick={onToggleView}>
          {viewMode === "graph" ? t("graph.action.listView") : t("graph.action.graphView")}
        </Button>
      )}
    </div>
  );
}
