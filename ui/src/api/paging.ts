import { useEffect } from "react";

/**
 * Cursor-paging glue for the generated `use*Infinite` hooks: the generated
 * queryFn feeds `pageParam` into the spec's `cursor` param, but TanStack
 * Query v5 needs `initialPageParam`/`getNextPageParam` from the caller.
 */
interface CursorPage {
  has_more: boolean;
  next_cursor?: string | null;
}

export function cursorPaging<TPage extends CursorPage>() {
  return {
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage: TPage) =>
      lastPage.has_more ? (lastPage.next_cursor ?? undefined) : undefined,
  };
}

/** Flattens infinite-query pages into a single item list. */
export function flattenPages<TItem>(pages: { items: TItem[] }[] | undefined): TItem[] {
  return pages?.flatMap((page) => page.items) ?? [];
}

interface DrainableQuery {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  isError: boolean;
  fetchNextPage: () => unknown;
}

/**
 * Auto-drains an infinite query: whole-collection consumers (the graph's
 * task/relation feeds, knowledge search) need every page, not a Load-more
 * button. Pair with `limit: 100` to keep the round-trips few. Stops on
 * error — the screen's error state takes over.
 */
export function useAllPages(query: DrainableQuery): void {
  const { hasNextPage, isFetchingNextPage, isError, fetchNextPage } = query;
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && !isError) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, isError, fetchNextPage]);
}
