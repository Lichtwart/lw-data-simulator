// Usage: node scripts/release-notes.mjs 0.1.0  -> prints that CHANGELOG section to stdout.
import { readFileSync } from "node:fs";

const version = process.argv[2];
const lines = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8").split("\n");
const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
if (start < 0) {
  console.error(`No CHANGELOG section for ${version}`);
  process.exit(1);
}
let end = lines.findIndex((l, i) => i > start && /^## \[/.test(l));
if (end < 0) end = lines.length;
console.log(
  lines
    .slice(start + 1, end)
    .join("\n")
    .trim(),
);
