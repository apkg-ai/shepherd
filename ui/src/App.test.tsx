import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";

test("renders the shepherd shell", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "shepherd" })).toBeInTheDocument();
});

// Regression guard for #21: without afterEach(cleanup) in the test setup,
// the first test's DOM leaks into this one and two headings match.
test("renders into a clean DOM", () => {
  render(<App />);
  expect(screen.getAllByRole("heading", { name: "shepherd" })).toHaveLength(1);
});
