#!/usr/bin/env node
// Merge per-category lcov files into a markdown coverage report for the
// sticky PR comment (Coverage Report job in quality-gates.yaml).
// Dependency-free: parses lcov summary records (LF/LH, FNF/FNH, BRF/BRH).
//
// Usage: node scripts/coverage-report.mjs [--dir coverage] [--out report.md]
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const MARKER = "<!-- shepherd-coverage-report -->";
const LABELS = {
  "core-unit": "Core (Unit)",
  "ui-unit": "UI (Unit)",
};

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const dir = opt("dir", "coverage");
const out = opt("out", null);

const parse = (file) => {
  const totals = { LF: 0, LH: 0, FNF: 0, FNH: 0, BRF: 0, BRH: 0 };
  for (const line of readFileSync(file, "utf8").split("\n")) {
    const m = line.match(/^(LF|LH|FNF|FNH|BRF|BRH):(\d+)/);
    if (m) totals[m[1]] += Number(m[2]);
  }
  return totals;
};

const pct = (hit, found) =>
  found === 0 ? "—" : `${((hit / found) * 100).toFixed(1)}% (${hit}/${found})`;

const row = (label, t) =>
  `| ${label} | ${pct(t.LH, t.LF)} | ${pct(t.FNH, t.FNF)} | ${pct(t.BRH, t.BRF)} |`;

const files = readdirSync(dir)
  .filter((f) => f.endsWith(".lcov"))
  .sort();
if (files.length === 0) {
  console.error(`coverage-report: no .lcov files in ${dir}`);
  process.exit(1);
}

const total = { LF: 0, LH: 0, FNF: 0, FNH: 0, BRF: 0, BRH: 0 };
const lines = [
  MARKER,
  "## Coverage",
  "",
  "| Suite | Lines | Functions | Branches |",
  "|---|---|---|---|",
];
for (const file of files) {
  const suite = basename(file, ".lcov");
  const totals = parse(join(dir, file));
  for (const key of Object.keys(total)) total[key] += totals[key];
  lines.push(row(LABELS[suite] ?? suite, totals));
}
lines.push(row("**Total**", total));

const report = `${lines.join("\n")}\n`;
if (out) {
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, report);
} else {
  process.stdout.write(report);
}
