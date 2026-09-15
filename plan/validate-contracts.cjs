/* Read-only JSON Schema checks; uses existing repo dev dependencies. */
const fs = require('node:fs');
const path = require('node:path');
const Ajv = require('ajv/dist/2020');
const root = __dirname;
const read = p => JSON.parse(fs.readFileSync(path.join(root, p), 'utf8'));
const ajv = new Ajv({strict: false, allErrors: true});
ajv.addFormat('int32', {type: 'number', validate: v => Number.isInteger(v) && v >= -2147483648 && v <= 2147483647});
ajv.addFormat('uuid', /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
ajv.addFormat('date-time', v => /^\d{4}-\d\d-\d\dT/.test(v) && !Number.isNaN(Date.parse(v)));
ajv.addFormat('uri', v => {try {return Boolean(new URL(v).protocol)} catch {return false}});
const api = read('contracts/openapi.yaml');
const defs = JSON.parse(JSON.stringify(api.components.schemas).replace(/#\/components\/schemas\//g, '#/$defs/'));
ajv.addSchema({$id: 'https://shepherd.local/plan/models', $defs: defs});
let checked = 0;
function validate(name, value) {
  const fn = ajv.getSchema('https://shepherd.local/plan/models#/$defs/' + name);
  if (!fn(value)) throw new Error(name + ': ' + JSON.stringify(fn.errors));
  checked++;
}
const ops = new Map(read('contracts/operations.json').map(o => [o.operation_id,o]));
for (const example of read('examples/operations.json')) {
  const op = ops.get(example.operation_id);
  if(op.request) validate(op.request, example.request);
  validate(op.response, example.response);
}
const exportSchema = read('contracts/export.schema.json');
const exportValidator = ajv.compile(exportSchema);
const fixture = read('examples/reference-project.json');
if(!exportValidator(fixture)) throw new Error(JSON.stringify(exportValidator.errors));
const bad = structuredClone(fixture); bad.tasks[0].epic_id = null;
if(exportValidator(bad)) throw new Error('Invalid missing epic accepted');
const extra = structuredClone(fixture); extra.tasks[0].invented_status = 'ready';
if(exportValidator(extra)) throw new Error('Unexpected field accepted');
console.log(`PASS: ${checked} operation request/response examples; export fixture and rejection checks.`);
