#!/usr/bin/env node
// Fixture manipulation for scripts/check-v1-contracts.sh: known mutations for
// the negative checks, and the generation-warmup server config.
//
// Usage:
//   node scripts/contract-fixtures.ts mutate <broken-ref|corrupt-example|renamed-operation> <plan-copy-dir>
//   node scripts/contract-fixtures.ts server-config <repo-root> <out-toml> <output-dir>
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const readJson = (path: string): unknown => JSON.parse(readFileSync(path, "utf8"));
const writeJson = (path: string, value: unknown): void => {
  writeFileSync(path, JSON.stringify(value));
};

const [command, ...args] = process.argv.slice(2);

if (command === "mutate") {
  const [mutation, planCopy] = args;
  if (!mutation || !planCopy) throw new Error("usage: mutate <case> <plan-copy-dir>");
  const specPath = resolve(planCopy, "contracts/openapi.yaml");
  if (mutation === "broken-ref") {
    const spec = readJson(specPath) as {
      paths: Record<string, { get: { responses: Record<string, { content: Record<string, { schema: { $ref: string } }> }> } }>;
    };
    spec.paths["/health"].get.responses["200"].content["application/json"].schema.$ref =
      "#/components/schemas/__MissingSchema";
    writeJson(specPath, spec);
  } else if (mutation === "corrupt-example") {
    const examplesPath = resolve(planCopy, "examples/operations.json");
    const examples = readJson(examplesPath) as { operation_id: string; response: unknown }[];
    const health = examples.find((e) => e.operation_id === "getHealth");
    if (!health) throw new Error("getHealth example not found");
    health.response = 42;
    writeJson(examplesPath, examples);
  } else if (mutation === "renamed-operation") {
    const spec = readJson(specPath) as {
      paths: Record<string, { get: { operationId: string } }>;
    };
    spec.paths["/health"].get.operationId = "getHealthRenamed";
    writeJson(specPath, spec);
  } else {
    throw new Error(`unknown mutation: ${mutation}`);
  }
  console.log(`mutated ${mutation} in ${planCopy}`);
} else if (command === "server-config") {
  const [root, outToml, outputDir] = args;
  if (!root || !outToml || !outputDir) throw new Error("usage: server-config <repo-root> <out-toml> <output-dir>");
  // Mirror of the server config in plan/check-generation.py (read-only there);
  // update both together if a generator bump changes it.
  const operations = readJson(resolve(root, "plan/contracts/operations.json")) as {
    operation_id: string;
  }[];
  const handled = operations
    .map((o) => o.operation_id)
    .filter((id) => !["getEvents", "exportProject", "importProject"].includes(id));
  const spec = resolve(root, "plan/contracts/openapi.yaml");
  writeFileSync(
    outToml,
    `[generator]\nspec_path=${JSON.stringify(spec)}\noutput_dir=${JSON.stringify(outputDir)}\nmodule_name="shepherd"\n` +
      `[features]\nenable_async_client=false\n` +
      `[generator.types]\ndate_time="chrono"\nuuid="uuid"\n` +
      `[server]\nframework="axum"\noperations=${JSON.stringify(handled)}\nprune_models=true\n` +
      `[server.validation]\nenabled=true\nmax_body_bytes=2097152\nmax_errors=16\n`,
  );
  console.log(`wrote ${outToml} (${handled.length} operations)`);
} else {
  throw new Error(`unknown command: ${command ?? "(none)"}`);
}
