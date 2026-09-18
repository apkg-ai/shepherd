#!/usr/bin/env node
// Coverage reports + threshold gate. Totals are a union-merge (per file,
// per line, max hit), not a sum; thresholds gate on LINE coverage.
//
// Usage:
//   node scripts/coverage-report.ts --report core|ui [--dir coverage] [--out file.md]
//   node scripts/coverage-report.ts --check [--dir coverage]   # exit 1 on any failure
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

interface SuiteConfig {
  label: string;
  threshold: number;
  pending?: string;
}

interface ReportConfig {
  marker: string;
  title: string;
  suites: Record<string, SuiteConfig>;
  total: number;
}

interface SuiteData {
  lines: Map<string, Map<number, number>>;
  fnf: number;
  fnh: number;
}

interface Row {
  label: string;
  lines: string;
  fns: string;
  threshold: number;
  status: string;
}

interface Evaluation {
  config: ReportConfig;
  rows: Row[];
  failures: string[];
}

const REPORTS: Record<string, ReportConfig> = {
  core: {
    marker: "<!-- shepherd-coverage-core -->",
    title: "Coverage — Core (Rust)",
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
      // Playwright suite (test-e2e job) — monocart emits coverage/e2e.lcov.
      "e2e": { label: "E2E", threshold: 50 },
    },
    total: 92,
  },
};

const args = process.argv.slice(2);
const arg = (name: string): string | null => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? null : args[i + 1];
};
const dir = arg("dir") ?? "coverage";
const out = arg("out");
const reportName = arg("report");
const checkMode = args.includes("--check");

// lcov → { lines: Map<file, Map<line, maxHit>>, fnf, fnh }
const parseSuite = (suite: string): SuiteData | null => {
  const path = join(dir, `${suite}.lcov`);
  if (!existsSync(path)) return null;
  const lines = new Map<string, Map<number, number>>();
  let fnf = 0;
  let fnh = 0;
  let current: string | null = null;
  for (const raw of readFileSync(path, "utf8").split("\n")) {
    if (raw.startsWith("SF:")) {
      current = raw.slice(3);
      if (!lines.has(current)) lines.set(current, new Map());
    } else if (raw.startsWith("DA:") && current) {
      const [line, hit] = raw.slice(3).split(",").map(Number);
      const file = lines.get(current);
      if (file) file.set(line, Math.max(file.get(line) ?? 0, hit));
    } else if (raw.startsWith("FNF:")) {
      fnf += Number(raw.slice(4));
    } else if (raw.startsWith("FNH:")) {
      fnh += Number(raw.slice(4));
    }
  }
  return { lines, fnf, fnh };
};

const mergeInto = (target: Map<string, Map<number, number>>, source: Map<string, Map<number, number>>) => {
  for (const [file, srcLines] of source) {
    const dst = target.get(file) ?? new Map<number, number>();
    for (const [line, hit] of srcLines) dst.set(line, Math.max(dst.get(line) ?? 0, hit));
    target.set(file, dst);
  }
};

const lineStats = (lines: Map<string, Map<number, number>>) => {
  let found = 0;
  let hit = 0;
  for (const file of lines.values()) {
    found += file.size;
    for (const count of file.values()) if (count > 0) hit += 1;
  }
  return { hit, found };
};

const pct = (hit: number, found: number): number | null => (found === 0 ? null : (hit / found) * 100);
const fmt = (hit: number, found: number): string => {
  const ratio = pct(hit, found);
  return ratio === null ? "—" : `${ratio.toFixed(1)}% (${hit}/${found})`;
};

const evaluate = (name: string): Evaluation => {
  const config = REPORTS[name];
  const rows: Row[] = [];
  const failures: string[] = [];
  const union = new Map<string, Map<number, number>>();
  for (const [suite, { label, threshold, pending }] of Object.entries(config.suites)) {
    const parsed = parseSuite(suite);
    if (!parsed) {
      // Only suites marked pending may be absent; a missing lcov means a failed or lost CI artifact.
      if (pending) {
        rows.push({ label, lines: "—", fns: "—", threshold, status: `⏳ lands in ${pending}` });
      } else {
        failures.push(`${name}/${suite}: coverage artifact missing`);
        rows.push({ label, lines: "—", fns: "—", threshold, status: "❌ missing artifact" });
      }
      continue;
    }
    const { hit, found } = lineStats(parsed.lines);
    const ratio = pct(hit, found);
    const ok = ratio !== null && ratio >= threshold;
    if (!ok) failures.push(`${name}/${suite}: lines ${fmt(hit, found)} < ${threshold}%`);
    rows.push({ label, lines: fmt(hit, found), fns: fmt(parsed.fnh, parsed.fnf), threshold, status: ok ? "✅" : "❌" });
    mergeInto(union, parsed.lines);
  }
  const { hit, found } = lineStats(union);
  const totalRatio = pct(hit, found);
  const totalOk = totalRatio !== null && totalRatio >= config.total;
  if (!totalOk) failures.push(`${name}/total: lines ${fmt(hit, found)} < ${config.total}%`);
  rows.push({ label: "**Total (union)**", lines: fmt(hit, found), fns: "—", threshold: config.total, status: totalOk ? "✅" : "❌" });
  return { config, rows, failures };
};

const render = ({ config, rows }: Evaluation): string => {
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
  if (reportName === null || !REPORTS[reportName]) {
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
