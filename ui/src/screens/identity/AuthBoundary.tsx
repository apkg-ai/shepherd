import { useEffect, useMemo } from "react";
import { Navigate, Outlet } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import {
  useGetBrowserSession,
  useDeleteBrowserSession,
  getGetBrowserSessionQueryKey,
} from "../../api/generated/identity/identity";
import { isShepherdError } from "../../api/problem";
import { setCsrfToken } from "../../api/client";
import { SessionContext, type SessionContextValue } from "../../api/session";
import { ErrorState, LoadingState } from "../../components/states";

export function AuthBoundary() {
  const queryClient = useQueryClient();
  const session = useGetBrowserSession();
  const logoutMutation = useDeleteBrowserSession();

  useEffect(() => {
    if (session.data) {
      setCsrfToken(session.data.csrf_token);
    }
    return () => setCsrfToken(undefined);
  }, [session.data]);

  const value = useMemo<SessionContextValue | null>(() => {
    if (!session.data) return null;
    return {
      session: session.data,
      logout: () => {
        logoutMutation.mutate(undefined, {
          onSettled: () => {
            setCsrfToken(undefined);
            queryClient.removeQueries({ queryKey: getGetBrowserSessionQueryKey() });
          },
        });
      },
    };
  }, [session.data, logoutMutation, queryClient]);

  if (session.isPending) return <LoadingState />;

  if (session.isError) {
    if (isShepherdError(session.error) && session.error.status === 401) {
      return <Navigate to="/login" replace />;
    }
    return <ErrorState error={session.error} onRetry={() => void session.refetch()} />;
  }

  if (!value) return null;

  return (
    <SessionContext value={value}>
      <Outlet />
    </SessionContext>
  );
}
