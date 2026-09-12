import { describe, expect, it } from "vitest";
import { isHttpUrl } from "./links";

describe("isHttpUrl", () => {
  it("accepts http and https URLs", () => {
    expect(isHttpUrl("https://example.com/pr/1")).toBe(true);
    expect(isHttpUrl("http://example.com")).toBe(true);
  });

  it("accepts uppercase schemes and surrounding whitespace", () => {
    expect(isHttpUrl("HTTPS://EXAMPLE.COM")).toBe(true);
    expect(isHttpUrl("  https://example.com  ")).toBe(true);
  });

  it("rejects other schemes", () => {
    expect(isHttpUrl("javascript:alert(1)")).toBe(false);
    expect(isHttpUrl("data:text/html,<script>alert(1)</script>")).toBe(false);
    expect(isHttpUrl("vbscript:msgbox")).toBe(false);
  });

  it("rejects scheme-less text, empty input, and tab/newline obfuscation", () => {
    expect(isHttpUrl("just some text")).toBe(false);
    expect(isHttpUrl("")).toBe(false);
    expect(isHttpUrl("java\nscript:alert(1)")).toBe(false);
    expect(isHttpUrl("java\tscript:alert(1)")).toBe(false);
  });
});
