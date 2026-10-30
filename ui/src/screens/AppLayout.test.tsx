import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { makeProject } from "../test/fixtures";
import { sessionHandler, projectHandler, goalListHandler, problemResponse } from "../test/msw";
import { renderRoute } from "../test/test-utils";
import { http } from "msw";

const project = makeProject({ id: "p1", name: "Space Game" });

describe("AppLayout", () => {
  it("renders child content inside the layout", async () => {
    renderRoute("/projects/p1", {
      handlers: [sessionHandler(), projectHandler(project), goalListHandler([])],
    });

    expect(await screen.findByRole("heading", { name: "Space Game" })).toBeInTheDocument();
  });

  it("shows error state when project fails to load", async () => {
    renderRoute("/projects/p1", {
      handlers: [
        sessionHandler(),
        http.get("/api/v1/projects/p1", () =>
          problemResponse(500, "internal-error", { title: "Server Error" }),
        ),
        goalListHandler([]),
      ],
    });

    expect(await screen.findByText("Server Error")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("shows loading state while project loads", async () => {
    renderRoute("/projects/p1", {
      handlers: [sessionHandler(), projectHandler(project), goalListHandler([])],
    });

    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });
});
