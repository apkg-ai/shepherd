import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { flattenPages, useAllPages } from "./paging";

/** Minimal infinite-query result shaped like TanStack's UseInfiniteQueryResult. */
function makeQuery(overrides: Partial<Parameters<typeof useAllPages>[0]>) {
  return {
    hasNextPage: false,
    isFetchingNextPage: false,
    isError: false,
    fetchNextPage: vi.fn(),
    ...overrides,
  };
}

describe("useAllPages", () => {
  it("drains while pages remain", () => {
    const fetchNextPage = vi.fn();
    const { rerender } = renderHook(
      ({ query }: { query: Parameters<typeof useAllPages>[0] }) => useAllPages(query),
      { initialProps: { query: makeQuery({ hasNextPage: true, fetchNextPage }) } },
    );

    expect(fetchNextPage).toHaveBeenCalledTimes(1);

    // A fetch completes: still more pages → drain continues.
    rerender({
      query: makeQuery({
        hasNextPage: true,
        fetchNextPage,
        data: { pageParams: [undefined, "cursor-1"] },
      }),
    });
    expect(fetchNextPage).toHaveBeenCalledTimes(2);
  });

  it("stops when no next page or a fetch is in flight", () => {
    const fetchNextPage = vi.fn();
    const { rerender } = renderHook(
      ({ query }: { query: Parameters<typeof useAllPages>[0] }) => useAllPages(query),
      { initialProps: { query: makeQuery({ hasNextPage: true, fetchNextPage }) } },
    );

    rerender({ query: makeQuery({ hasNextPage: false, fetchNextPage }) });
    rerender({
      query: makeQuery({ hasNextPage: true, isFetchingNextPage: true, fetchNextPage }),
    });
    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("stops on error — the screen's error state takes over", () => {
    const fetchNextPage = vi.fn();
    const { rerender } = renderHook(
      ({ query }: { query: Parameters<typeof useAllPages>[0] }) => useAllPages(query),
      { initialProps: { query: makeQuery({ hasNextPage: true, fetchNextPage }) } },
    );

    rerender({ query: makeQuery({ hasNextPage: true, isError: true, fetchNextPage }) });
    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("stops when the cursor stops advancing (repeated page param)", () => {
    const fetchNextPage = vi.fn();
    const { rerender } = renderHook(
      ({ query }: { query: Parameters<typeof useAllPages>[0] }) => useAllPages(query),
      { initialProps: { query: makeQuery({ hasNextPage: true, fetchNextPage }) } },
    );

    // Page 2 returns the same cursor as page 1 — the drain must not loop.
    rerender({
      query: makeQuery({
        hasNextPage: true,
        fetchNextPage,
        data: { pageParams: [undefined, "cursor-1", "cursor-1"] },
      }),
    });
    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("continues after a background refetch replays the same params", () => {
    const fetchNextPage = vi.fn();
    const { rerender } = renderHook(
      ({ query }: { query: Parameters<typeof useAllPages>[0] }) => useAllPages(query),
      { initialProps: { query: makeQuery({ hasNextPage: true, fetchNextPage }) } },
    );

    // Drain page 2, then an invalidation refetches the same pages — the
    // last two params are distinct, so the drain must continue.
    rerender({
      query: makeQuery({
        hasNextPage: true,
        fetchNextPage,
        data: { pageParams: [undefined, "cursor-1"] },
      }),
    });
    rerender({
      query: makeQuery({
        hasNextPage: true,
        fetchNextPage,
        data: { pageParams: [undefined, "cursor-1"] },
      }),
    });
    expect(fetchNextPage).toHaveBeenCalledTimes(3);
  });
});

describe("flattenPages", () => {
  it("flattens pages and defaults to empty", () => {
    expect(flattenPages([{ items: [{ id: "a" }, { id: "b" }] }, { items: [{ id: "c" }] }])).toEqual(
      [{ id: "a" }, { id: "b" }, { id: "c" }],
    );
    expect(flattenPages(undefined)).toEqual([]);
  });
});
