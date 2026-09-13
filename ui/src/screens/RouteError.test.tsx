import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it } from "vitest";
import { RouteError } from "./RouteError";

/** A route whose element throws during render — errorElement catches it. */
function renderFailingRoute(fail: () => unknown) {
  function Boom() {
    fail();
    return null;
  }
  const router = createMemoryRouter([
    {
      path: "/",
      element: <Boom />,
      errorElement: <RouteError />,
    },
  ]);
  render(<RouterProvider router={router} />);
}

describe("RouteError", () => {
  it("renders a fallback with the error message and a way back", () => {
    renderFailingRoute(() => {
      throw new Error("kaboom");
    });

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("kaboom")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Back to projects" })).toHaveAttribute("href", "/");
  });

  it("renders route error responses with status and status text", async () => {
    // A loader rejection with a Response is react-router's ErrorResponse
    // path (isRouteErrorResponse).
    const router = createMemoryRouter([
      {
        path: "/",
        loader: async () => {
          throw new Response("Not Found", { status: 404, statusText: "Not Found" });
        },
        element: <p>never rendered</p>,
        errorElement: <RouteError />,
      },
    ]);
    render(<RouterProvider router={router} />);

    expect(await screen.findByText("404 Not Found")).toBeInTheDocument();
  });

  it("renders a generic detail for non-Error throwables", () => {
    renderFailingRoute(() => {
      throw "just a string";
    });

    expect(screen.getByText("Unexpected error")).toBeInTheDocument();
  });
});
