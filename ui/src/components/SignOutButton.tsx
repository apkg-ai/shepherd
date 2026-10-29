import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useDeleteBrowserSession, useGetBrowserSession } from "../api/generated/identity/identity";
import { setCsrfToken } from "../api/client";
import { Button } from "./Button";

export function SignOutButton() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  // Cache-only subscription: AuthBoundary owns the fetch lifecycle.
  const session = useGetBrowserSession({ query: { enabled: false } });

  const logout = useDeleteBrowserSession({
    mutation: {
      // Errors surface through the global mutation toast; the client-side
      // session state is only discarded once the server confirms sign-out.
      onSuccess: () => {
        setCsrfToken(undefined);
        queryClient.clear();
        void navigate("/login", { replace: true });
      },
    },
  });

  if (!session.data || session.isError) return null;

  return (
    <Button variant="ghost" busy={logout.isPending} onClick={() => logout.mutate(undefined)}>
      {t("auth.session.logout")}
    </Button>
  );
}
