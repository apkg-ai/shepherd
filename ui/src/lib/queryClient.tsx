import { MutationCache, QueryCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { isShepherdError, errorSlug } from "../api/problem";
import { setCsrfToken } from "../api/client";
import { useToast } from "../components/Toast";

interface QueryClientOptions {
  onMutationError?: (message: string) => void;
  onAuthExpired?: () => void;
}

export function createQueryClient(options?: QueryClientOptions): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        // storage_busy is already retried at the transport layer (client.ts);
        // retrying here too would multiply attempts.
        retry: (failureCount, error) =>
          !(
            isShepherdError(error) &&
            (error.status < 500 || errorSlug(error) === "storage_busy")
          ) && failureCount < 2,
        staleTime: 5_000,
        refetchOnWindowFocus: true,
      },
      mutations: { retry: false },
    },
    queryCache: new QueryCache({
      onError: (error) => {
        if (isShepherdError(error) && error.status === 401) {
          options?.onAuthExpired?.();
        }
      },
    }),
    mutationCache: new MutationCache({
      onError: (error, _variables, _context, mutation) => {
        if (isShepherdError(error) && error.status === 401) {
          options?.onAuthExpired?.();
          return;
        }
        if (mutation.meta?.silent === true) return;
        options?.onMutationError?.(
          isShepherdError(error) ? (error.detail ?? error.title) : "Request failed",
        );
      },
    }),
  });
}

export function AppQueryProvider({ children }: { children: ReactNode }) {
  const { toast } = useToast();
  const [client] = useState(() =>
    createQueryClient({
      onMutationError: (message) => toast(message, "error"),
      onAuthExpired: () => {
        setCsrfToken(undefined);
        window.location.hash = "#/login";
      },
    }),
  );
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
