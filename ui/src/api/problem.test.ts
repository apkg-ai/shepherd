import { describe, expect, it } from "vitest";
import { errorSlug, fieldErrors, isShepherdError, ShepherdError } from "./problem";

describe("ShepherdError", () => {
  it("uses detail as the message when present", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:claim-conflict",
      title: "Claim conflict",
      status: 409,
      detail: "Task is already claimed",
    });
    expect(err.message).toBe("Task is already claimed");
    expect(err.title).toBe("Claim conflict");
    expect(err.status).toBe(409);
    expect(err.name).toBe("ShepherdError");
  });

  it("falls back to title as the message", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:not-found",
      title: "Not found",
      status: 404,
    });
    expect(err.message).toBe("Not found");
    expect(err.detail).toBeUndefined();
  });
});

describe("isShepherdError", () => {
  it("narrows ShepherdError instances only", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:x-y",
      title: "t",
      status: 400,
    });
    expect(isShepherdError(err)).toBe(true);
    expect(isShepherdError(new Error("nope"))).toBe(false);
    expect(isShepherdError(undefined)).toBe(false);
    expect(isShepherdError({ status: 400 })).toBe(false);
  });
});

describe("errorSlug", () => {
  it("strips the URN prefix", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:dependency-cycle",
      title: "t",
      status: 409,
    });
    expect(errorSlug(err)).toBe("dependency-cycle");
  });
});

describe("fieldErrors", () => {
  it("maps JSON Pointers to field names, first message wins", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:validation-error",
      title: "Validation failed",
      status: 422,
      errors: [
        { field: "/title", message: "must not be empty" },
        { field: "/title", message: "second message ignored" },
        { field: "/settings/review_gate", message: "must be a boolean" },
      ],
    });
    expect(fieldErrors(err)).toEqual({
      title: "must not be empty",
      "settings.review_gate": "must be a boolean",
    });
  });

  it("returns an empty map when there are no validation errors", () => {
    const err = new ShepherdError({
      type: "urn:shepherd:error:validation-error",
      title: "Validation failed",
      status: 422,
    });
    expect(fieldErrors(err)).toEqual({});
  });
});
