import { QueryClientProvider } from "@tanstack/react-query";
import { render, type RenderResult } from "@testing-library/react";
import type { RequestHandler } from "msw";
import { useState, type ReactNode } from "react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { ToastProvider, useToast } from "../components/Toast";
import { createQueryClient } from "../lib/queryClient";
import { routes } from "../router";
import { server } from "./msw";

/**
 * Same provider stack as main.tsx (toast-wired mutation errors included),
 * with query retries disabled so error-path tests don't sit through backoff.
 */
function TestQueryProvider({ children }: { children: ReactNode }) {
  const { toast } = useToast();
  const [client] = useState(() => {
    const queryClient = createQueryClient((message) => toast(message, "error"));
    queryClient.setDefaultOptions({
      queries: {
        ...queryClient.getDefaultOptions().queries,
        retry: false,
        gcTime: 0,
      },
      mutations: { retry: false },
    });
    return queryClient;
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/**
 * Renders the real app (route table, providers) at `path` through a memory
 * router, so tests exercise routing, params, and search params for real.
 * `handlers` are pushed onto the MSW server first and win over the generated
 * background handlers.
 */
export function renderRoute(
  path: string,
  { handlers = [] }: { handlers?: RequestHandler[] } = {},
): RenderResult & { router: ReturnType<typeof createMemoryRouter> } {
  if (handlers.length > 0) server.use(...handlers);
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  const result = render(
    <ToastProvider>
      <TestQueryProvider>
        <RouterProvider router={router} />
      </TestQueryProvider>
    </ToastProvider>,
  );
  return { ...result, router };
}
