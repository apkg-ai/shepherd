import { afterEach, describe, expect, it, vi } from "vitest";
import { downloadJson, slugify } from "./download";

describe("downloadJson", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("creates and clicks a download anchor", () => {
    const createObjectURL = vi.fn(() => "blob:mock");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL,
      revokeObjectURL,
    });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    downloadJson({ hello: "world" }, "export.json");

    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(click).toHaveBeenCalledOnce();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock");
    click.mockRestore();
  });
});

describe("slugify", () => {
  it("lowercases and dashes non-alphanumerics", () => {
    expect(slugify("My Project! v2")).toBe("my-project-v2");
  });

  it("falls back for fully non-alphanumeric names", () => {
    expect(slugify("!!!")).toBe("project");
  });
});
