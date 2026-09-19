#!/usr/bin/env node
// Regenerate the freeze list only in a step that owns a contract change:
//   node scripts/check-operation-freeze.ts --print > scripts/v1-operation-ids.txt
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";

const root = resolve(import.meta.dirname, "..");
const { values } = parseArgs({
  options: { spec: { type: "string" }, print: { type: "boolean" } },
});
const specPath = values.spec ?? resolve(root, "plan/contracts/openapi.yaml");
// Contracts are JSON-syntax YAML by design (plan/contracts/README.md).
const spec = JSON.parse(readFileSync(specPath, "utf8")) as {
  paths: Record<string, Record<string, unknown>>;
};
const operationMethods = new Set(["get", "put", "post", "delete", "options", "head", "patch", "trace"]);
const actual = Object.entries(spec.paths)
  .flatMap(([path, item]) =>
    Object.entries(item)
      .filter(([method]) => operationMethods.has(method))
      .map(([method, op]) => {
        const id = (op as { operationId?: unknown })?.operationId;
        if (typeof id !== "string") throw new Error(`${method} ${path}: missing operationId`);
        return id;
      }),
  )
  .sort();

if (values.print) {
  process.stdout.write(actual.join("\n") + "\n");
  process.exit(0);
}

const frozenPath = resolve(root, "scripts/v1-operation-ids.txt");
const frozen = readFileSync(frozenPath, "utf8").trim().split("\n");
if (actual.join("\n") !== frozen.join("\n")) {
  for (const id of actual.filter((x) => !frozen.includes(x))) {
    console.error(`+ ${id} (not in freeze list)`);
  }
  for (const id of frozen.filter((x) => !actual.includes(x))) {
    console.error(`- ${id} (missing from spec)`);
  }
  console.error(
    `operation-freeze: spec has ${actual.length} operation IDs, freeze list has ${frozen.length}; update scripts/v1-operation-ids.txt only in a step that owns the contract change`,
  );
  process.exit(1);
}
console.log(`PASS: ${actual.length} operation IDs match scripts/v1-operation-ids.txt.`);
