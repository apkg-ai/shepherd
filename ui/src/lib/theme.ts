/**
 * Theme handling. The stored *preference* is light | dark | system; the
 * <html data-theme> attribute always holds the *resolved* theme (light or
 * dark) so tokens.css needs a single dark block. A pre-paint inline script in
 * index.html applies the same logic before React loads to avoid a flash.
 */

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "shepherd-theme";

export function readThemePreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark" || stored === "system") {
      return stored;
    }
  } catch {
    // storage unavailable (privacy mode) — fall through to system
  }
  return "system";
}

export function systemPrefersDark(): boolean {
  // jsdom implements matchMedia minimally or not at all — guard it.
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

export function resolveTheme(preference: ThemePreference, systemDark: boolean): ResolvedTheme {
  if (preference === "system") return systemDark ? "dark" : "light";
  return preference;
}

/** Persists the preference and applies the resolved theme to <html>. */
export function applyThemePreference(preference: ThemePreference): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // best effort — theme still applies for this session
  }
  document.documentElement.dataset["theme"] = resolveTheme(preference, systemPrefersDark());
}

/**
 * Watches OS theme changes; returns an unsubscribe. No-op environments
 * (jsdom without matchMedia listeners) get a noop cleanup.
 */
export function watchSystemTheme(onChange: (dark: boolean) => void): () => void {
  if (typeof window.matchMedia !== "function") return () => {};
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  if (typeof query.addEventListener !== "function") return () => {};
  const listener = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}
