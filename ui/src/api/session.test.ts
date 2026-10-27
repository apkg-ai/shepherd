import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useSession } from "./session";

describe("useSession", () => {
  it("throws when used outside AuthBoundary", () => {
    expect(() => renderHook(() => useSession())).toThrow(
      "useSession must be used within AuthBoundary",
    );
  });
});
