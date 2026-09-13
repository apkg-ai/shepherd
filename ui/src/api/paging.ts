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
  /** The infinite-query data; `pageParams` drives the stuck-cursor guard. */
  data?: { pageParams?: readonly unknown[] };
}

/**
 * Auto-drains an infinite query: whole-collection consumers (the graph's
 * task/relation feeds, knowledge search) need every page, not a Load-more
 * button. Pair with `limit: 100` to keep the round-trips few. Stops on
 * error — the screen's error state takes over.
 *
 * Also stops when the cursor stops advancing (the last two page params are
 * identical): a server-side paging bug would otherwise turn the drain into
 * an unbounded request loop.
 */
export function useAllPages(query: DrainableQuery): void {
  const { hasNextPage, isFetchingNextPage, isError, fetchNextPage, data } = query;
  const pageParams = data?.pageParams;
  useEffect(() => {
    if (!hasNextPage || isFetchingNextPage || isError) return;
    if (
      pageParams !== undefined &&
      pageParams.length >= 2 &&
      JSON.stringify(pageParams.at(-1)) === JSON.stringify(pageParams.at(-2))
    ) {
      return;
    }
    void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, isError, fetchNextPage, pageParams]);
}
