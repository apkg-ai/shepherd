import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { RouteError } from "./RouteError";

// Mock only useRouteError — the component is a pure presenter over the
// router's caught error. Driving a real render-time throw makes React
// Router log the error and surfaces it as an unhandled error in vitest,
// which fails the CI run; the errorElement wiring itself is exercised by
// the router in production and by the graph screen tests' real routes.
const useRouteErrorMock = vi.hoisted(() => vi.fn());
vi.mock("react-router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router")>();
  return { ...actual, useRouteError: useRouteErrorMock };
});

function renderRouteError(error: unknown) {
  useRouteErrorMock.mockReturnValue(error);
  render(
    <MemoryRouter>
      <RouteError />
    </MemoryRouter>,
  );
}

describe("RouteError", () => {
  it("renders a fallback with the error message and a way back", () => {
    renderRouteError(new Error("kaboom"));

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("kaboom")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to projects" })).toHaveAttribute("href", "/");
  });

  it("renders route error responses with status and status text", () => {
    // The ErrorResponse shape react-router passes to errorElement
    // (isRouteErrorResponse duck-types on these fields).
    renderRouteError({ status: 404, statusText: "Not Found", internal: false, data: null });

    expect(screen.getByText("404 Not Found")).toBeInTheDocument();
  });

  it("renders a generic detail for non-Error throwables", () => {
    renderRouteError("just a string");

    expect(screen.getByText("Unexpected error")).toBeInTheDocument();
  });
});
