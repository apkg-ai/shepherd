import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { makeBrowserSession } from "../../test/fixtures";
import {
  sessionHandler,
  expiredSessionHandler,
  projectListHandler,
  problemResponse,
} from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("AuthBoundary", () => {
  it("renders children when the session is valid", async () => {
    renderRoute("/", {
      handlers: [sessionHandler(), projectListHandler([])],
    });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
  });

  it("redirects to /login on 401", async () => {
    const { router } = renderRoute("/", {
      handlers: [expiredSessionHandler()],
    });

    await waitFor(() => expect(router.state.location.pathname).toBe("/login"));
  });

  it("shows loading state while the session is pending", () => {
    renderRoute("/", {
      handlers: [http.get("/api/v1/session", () => new Promise(() => {}))],
    });

    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });

  it("shows an error state with retry for non-401 errors", async () => {
    const user = userEvent.setup();
    let failing = true;
    renderRoute("/", {
      handlers: [
        http.get("/api/v1/session", () => {
          if (failing)
            return problemResponse(500, "internal-error", { title: "Internal Server Error" });
          return undefined;
        }),
        sessionHandler(),
        projectListHandler([]),
      ],
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("Internal Server Error");
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();

    failing = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByText("No projects yet")).toBeInTheDocument());
  });

  it("keeps children mounted when a background session refetch fails", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      let sessionCalls = 0;
      renderRoute("/", {
        handlers: [
          http.get("/api/v1/session", () => {
            sessionCalls++;
            if (sessionCalls === 1) return HttpResponse.json(makeBrowserSession());
            return problemResponse(500, "internal-error", { title: "Transient error" });
          }),
          projectListHandler([]),
        ],
      });

      expect(await screen.findByText("No projects yet")).toBeInTheDocument();

      vi.advanceTimersByTime(6000); // past the 5s staleTime
      window.dispatchEvent(new Event("visibilitychange"));

      await waitFor(() => expect(sessionCalls).toBe(2));
      expect(screen.getByText("No projects yet")).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
