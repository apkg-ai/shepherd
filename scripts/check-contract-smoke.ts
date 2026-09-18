#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
// ajv is hoisted at the repo root via spectral, same as plan/validate-contracts.cjs.
import { Ajv2020 } from "ajv/dist/2020.js";

interface MediaObject {
  schema?: unknown;
  example?: unknown;
  examples?: Record<string, { value: unknown }>;
}

interface OperationObject {
  operationId: string;
  requestBody?: { content?: Record<string, MediaObject> };
  responses?: Record<string, { content?: Record<string, MediaObject> }>;
}

const root = resolve(import.meta.dirname, "..");
const spec = JSON.parse(
  readFileSync(resolve(root, "plan/contracts/openapi.yaml"), "utf8"),
) as {
  components: { schemas: Record<string, unknown> };
  paths: Record<string, Record<string, OperationObject>>;
};

const ajv = new Ajv2020({ strict: false, allErrors: true });
// Same formats as plan/validate-contracts.cjs; the contract uses no others.
ajv.addFormat("int32", {
  type: "number",
  validate: (v: number) => Number.isInteger(v) && v >= -2147483648 && v <= 2147483647,
});
ajv.addFormat("uuid", /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
ajv.addFormat("date-time", (v: string) => /^\d{4}-\d\d-\d\dT/.test(v) && !Number.isNaN(Date.parse(v)));
ajv.addFormat("uri", (v: string) => {
  try {
    return Boolean(new URL(v).protocol);
  } catch {
    return false;
  }
});

const MODELS = "https://shepherd.local/plan/smoke";
const relocate = (schema: unknown): object =>
  JSON.parse(
    JSON.stringify(schema).replace(/#\/components\/schemas\//g, () => `${MODELS}#/$defs/`),
  ) as object;
ajv.addSchema({ $id: MODELS, $defs: relocate(spec.components.schemas) });

const names = Object.keys(spec.components.schemas);
for (const name of names) {
  if (!ajv.getSchema(`${MODELS}#/$defs/${name}`)) throw new Error(`schema did not compile: ${name}`);
}

// text/event-stream only; no JSON example by design.
const SSE_ONLY = new Set(["getEvents"]);
let validated = 0;
const check = (id: string, where: string, media: MediaObject | undefined): number => {
  if (!media) return 0;
  const samples =
    media.example !== undefined
      ? [media.example]
      : Object.values(media.examples ?? {}).map((e) => e.value);
  if (!media.schema) {
    if (samples.length > 0) throw new Error(`${id} ${where}: example without a schema`);
    return 0;
  }
  if (samples.length === 0) return 0;
  const fn = ajv.compile(relocate(media.schema));
  for (const sample of samples) {
    if (!fn(sample)) throw new Error(`${id} ${where}: ${JSON.stringify(fn.errors)}`);
    validated++;
  }
  return samples.length;
};

for (const item of Object.values(spec.paths)) {
  for (const op of Object.values(item)) {
    let count = check(op.operationId, "request", op.requestBody?.content?.["application/json"]);
    for (const [status, response] of Object.entries(op.responses ?? {})) {
      count += check(op.operationId, `response ${status}`, response.content?.["application/json"]);
    }
    if (count === 0 && !SSE_ONLY.has(op.operationId)) {
      throw new Error(`no inline example validated for ${op.operationId}`);
    }
  }
}
console.log(`PASS: ${names.length} schemas compile; ${validated} inline operation examples validate.`);
