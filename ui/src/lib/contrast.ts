/**
 * WCAG 2.x contrast math + a tokens.css theme parser, powering the contrast
 * gate in contrast.test.ts: every fg/bg pairing the UI uses is asserted
 * ≥ 4.5:1 (text) or ≥ 3:1 (non-text) in BOTH themes on every test run.
 */

/** #rrggbb → WCAG relative luminance (sRGB linearization). */
export function relativeLuminance(hex: string): number {
  const match = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) throw new Error(`expected #rrggbb color, got "${hex}"`);
  const channels = [0, 2, 4].map((offset) => {
    const value = parseInt(match[1]!.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0]! + 0.7152 * channels[1]! + 0.0722 * channels[2]!;
}

/** WCAG contrast ratio between two #rrggbb colors (1..21, order-independent). */
export function contrastRatio(a: string, b: string): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const [lighter, darker] = la >= lb ? [la, lb] : [lb, la];
  return (lighter + 0.05) / (darker + 0.05);
}

export interface ThemeTokens {
  light: Record<string, string>;
  dark: Record<string, string>;
}

function extractBlock(css: string, selector: string): Record<string, string> {
  const start = css.indexOf(selector);
  if (start === -1) throw new Error(`selector "${selector}" not found`);
  const open = css.indexOf("{", start);
  const close = css.indexOf("\n}", open);
  const body = css.slice(open + 1, close);
  const tokens: Record<string, string> = {};
  for (const match of body.matchAll(/--color-([a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
    tokens[match[1]!] = match[2]!.toLowerCase();
  }
  return tokens;
}

/**
 * Parses tokens.css into per-theme color maps (keys without the `--color-`
 * prefix). Dark spreads over light, mirroring the cascade: any token the
 * dark block doesn't override inherits its light value.
 */
export function parseThemes(css: string): ThemeTokens {
  const light = extractBlock(css, ":root");
  const dark = { ...light, ...extractBlock(css, '[data-theme="dark"]') };
  return { light, dark };
}
