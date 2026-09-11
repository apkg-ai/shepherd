import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyThemePreference,
  readThemePreference,
  resolveTheme,
  systemPrefersDark,
  THEME_STORAGE_KEY,
  watchSystemTheme,
} from "./theme";

function stubMatchMedia(matches: boolean) {
  const listeners = new Set<(e: { matches: boolean }) => void>();
  const query = {
    matches,
    addEventListener: (_: string, cb: (e: { matches: boolean }) => void) => listeners.add(cb),
    removeEventListener: (_: string, cb: (e: { matches: boolean }) => void) => listeners.delete(cb),
  };
  vi.stubGlobal("matchMedia", () => query);
  return {
    fire: (dark: boolean) => listeners.forEach((cb) => cb({ matches: dark })),
    listeners,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  delete document.documentElement.dataset["theme"];
});

describe("readThemePreference", () => {
  it("returns stored valid preferences", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "dark");
    expect(readThemePreference()).toBe("dark");
  });

  it("falls back to system for missing or garbage values", () => {
    expect(readThemePreference()).toBe("system");
    localStorage.setItem(THEME_STORAGE_KEY, "hotdog");
    expect(readThemePreference()).toBe("system");
  });

  it("falls back to system when storage throws", () => {
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(readThemePreference()).toBe("system");
    spy.mockRestore();
  });
});

describe("resolveTheme", () => {
  it("passes explicit preferences through", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("resolves system from the OS", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
});

describe("systemPrefersDark", () => {
  it("is false when matchMedia is unavailable (jsdom default)", () => {
    expect(systemPrefersDark()).toBe(false);
  });

  it("reflects the media query when available", () => {
    stubMatchMedia(true);
    expect(systemPrefersDark()).toBe(true);
  });
});

describe("applyThemePreference", () => {
  it("persists the preference and sets the resolved theme", () => {
    applyThemePreference("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(document.documentElement.dataset["theme"]).toBe("dark");
  });

  it("resolves system against the OS preference", () => {
    stubMatchMedia(true);
    applyThemePreference("system");
    expect(document.documentElement.dataset["theme"]).toBe("dark");
  });

  it("still applies the theme when storage writes fail", () => {
    const spy = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("full");
    });
    applyThemePreference("light");
    expect(document.documentElement.dataset["theme"]).toBe("light");
    spy.mockRestore();
  });
});

describe("watchSystemTheme", () => {
  it("is a noop without matchMedia", () => {
    const unsubscribe = watchSystemTheme(() => {});
    expect(unsubscribe).toBeTypeOf("function");
    unsubscribe();
  });

  it("subscribes and unsubscribes from OS changes", () => {
    const media = stubMatchMedia(false);
    const onChange = vi.fn();
    const unsubscribe = watchSystemTheme(onChange);
    media.fire(true);
    expect(onChange).toHaveBeenCalledWith(true);
    unsubscribe();
    expect(media.listeners.size).toBe(0);
  });
});
