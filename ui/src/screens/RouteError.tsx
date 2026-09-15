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
  const error = useRouteError();
  const detail = isRouteErrorResponse(error)
    ? `${error.status} ${error.statusText}`
    : error instanceof Error
      ? error.message
      : "Unexpected error";
  return (
    <div className={styles.state} role="alert">
      <p className={styles.errorTitle}>Something went wrong</p>
      <p className={styles.errorDetail}>{detail}</p>
      <p>
        <Link to="/">Back home</Link>
      </p>
    </div>
  );
}
