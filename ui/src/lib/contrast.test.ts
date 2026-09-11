import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { contrastRatio, parseThemes, relativeLuminance } from "./contrast";

// vitest 5 stubs `.css?raw` imports to "" and its import.meta.url is not a
// file: URL, so read via the vitest root (ui/) instead.
const tokensCss = readFileSync("src/styles/tokens.css", "utf8");

describe("contrast math", () => {
  it("computes the canonical extremes", () => {
    expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 5);
    expect(contrastRatio("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
  });

  it("is order-independent", () => {
    expect(contrastRatio("#1a1d21", "#f6f7f9")).toBeCloseTo(
      contrastRatio("#f6f7f9", "#1a1d21"),
      10,
    );
  });

  it("matches known reference values", () => {
    // #767676 on white is the classic "just passes AA" grey (~4.54).
    expect(contrastRatio("#767676", "#ffffff")).toBeGreaterThan(4.5);
    expect(contrastRatio("#767676", "#ffffff")).toBeLessThan(4.6);
    expect(relativeLuminance("#ff0000")).toBeCloseTo(0.2126, 4);
  });

  it("rejects malformed colors", () => {
    expect(() => relativeLuminance("red")).toThrow(/rrggbb/);
    expect(() => relativeLuminance("#fff")).toThrow(/rrggbb/);
  });
});

describe("parseThemes", () => {
  it("inherits non-overridden tokens from light into dark", () => {
    const themes = parseThemes(
      `:root {\n  --color-bg: #ffffff;\n  --color-accent: #1d4ed8;\n}\n[data-theme="dark"] {\n  --color-bg: #0f1216;\n}`,
    );
    expect(themes.light["accent"]).toBe("#1d4ed8");
    expect(themes.dark["bg"]).toBe("#0f1216");
    expect(themes.dark["accent"]).toBe("#1d4ed8"); // inherited
  });
});

/**
 * THE GATE — every fg/bg pairing the UI renders, both themes, WCAG AA.
 * 4.5:1 for text (everything in this UI is small text), 3:1 for non-text
 * (focus rings, status accent bars). Adding a pairing is one line; breaking
 * a token makes this fail with the exact pairing and computed ratio.
 */
const AA_TEXT = 4.5;
const AA_NON_TEXT = 3;

interface Pair {
  fg: string;
  bg: string;
  min: number;
  note: string;
}

const PAIRS: Pair[] = [
  // Body text on the three surfaces
  { fg: "text", bg: "bg", min: AA_TEXT, note: "body text / inputs" },
  { fg: "text", bg: "surface", min: AA_TEXT, note: "metadata pre, approved badge, brand" },
  { fg: "text", bg: "surface-raised", min: AA_TEXT, note: "cards, dialogs, toasts" },
  // Muted text
  { fg: "text-muted", bg: "bg", min: AA_TEXT, note: "descriptions, timeline meta, tabs" },
  {
    fg: "text-muted",
    bg: "surface",
    min: AA_TEXT,
    note: "sidebar links, table headers, proposed badge",
  },
  {
    fg: "text-muted",
    bg: "surface-raised",
    min: AA_TEXT,
    note: "chips, dialog close, fact labels",
  },
  { fg: "text-muted", bg: "success-soft", min: AA_TEXT, note: "toast dismiss (success)" },
  { fg: "text-muted", bg: "danger-soft", min: AA_TEXT, note: "toast dismiss (error)" },
  // Faint text
  { fg: "text-faint", bg: "bg", min: AA_TEXT, note: "cancelled badge, knowledge type labels" },
  { fg: "text-faint", bg: "surface", min: AA_TEXT, note: "sidebar section label, done badge" },
  { fg: "text-faint", bg: "surface-raised", min: AA_TEXT, note: "parent crumb, review row type" },
  // Accent
  { fg: "accent", bg: "bg", min: AA_TEXT, note: "links, ghost buttons, active tab" },
  { fg: "accent", bg: "surface", min: AA_TEXT, note: "sidebar hover states" },
  { fg: "accent", bg: "surface-raised", min: AA_TEXT, note: "ghost button in cards" },
  { fg: "accent", bg: "accent-soft", min: AA_TEXT, note: "ready badge, active nav/switcher/theme" },
  { fg: "on-accent", bg: "accent", min: AA_TEXT, note: "primary button, in_progress badge" },
  // Semantic colors
  { fg: "danger", bg: "bg", min: AA_TEXT, note: "danger buttons, form errors, failed outcome" },
  { fg: "danger", bg: "surface-raised", min: AA_TEXT, note: "dialog form errors" },
  { fg: "danger", bg: "danger-soft", min: AA_TEXT, note: "failure reason, failed attempts badge" },
  { fg: "success", bg: "bg", min: AA_TEXT, note: "succeeded outcome" },
  { fg: "success", bg: "success-soft", min: AA_TEXT, note: "success accents on soft fill" },
  { fg: "warning", bg: "warning-soft", min: AA_TEXT, note: "blocked badge, waits-on chip" },
  { fg: "attention", bg: "attention-soft", min: AA_TEXT, note: "in_review badge, tab counts" },
  { fg: "pink", bg: "pink-soft", min: AA_TEXT, note: "proposed badge" },
  { fg: "teal", bg: "teal-soft", min: AA_TEXT, note: "approved badge" },
  { fg: "bg", bg: "attention", min: AA_TEXT, note: "sidebar review badge (inverted)" },
  // Toast messages inherit --color-text over soft fills
  { fg: "text", bg: "success-soft", min: AA_TEXT, note: "toast success message" },
  { fg: "text", bg: "danger-soft", min: AA_TEXT, note: "toast error message" },
  // Non-text UI (3:1): focus ring and status row accent bars
  {
    fg: "accent",
    bg: "surface",
    min: AA_NON_TEXT,
    note: "focus ring on sidebar/table-header surfaces",
  },
  { fg: "accent", bg: "accent-soft", min: AA_NON_TEXT, note: "focus ring on active nav items" },
  {
    fg: "accent",
    bg: "surface-raised",
    min: AA_NON_TEXT,
    note: "ready/in_progress row accent bar",
  },
  { fg: "attention", bg: "surface-raised", min: AA_NON_TEXT, note: "in_review row accent bar" },
  { fg: "warning", bg: "surface-raised", min: AA_NON_TEXT, note: "blocked row accent bar" },
];

describe("WCAG AA token gate", () => {
  const themes = parseThemes(tokensCss);

  for (const theme of ["light", "dark"] as const) {
    describe(`${theme} theme`, () => {
      it.each(PAIRS)("$note — $fg on $bg ≥ $min", ({ fg, bg, min, note }) => {
        const tokens = themes[theme];
        const fgColor = tokens[fg];
        const bgColor = tokens[bg];
        expect(fgColor, `token --color-${fg} missing`).toBeDefined();
        expect(bgColor, `token --color-${bg} missing`).toBeDefined();
        const ratio = contrastRatio(fgColor!, bgColor!);
        expect(
          ratio,
          `${note}: --color-${fg} (${fgColor}) on --color-${bg} (${bgColor}) = ${ratio.toFixed(2)}:1, needs ≥ ${min}:1 [${theme}]`,
        ).toBeGreaterThanOrEqual(min);
      });
    });
  }
});
