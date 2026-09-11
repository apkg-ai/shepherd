import { describe, expect, it } from "vitest";
import * as zod from "zod";
import { zodFieldErrors } from "./forms";

describe("zodFieldErrors", () => {
  const schema = zod.object({
    title: zod.string().min(1),
    settings: zod.object({ review_gate: zod.boolean() }),
  });

  it("maps issue paths to dotted field names", () => {
    const result = schema.safeParse({ title: "", settings: { review_gate: 1 } });
    expect(result.success).toBe(false);
    if (result.success) return;
    const errors = zodFieldErrors(result.error);
    expect(Object.keys(errors).sort()).toEqual(["settings.review_gate", "title"]);
  });

  it("collects pathless issues under _form", () => {
    const refined = zod.object({ a: zod.string() }).refine(() => false, { message: "nope" });
    const result = refined.safeParse({ a: "x" });
    expect(result.success).toBe(false);
    if (result.success) return;
    expect(zodFieldErrors(result.error)).toEqual({ _form: "nope" });
  });
});
