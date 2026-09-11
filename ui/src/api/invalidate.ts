import type { QueryClient } from "@tanstack/react-query";

/**
 * Invalidates every cached query whose key contains one of the given API
 * paths. Generated keys are path-based (`["/api/v1/projects"]`, with infinite
 * variants prefixed `["infinite", path, …]`), so exact-path membership hits
 * both flavors and all param variations at once.
 */
export function invalidatePaths(queryClient: QueryClient, ...paths: string[]): void {
  void queryClient.invalidateQueries({
    predicate: (query) =>
      query.queryKey.some((part) => typeof part === "string" && paths.includes(part)),
  });
}
