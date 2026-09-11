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
