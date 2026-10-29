import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { sessionHandler, projectListHandler, problemResponse } from "../test/msw";
import { renderRoute } from "../test/test-utils";

describe("SignOutButton", () => {
  it("is hidden when no session exists", () => {
    renderRoute("/login");

    expect(screen.queryByRole("button", { name: "Sign out" })).not.toBeInTheDocument();
  });

  it("signs out, clears cached data, and navigates to login", async () => {
    const user = userEvent.setup();
    let deleted = false;
    const { router } = renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.delete("/api/v1/session", () => {
          deleted = true;
          return new HttpResponse(null, { status: 204 });
        }),
      ],
    });

    await screen.findByText("No projects yet");
    await user.click(screen.getByRole("button", { name: "Sign out" }));

    expect(deleted).toBe(true);
    await waitFor(() => expect(router.state.location.pathname).toBe("/login"));
  });

  it("keeps the session when sign-out fails", async () => {
    const user = userEvent.setup();
    const { router } = renderRoute("/", {
      handlers: [
        sessionHandler(),
        projectListHandler([]),
        http.delete("/api/v1/session", () =>
          problemResponse(500, "internal-error", { title: "Server Error" }),
        ),
      ],
    });

    await screen.findByText("No projects yet");
    await user.click(screen.getByRole("button", { name: "Sign out" }));

    expect(await screen.findByText("Server Error")).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/");
    expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
  });
});
