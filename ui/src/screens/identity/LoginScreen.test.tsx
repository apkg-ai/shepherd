import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { makeBrowserSession } from "../../test/fixtures";
import { problemResponse, sessionHandler, projectListHandler } from "../../test/msw";
import { renderRoute } from "../../test/test-utils";

describe("LoginScreen", () => {
  it("renders the sign-in form with title and password input", () => {
    renderRoute("/login");

    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByLabelText("Owner token")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });

  it("disables submit when the token field is empty", () => {
    renderRoute("/login");

    expect(screen.getByRole("button", { name: "Sign in" })).toBeDisabled();
  });

  it("navigates to / on successful login", async () => {
    const user = userEvent.setup();
    const { router } = renderRoute("/login", {
      handlers: [
        http.post("/api/v1/session", () =>
          HttpResponse.json(makeBrowserSession(), { status: 201 }),
        ),
        sessionHandler(),
        projectListHandler([]),
      ],
    });

    await user.type(screen.getByLabelText("Owner token"), "valid-token");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/"));
  });

  it("shows an invalid token error on 401", async () => {
    const user = userEvent.setup();
    renderRoute("/login", {
      handlers: [
        http.post("/api/v1/session", () =>
          problemResponse(401, "unauthenticated", { title: "Invalid token" }),
        ),
      ],
    });

    await user.type(screen.getByLabelText("Owner token"), "bad-token");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await screen.findByText("Invalid or expired token.")).toBeInTheDocument();
  });

  it("shows a capability message on 403", async () => {
    const user = userEvent.setup();
    renderRoute("/login", {
      handlers: [
        http.post("/api/v1/session", () =>
          problemResponse(403, "forbidden", { title: "Forbidden" }),
        ),
      ],
    });

    await user.type(screen.getByLabelText("Owner token"), "agent-token");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(
      await screen.findByText("This token does not have browser session capability."),
    ).toBeInTheDocument();
  });

  it("clears the token field after a failed attempt", async () => {
    const user = userEvent.setup();
    renderRoute("/login", {
      handlers: [http.post("/api/v1/session", () => problemResponse(401, "unauthenticated"))],
    });

    const input = screen.getByLabelText("Owner token") as HTMLInputElement;
    await user.type(input, "bad-token");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    await screen.findByText("Invalid or expired token.");
    expect(input.value).toBe("");
  });
});
