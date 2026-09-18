import { http } from "msw";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { getGetHealthMockHandler } from "../api/generated/system/system.msw";
import { problemResponse } from "../test/msw";
import { renderRoute } from "../test/test-utils";

const healthy = getGetHealthMockHandler({
  status: "pass",
  version: "0.1.0",
  description: "shepherd local daemon",
});

describe("AppShell", () => {
  it("renders the accessible scaffold: skip link, landmarks, brand, theme toggle", async () => {
    renderRoute("/", { handlers: [healthy] });

    expect(screen.getByRole("link", { name: "Skip to content" })).toHaveAttribute("href", "#main");
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Shepherd" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Dark" })).toBeInTheDocument();
    await screen.findByText("0.1.0"); // settle the health query
  });

  it("skip link focuses main without navigating", async () => {
    const user = userEvent.setup();
    renderRoute("/", { handlers: [healthy] });

    await user.click(screen.getByRole("link", { name: "Skip to content" }));
    expect(screen.getByRole("main")).toHaveFocus();
    expect(screen.getByRole("heading", { level: 1, name: "Shepherd" })).toBeInTheDocument();
  });
});

describe("HomeScreen", () => {
  it("shows the daemon's health status and version", async () => {
    renderRoute("/", { handlers: [healthy] });

    expect(await screen.findByText("pass")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("shepherd local daemon")).toBeInTheDocument();
  });

  it("shows an error state with retry when health fails, and recovers", async () => {
    const user = userEvent.setup();
    let failing = true;
    renderRoute("/", {
      handlers: [
        http.get("*/health", () => {
          if (failing) {
            return problemResponse(500, "internal-error", {
              title: "Internal Server Error",
              detail: "Health check failed.",
            });
          }
          return undefined;
        }),
        healthy,
      ],
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("Internal Server Error");

    failing = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByText("pass")).toBeInTheDocument());
  });
});

describe("NotFound", () => {
  it("renders for unknown routes with a way back home", async () => {
    renderRoute("/definitely-not-a-route", { handlers: [healthy] });

    expect(screen.getByText("Page not found")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back home" })).toHaveAttribute("href", "/");
  });
});
