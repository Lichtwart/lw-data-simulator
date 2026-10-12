// Usage:
//   node scripts/release-check.mjs v0.1.0   release mode: everything must match the tag and the
//                                           updater placeholders must be filled in
//   node scripts/release-check.mjs           no-tag mode (CI): all version files must agree with
//                                           each other; placeholders are only warned about
//                                           (add --strict to fail on them)
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const strict = args.includes("--strict");
const tag = args.find((a) => !a.startsWith("--")) ?? "";
const releaseMode = tag !== "";
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

const pkg = JSON.parse(read("package.json")).version;
const cargo = read("src-tauri/Cargo.toml").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
// tauri.conf.json may point at package.json ("../package.json"): then it is in sync by design.
const tauri = String(conf.version).endsWith("package.json") ? pkg : conf.version;
const cargoLock = read("src-tauri/Cargo.lock").match(
  /name = "lichtwart-demo-simulator"\r?\nversion = "([^"]+)"/,
)?.[1];
const npmLock = JSON.parse(read("package-lock.json")).packages?.[""]?.version;

const found = {
  "package.json": pkg,
  "package-lock.json": npmLock,
  "Cargo.toml": cargo,
  "Cargo.lock": cargoLock,
  "tauri.conf.json": tauri,
};
const expected = releaseMode ? tag.replace(/^v/, "") : pkg;

let ok = true;
const fail = (msg) => {
  ok = false;
  console.error(msg);
};

if (releaseMode && !/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(tag)) {
  fail(`Tag "${tag}" is not vMAJOR.MINOR.PATCH`);
}
if (!SEMVER.test(expected)) fail(`Version "${expected}" is not MAJOR.MINOR.PATCH`);
for (const [file, v] of Object.entries(found)) {
  if (v !== expected) {
    fail(`${file} has version ${v}, ${releaseMode ? "tag" : "package.json"} expects ${expected}`);
  }
}
if (releaseMode && !read("CHANGELOG.md").includes(`## [${expected}]`)) {
  fail(`CHANGELOG.md has no "## [${expected}]" section`);
}

// Local builds must work without signing keys: the updater artifacts are switched on by an
// overlay config that only the release workflow passes (`--config`).
if (releaseMode) {
  if (conf.bundle?.createUpdaterArtifacts) {
    fail("tauri.conf.json must not set bundle.createUpdaterArtifacts (local builds need no keys)");
  }
  let overlay = null;
  try {
    overlay = JSON.parse(read("src-tauri/tauri.release.conf.json"));
  } catch {}
  if (overlay?.bundle?.createUpdaterArtifacts !== true) {
    fail("src-tauri/tauri.release.conf.json must set bundle.createUpdaterArtifacts to true");
  }
  if (
    !read(".github/workflows/release.yml").includes("--config src-tauri/tauri.release.conf.json")
  ) {
    fail("the release workflow must build with --config src-tauri/tauri.release.conf.json");
  }
}

// Updater placeholders: a build with these ships an app that can never update.
const placeholders = [];
const updater = conf.plugins?.updater ?? {};
const pubkey = String(updater.pubkey ?? "");
if (!pubkey || /TODO/i.test(pubkey)) {
  placeholders.push("plugins.updater.pubkey is a placeholder (see docs/RELEASING.md, step 2)");
} else {
  const decoded = Buffer.from(pubkey, "base64").toString("utf8");
  if (!/^untrusted comment:.*public key/im.test(decoded)) {
    placeholders.push("plugins.updater.pubkey is not a base64-encoded minisign public key");
  }
}
const endpoints = updater.endpoints ?? [];
if (endpoints.length === 0) placeholders.push("plugins.updater.endpoints is empty");
for (const e of endpoints) {
  if (/OWNER\/REPO/.test(e)) {
    placeholders.push(`updater endpoint still contains OWNER/REPO (see docs/RELEASING.md, step 1)`);
  }
}
const repo = process.env.GITHUB_REPOSITORY;
if (repo && releaseMode) {
  const want = `https://github.com/${repo}/releases/latest/download/latest.json`;
  if (!endpoints.includes(want)) placeholders.push(`updater endpoints must contain ${want}`);
}
for (const p of placeholders) {
  if (releaseMode || strict) fail(p);
  else console.warn(`warning: ${p}`);
}

if (!ok) process.exit(1);
console.log(`Version ${expected} is consistent${releaseMode ? "" : " (no-tag mode)"}.`);
