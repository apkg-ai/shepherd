import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import styles from "./GraphToolbar.module.css";

interface GraphToolbarProps {
  viewMode: "graph" | "list";
  hasSelection: boolean;
  onFit: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onResetLayout: () => void;
  onToggleView: () => void;
  onAddDependency: () => void;
}

export function GraphToolbar({
  viewMode,
  hasSelection,
  onFit,
  onZoomIn,
  onZoomOut,
  onResetLayout,
  onToggleView,
  onAddDependency,
}: GraphToolbarProps) {
  const { t } = useTranslation();

  return (
    <div role="toolbar" aria-label={t("graph.toolbar.label")} className={styles.toolbar}>
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
      <div className={styles.spacer} />
      <Button variant="ghost" onClick={onToggleView}>
        {viewMode === "graph" ? t("graph.action.listView") : t("graph.action.graphView")}
      </Button>
    </div>
  );
}
