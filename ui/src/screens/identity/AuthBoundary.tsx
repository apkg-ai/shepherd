import { useEffect } from "react";
import { Navigate, Outlet } from "react-router";
import { useGetBrowserSession } from "../../api/generated/identity/identity";
import { isShepherdError } from "../../api/problem";
import { setCsrfToken } from "../../api/client";
import { ErrorState, LoadingState } from "../../components/states";

export function AuthBoundary() {
  const session = useGetBrowserSession();

  useEffect(() => {
    if (session.data) {
      setCsrfToken(session.data.csrf_token);
    }
    return () => setCsrfToken(undefined);
  }, [session.data]);

  if (session.isPending) return <LoadingState />;

  // A failed refetch with cached data (e.g. a transient error on tab focus)
  // must not unmount the app and lose in-progress form state.
  if (session.isError) {
    if (isShepherdError(session.error) && session.error.status === 401) {
      return <Navigate to="/login" replace />;
    }
    if (!session.data) {
      return <ErrorState error={session.error} onRetry={() => void session.refetch()} />;
    }
  }

  return <Outlet />;
}
