// Usage: node scripts/check-tauri-versions.mjs
// `tauri build` refuses to run when a Tauri crate and its npm package differ in major.minor
// (`tauri info` prints the mismatch but exits 0). Check the locked versions of each pair:
//   tauri            <-> @tauri-apps/api
//   tauri-plugin-X   <-> @tauri-apps/plugin-X   (only when both sides exist)
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

const crates = new Map();
for (const block of read("src-tauri/Cargo.lock").split("[[package]]")) {
  const name = block.match(/^name = "([^"]+)"/m)?.[1];
  const version = block.match(/^version = "([^"]+)"/m)?.[1];
  if (name && version && (name === "tauri" || name.startsWith("tauri-plugin-"))) {
    crates.set(name, version);
  }
}

const lock = JSON.parse(read("package-lock.json")).packages ?? {};
const npm = new Map();
for (const [key, meta] of Object.entries(lock)) {
  const m = key.match(/^node_modules\/(@tauri-apps\/(?:api|plugin-[a-z0-9-]+))$/);
  if (m) npm.set(m[1], meta.version);
}

const minor = (v) => v.split(".").slice(0, 2).join(".");
let ok = true;
let pairs = 0;
for (const [pkg, npmVersion] of npm) {
  const crate = pkg === "@tauri-apps/api" ? "tauri" : pkg.replace("@tauri-apps/", "tauri-");
  const crateVersion = crates.get(crate);
  if (!crateVersion) {
    console.error(`${pkg} ${npmVersion} has no Rust crate "${crate}" in Cargo.lock`);
    ok = false;
    continue;
  }
  pairs++;
  if (minor(crateVersion) !== minor(npmVersion)) {
    ok = false;
    console.error(
      `Version mismatch: ${crate} ${crateVersion} (Cargo.lock) vs ${pkg} ${npmVersion} (package-lock.json)`,
    );
  }
}
if (!ok) process.exit(1);
console.log(`Tauri crate/npm versions match (${pairs} pairs).`);
