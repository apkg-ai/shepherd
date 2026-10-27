import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import { Breadcrumbs } from "./Breadcrumbs";
import "../i18n";

function renderBreadcrumbs(items: { label: string; to?: string }[]) {
  return render(
    <MemoryRouter>
      <Breadcrumbs items={items} />
    </MemoryRouter>,
  );
}

describe("Breadcrumbs", () => {
  it("renders items with links and current page", () => {
    renderBreadcrumbs([
      { label: "Projects", to: "/" },
      { label: "Space Game", to: "/projects/1" },
      { label: "Combat" },
    ]);

    expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/");
    expect(screen.getByRole("link", { name: "Space Game" })).toHaveAttribute("href", "/projects/1");
    expect(screen.getByText("Combat")).not.toHaveAttribute("href");
  });

  it("marks the last item with aria-current=page", () => {
    renderBreadcrumbs([{ label: "Projects", to: "/" }, { label: "Current page" }]);

    expect(screen.getByText("Current page")).toHaveAttribute("aria-current", "page");
    expect(screen.getByText("Projects")).not.toHaveAttribute("aria-current");
  });

  it("renders a single item without link", () => {
    renderBreadcrumbs([{ label: "Home" }]);

    expect(screen.getByText("Home")).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });
});
