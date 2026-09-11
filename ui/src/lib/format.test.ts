import { describe, expect, it } from "vitest";
import { formatDateTime, formatDuration } from "./format";

describe("formatDateTime", () => {
  it("formats ISO timestamps", () => {
    expect(formatDateTime("2026-09-01T10:00:00Z")).toMatch(/2026/);
  });

  it("passes through unparseable input", () => {
    expect(formatDateTime("not-a-date")).toBe("not-a-date");
  });
});

describe("formatDuration", () => {
  it("formats sub-minute durations as seconds", () => {
    expect(formatDuration("2026-09-01T10:00:00Z", "2026-09-01T10:00:42Z")).toBe("42s");
  });

  it("formats minutes with padded seconds", () => {
    expect(formatDuration("2026-09-01T10:00:00Z", "2026-09-01T10:04:05Z")).toBe("4m 05s");
  });

  it("formats hours with padded minutes", () => {
    expect(formatDuration("2026-09-01T10:00:00Z", "2026-09-01T12:14:00Z")).toBe("2h 14m");
  });

  it("returns a dash for negative or invalid ranges", () => {
    expect(formatDuration("2026-09-01T10:00:00Z", "2026-09-01T09:00:00Z")).toBe("—");
    expect(formatDuration("bad", "2026-09-01T09:00:00Z")).toBe("—");
  });
});
