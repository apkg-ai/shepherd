import { useQueryClient } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ShepherdError } from "../api/problem";
import { ToastProvider } from "../components/Toast";
import { AppQueryProvider, createQueryClient } from "./queryClient";

function shepherd(status: number): ShepherdError {
  return new ShepherdError({
    type: "urn:shepherd:error:test-error",
    title: "Test",
    status,
  });
}

describe("createQueryClient", () => {
  const retry = createQueryClient().getDefaultOptions().queries?.retry as (
    failureCount: number,
    error: unknown,
  ) => boolean;

  it("never retries problem+json client errors", () => {
    expect(retry(0, shepherd(404))).toBe(false);
    expect(retry(0, shepherd(409))).toBe(false);
    expect(retry(0, shepherd(422))).toBe(false);
  });

  it("retries server errors and unknown failures twice", () => {
    expect(retry(0, shepherd(500))).toBe(true);
    expect(retry(1, new TypeError("network down"))).toBe(true);
    expect(retry(2, shepherd(500))).toBe(false);
  });

  it("reports mutation errors unless the mutation opts out", async () => {
    const onError = vi.fn();
    const client = createQueryClient(onError);

    await client
      .getMutationCache()
      .build(client, {
        mutationFn: () => Promise.reject(shepherd(409)),
      })
      .execute(undefined)
      .catch(() => {});
    expect(onError).toHaveBeenCalledWith("Test");

    onError.mockClear();
    await client
      .getMutationCache()
      .build(client, {
        mutationFn: () => Promise.reject(shepherd(422)),
        meta: { silent: true },
      })
      .execute(undefined)
      .catch(() => {});
    expect(onError).not.toHaveBeenCalled();

    await client
      .getMutationCache()
      .build(client, {
        mutationFn: () => Promise.reject(new Error("plain failure")),
      })
      .execute(undefined)
      .catch(() => {});
    expect(onError).toHaveBeenCalledWith("Request failed");
  });
});

describe("AppQueryProvider", () => {
  it("provides a query client wired to the toast stack", async () => {
    function Probe() {
      const client = useQueryClient();
      return <p>{client ? "has client" : "no client"}</p>;
    }
    render(
      <ToastProvider>
        <AppQueryProvider>
          <Probe />
        </AppQueryProvider>
      </ToastProvider>,
    );
    await waitFor(() => expect(screen.getByText("has client")).toBeInTheDocument());
  });
});
