#!/usr/bin/env node
// Validates every JSON file of a library folder against library/schema/ (PRD §11.1, §18).
// Usage: node scripts/validate-library.mjs [libraryDir ...]   (default: library)
// Semantic rules (unique ids, references, limits) are checked by `cargo test`
// (src-tauri/src/library/validate.rs); this script is the schema gate for CI.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";

const repo = fileURLToPath(new URL("..", import.meta.url));
const schemaDir = join(repo, "library", "schema");

const ajv = new Ajv2020({
  allErrors: true,
  strict: true,
  strictTypes: false,
  strictRequired: false,
});
for (const name of readdirSync(schemaDir).filter((f) => f.endsWith(".schema.json"))) {
  ajv.addSchema(JSON.parse(readFileSync(join(schemaDir, name), "utf8")));
}
const base = "https://schemas.lichtwart.io/demo-simulator/";
const validator = (name) => {
  const v = ajv.getSchema(base + name);
  if (!v) throw new Error(`schema ${name} not found`);
  return v;
};

/** Schema for a file path relative to the library root, or null for non-library files. */
function schemaFor(rel) {
  const parts = rel.split(sep);
  if (parts[0] === "schema" || !rel.endsWith(".json")) return null;
  if (parts[0] === "items" && parts.length === 2) return "items.schema.json";
  if (parts[0] === "demos" && parts.length === 2) return "demo.schema.json";
  if (parts[0] === "floorplans" && parts.length === 3) return "floorplan.schema.json";
  return "unknown";
}

function* walk(dir) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* walk(p);
    else yield p;
  }
}

const roots = process.argv.slice(2);
let failures = 0;
let checked = 0;
for (const root of roots.length ? roots : [join(repo, "library")]) {
  for (const file of walk(root)) {
    const rel = relative(root, file);
    const schema = schemaFor(rel);
    if (schema === null) continue;
    if (schema === "unknown") {
      console.error(`✗ ${rel}: JSON file outside items/, demos/, floorplans/<demoId>/`);
      failures++;
      continue;
    }
    checked++;
    let data;
    try {
      data = JSON.parse(readFileSync(file, "utf8"));
    } catch (e) {
      console.error(`✗ ${rel}: ${e.message}`);
      failures++;
      continue;
    }
    const validate = validator(schema);
    if (!validate(data)) {
      failures++;
      for (const err of validate.errors ?? []) {
        console.error(`✗ ${rel}: ${err.instancePath || "/"} ${err.message}`);
      }
    }
  }
}
console.log(`${checked} library file(s) checked, ${failures} invalid`);
process.exit(failures ? 1 : 0);
