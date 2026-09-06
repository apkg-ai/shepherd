import "@testing-library/jest-dom/vitest";
import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";

test("renders the shepherd shell", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "shepherd" })).toBeInTheDocument();
});
