// Usage: node scripts/release.mjs <version>   e.g. 0.2.0
// Bumps the app version (= library version, PRD 11.1) in package.json, package-lock.json,
// src-tauri/Cargo.toml, src-tauri/tauri.conf.json (when it holds a literal version) and moves
// CHANGELOG "Unreleased" into a dated section. Does not commit or tag.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
  console.error("Usage: node scripts/release.mjs <MAJOR.MINOR.PATCH>");
  process.exit(1);
}
const git = (...a) => execFileSync("git", a, { encoding: "utf8" }).trim();
const die = (msg) => {
  console.error(msg);
  process.exit(1);
};

// Preconditions, all checked before anything is written.
if (git("rev-parse", "--abbrev-ref", "HEAD") !== "main") die("Release from the main branch.");
if (git("status", "--porcelain") !== "") die("Working tree is not clean. Commit or stash first.");

const path = (p) => new URL(`../${p}`, import.meta.url);
const read = (p) => readFileSync(path(p), "utf8");
const write = (p, s) => writeFileSync(path(p), s);

const changelog = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8");
const unreleasedAt = changelog.indexOf("## [Unreleased]");
if (unreleasedAt < 0) die('CHANGELOG.md has no "## [Unreleased]" section');
if (changelog.includes(`## [${version}]`)) die(`CHANGELOG.md already has a section for ${version}`);
const afterUnreleased = changelog.slice(unreleasedAt + "## [Unreleased]".length);
const nextSection = afterUnreleased.search(/^## \[/m);
const unreleasedBody = nextSection < 0 ? afterUnreleased : afterUnreleased.slice(0, nextSection);
if (unreleasedBody.trim() === "")
  die('The "## [Unreleased]" section is empty. Add release notes first.');

function bumpJson(p, fn) {
  const json = JSON.parse(read(p));
  fn(json);
  write(p, `${JSON.stringify(json, null, 2)}\n`);
}

bumpJson("package.json", (j) => {
  j.version = version;
});
bumpJson("package-lock.json", (j) => {
  j.version = version;
  if (j.packages?.[""]) j.packages[""].version = version;
});
bumpJson("src-tauri/tauri.conf.json", (j) => {
  if (!String(j.version).endsWith("package.json")) j.version = version;
});

// Replace and fail loudly when the pattern did not match (a silent miss ships a stale version).
function replaceOrDie(p, re, replacement) {
  const before = read(p);
  const after = before.replace(re, replacement);
  if (after === before && !re.test(before)) die(`${p}: version pattern not found, nothing changed`);
  write(p, after);
}
replaceOrDie("src-tauri/Cargo.toml", /^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
// Keep Cargo.lock in sync for the root package.
replaceOrDie(
  "src-tauri/Cargo.lock",
  /(name = "lichtwart-demo-simulator"\r?\nversion = )"[^"]+"/,
  `$1"${version}"`,
);

const date = new Date().toISOString().slice(0, 10);
let log = read("CHANGELOG.md");
log = log.replace("## [Unreleased]", `## [Unreleased]\n\n## [${version}] - ${date}`);
write("CHANGELOG.md", log);

console.log(`Bumped to ${version}. Run "node scripts/release-check.mjs v${version}", review the diff, then:
  git commit -am "chore: release v${version}" && git tag -a v${version} -m "v${version}" && git push --follow-tags`);
