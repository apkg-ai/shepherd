import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { getGetHealthMockHandler } from "../api/generated/system/system.msw";
import { sessionHandler, projectListHandler } from "../test/msw";
import { renderRoute } from "../test/test-utils";

const healthy = getGetHealthMockHandler({
  status: "pass",
  version: "0.1.0",
  schema_version: 1,
});

const authed = [healthy, sessionHandler(), projectListHandler([])];

describe("AppShell", () => {
  it("renders the accessible scaffold: skip link, landmarks, brand, theme toggle", async () => {
    renderRoute("/", { handlers: authed });

    expect(screen.getByRole("link", { name: "Skip to content" })).toHaveAttribute("href", "#main");
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Shepherd" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Dark" })).toBeInTheDocument();
    await screen.findByText("Projects");
  });

  it("skip link focuses main without navigating", async () => {
    const user = userEvent.setup();
    renderRoute("/", { handlers: authed });

    await user.click(screen.getByRole("link", { name: "Skip to content" }));
    expect(screen.getByRole("main")).toHaveFocus();
    expect(screen.getByRole("heading", { level: 1, name: "Shepherd" })).toBeInTheDocument();
  });
});

describe("ProjectListScreen", () => {
  it("shows the empty state with CTA when no projects exist", async () => {
    renderRoute("/", { handlers: authed });

    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create your first project" })).toBeInTheDocument();
  });
});

describe("NotFound", () => {
  it("renders for unknown routes with a way back home", async () => {
    renderRoute("/definitely-not-a-route", { handlers: [healthy] });

    expect(screen.getByText("Page not found")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back home" })).toHaveAttribute("href", "/");
  });
});
