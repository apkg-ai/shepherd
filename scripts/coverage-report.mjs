#!/usr/bin/env node
// Coverage reports + threshold gate for the Coverage Report CI job.
// Two reports, each its own sticky PR comment: core (Rust) and ui (TypeScript).
// Suites are per-category lcov files (<suite>.lcov); totals are a proper
// union-merge (per file, per line, max hit) — not a sum across suites.
// Thresholds gate on LINE coverage; functions shown for information.
//
// Usage:
//   node scripts/coverage-report.mjs --report core|ui [--dir coverage] [--out file.md]
//   node scripts/coverage-report.mjs --check [--dir coverage]   # exit 1 on any failure
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const REPORTS = {
  core: {
    marker: "<!-- shepherd-coverage-core -->",
    title: "Coverage — Core (Rust)",
    // S3 adds property + migration, S4 adds contract as new suites here.
    suites: {
      "core-unit": { label: "Unit", threshold: 95 },
      "core-integration": { label: "Integration", threshold: 70 },
    },
    total: 92,
  },
  ui: {
    marker: "<!-- shepherd-coverage-ui -->",
    title: "Coverage — UI (TypeScript)",
    suites: {
      "ui-unit": { label: "Unit", threshold: 95 },
      // Enforced automatically once the suite's lcov exists (Playwright, S9).
      "e2e": { label: "E2E", threshold: 50, pending: "S9" },
    },
    total: 92,
  },
};

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1];
};
const dir = opt("dir", "coverage");
const out = opt("out", null);
const reportName = opt("report", null);
const checkMode = args.includes("--check");

// lcov → { lines: Map<file, Map<line, maxHit>>, fnf, fnh }
const parseSuite = (suite) => {
  const path = join(dir, `${suite}.lcov`);
  if (!existsSync(path)) return null;
  const lines = new Map();
  let fnf = 0;
  let fnh = 0;
  let current = null;
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    if (raw.startsWith("SF:")) {
      current = raw.slice(3);
      if (!lines.has(current)) lines.set(current, new Map());
    } else if (raw.startsWith("DA:") && current) {
      const [line, hit] = raw.slice(3).split(",").map(Number);
      const file = lines.get(current);
      file.set(line, Math.max(file.get(line) ?? 0, hit));
    } else if (raw.startsWith("FNF:")) {
      fnf += Number(raw.slice(4));
    } else if (raw.startsWith("FNH:")) {
      fnh += Number(raw.slice(4));
    }
  }
  return { lines, fnf, fnh };
};

const mergeInto = (target, source) => {
  for (const [file, srcLines] of source) {
    const dst = target.get(file) ?? new Map();
    for (const [line, hit] of srcLines) dst.set(line, Math.max(dst.get(line) ?? 0, hit));
    target.set(file, dst);
  }
};

const lineStats = (lines) => {
  let found = 0;
  let hit = 0;
  for (const file of lines.values()) {
    found += file.size;
    for (const count of file.values()) if (count > 0) hit += 1;
  }
  return { hit, found };
};

const pct = (hit, found) => (found === 0 ? null : (hit / found) * 100);
const fmt = (hit, found) =>
  found === 0 ? "—" : `${pct(hit, found).toFixed(1)}% (${hit}/${found})`;

const evaluate = (name) => {
  const config = REPORTS[name];
  const rows = [];
  const failures = [];
  const union = new Map();
  for (const [suite, { label, threshold, pending }] of Object.entries(config.suites)) {
    const parsed = parseSuite(suite);
    if (!parsed) {
      rows.push({ label, lines: "—", fns: "—", threshold, status: `⏳ lands in ${pending ?? "a later session"}` });
      continue;
    }
    const { hit, found } = lineStats(parsed.lines);
    const ok = pct(hit, found) >= threshold;
    if (!ok) failures.push(`${name}/${suite}: lines ${fmt(hit, found)} < ${threshold}%`);
    rows.push({ label, lines: fmt(hit, found), fns: fmt(parsed.fnh, parsed.fnf), threshold, status: ok ? "✅" : "❌" });
    mergeInto(union, parsed.lines);
  }
  const { hit, found } = lineStats(union);
  const totalOk = found > 0 && pct(hit, found) >= config.total;
  if (!totalOk) failures.push(`${name}/total: lines ${fmt(hit, found)} < ${config.total}%`);
  rows.push({ label: "**Total (union)**", lines: fmt(hit, found), fns: "—", threshold: config.total, status: totalOk ? "✅" : "❌" });
  return { config, rows, failures };
};

const render = ({ config, rows }) => {
  const lines = [
    config.marker,
    `## ${config.title}`,
    "",
    "| Suite | Lines | Functions | Threshold | Status |",
    "|---|---|---|---|---|",
    ...rows.map((r) => `| ${r.label} | ${r.lines} | ${r.fns} | ≥ ${r.threshold}% | ${r.status} |`),
  ];
  return `${lines.join("\n")}\n`;
};

if (checkMode) {
  const failures = Object.keys(REPORTS).flatMap((name) => evaluate(name).failures);
  if (failures.length > 0) {
    console.error("coverage thresholds not met:");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log("coverage thresholds met");
} else {
  if (!REPORTS[reportName]) {
    console.error(`coverage-report: --report must be one of: ${Object.keys(REPORTS).join(", ")}`);
    process.exit(1);
  }
  const report = render(evaluate(reportName));
  if (out) {
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, report);
  } else {
    process.stdout.write(report);
  }
}
