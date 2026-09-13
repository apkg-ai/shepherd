import { MutationCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { isShepherdError } from "../api/problem";
import { useToast } from "../components/Toast";

/**
 * QueryClient defaults for the app and tests:
 * - queries retry only unexpected failures (5xx/network), never problem+json
 *   client errors like 404/409/422;
 * - liveness comes from the SSE subscription (lib/events.ts); refetch-on-
 *   focus stays on as belt-and-braces for screens outside a project scope;
 * - every mutation error is toasted unless the mutation opts out with
 *   `meta: { silent: true }` (forms that render 422s as field errors).
 */
export function createQueryClient(onMutationError?: (message: string) => void): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: (failureCount, error) =>
          !(isShepherdError(error) && error.status < 500) && failureCount < 2,
        staleTime: 5_000,
        refetchOnWindowFocus: true,
      },
      mutations: { retry: false },
    },
    mutationCache: new MutationCache({
      onError: (error, _variables, _context, mutation) => {
        if (mutation.meta?.silent === true) return;
        onMutationError?.(
          isShepherdError(error) ? (error.detail ?? error.title) : "Request failed",
        );
      },
    }),
  });
}

/** Wires the QueryClient's global mutation-error handler to the toast stack. */
export function AppQueryProvider({ children }: { children: ReactNode }) {
  const { toast } = useToast();
  const [client] = useState(() => createQueryClient((message) => toast(message, "error")));
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
