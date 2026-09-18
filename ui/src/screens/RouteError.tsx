import { useTranslation } from "react-i18next";
import { Link, isRouteErrorResponse, useRouteError } from "react-router";
import styles from "../components/states.module.css";

/**
 * Router-level error boundary (`errorElement` on the root route): a
 * render-time crash in any screen falls back to this instead of a blank
 * page. The toast system lives inside the layout, so without a boundary a
 * single bad render would unmount the whole app — including the UI that
 * would have explained the problem.
 */
export function RouteError() {
  const { t } = useTranslation();
  const error = useRouteError();
  const detail = isRouteErrorResponse(error)
    ? `${error.status} ${error.statusText}`
    : error instanceof Error
      ? error.message
      : t("common.error.unexpected");
  return (
    <div className={styles.state} role="alert">
      <p className={styles.errorTitle}>{t("common.error.title")}</p>
      <p className={styles.errorDetail}>{detail}</p>
      <p>
        <Link to="/">{t("common.action.backHome")}</Link>
      </p>
    </div>
  );
}
