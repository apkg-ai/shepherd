import { describe, expect, it } from "vitest";
import { newIdempotencyKey } from "./idempotency";

describe("newIdempotencyKey", () => {
  it("returns a valid UUID", () => {
    const key = newIdempotencyKey();
    expect(key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  });

  it("returns unique keys on each call", () => {
    const a = newIdempotencyKey();
    const b = newIdempotencyKey();
    expect(a).not.toBe(b);
  });
});
