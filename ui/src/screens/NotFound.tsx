import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { EmptyState } from "../components/states";

export function NotFound() {
  const { t } = useTranslation();
  return (
    <EmptyState title={t("not-found.screen.title")}>
      <Link to="/">{t("common.action.backHome")}</Link>
    </EmptyState>
  );
}
