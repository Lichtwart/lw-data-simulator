# Phase 3 review — CI / release setup

Scope: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `scripts/release*.mjs`,
`src-tauri/tauri.conf.json`, `src-tauri/icons/`, `CHANGELOG.md`, `docs/RELEASING.md`.
Checked against PRD §4, §11.1, §15, §18, `package.json`, `src-tauri/Cargo.toml`.

Tools run: `actionlint 1.7.12` with shellcheck, on both workflows. It found nothing. Note that
actionlint does not know tauri-action's inputs, so it cannot catch #3 or #9. `npx tauri info`
reproduces #1. The tauri-action input list was checked against
`tauri-apps/tauri-action@v0/action.yml`.

Severity: **High** = a release breaks or users get a broken update; **Medium** = a real defect or
security gap; **Low** = hardening or cleanup.

---

## High

### 1. `src-tauri/Cargo.toml:32` — dialog plugin crate and npm package versions differ (confirmed)
`npx tauri info` prints: `Found version mismatched Tauri packages ... tauri-plugin-dialog (v2.7.3) :
@tauri-apps/plugin-dialog (v2.8.2)`. `tauri build` refuses to run because of this, so the Release
workflow (tauri-action runs `tauri build`) fails too. The cause: `tauri-plugin-dialog = "2"` lets
Cargo.lock stay on 2.7.3, while `package.json:26` asks for `^2.8.2`. crates.io has 2.8.2.
Other pairs are fine: tauri 2.12.3 / api 2.12.3, updater 2.13.3 / 2.13.3, process 2.4.0 / 2.4.0.
cli 2.12.2 is fine because only major.minor of tauri vs api is compared. `tauri-plugin-log` 2.10.1
has no npm package, which is fine because it is used from Rust only (see #22).
`tauri-plugin-fs` 2.5.2 comes in through dialog and has no npm package, which is also fine.
**Fix:** in `Cargo.toml` set `tauri-plugin-dialog = "2.8"`, then run
`cargo update -p tauri-plugin-dialog --manifest-path src-tauri/Cargo.toml`. That gives 2.8.2 and
also updates tauri-plugin-fs. To stop this from coming back, pin the npm plugins to a minor range
(`"~2.8.2"`, `"~2.13.3"`, `"~2.4.0"`, `"~2.12.3"` for api) so `npm install` cannot move the minor
alone, and add the CI gate from #10.

### 2. `release.yml:106` + matrix — the release is published before all platforms are built; the two jobs race
`releaseDraft: false` while macOS and Windows build in parallel means:
(a) Whichever job finishes first publishes a public release with a `latest.json` that lists only
its own platform. Installed apps on the other OS see no update, or later get a half-written
manifest.
(b) If one job fails, a public "latest" release stays up with missing assets, and
`releases/latest/download/latest.json` points to it.
(c) Both jobs may try to create the release at the same moment, and one fails with
`already_exists`.
**Fix:** use three jobs. First, `create-release`: `gh release create "$TAG" --draft --title ...
--notes-file release-notes.md`, and output the release id. Second, the `build` matrix with
tauri-action `releaseId: ${{ needs.create-release.outputs.id }}`. Third, `publish`, with
`needs: build`: `gh release edit "$TAG" --draft=false --latest`. The tauri-action README shows this
pattern (`releaseId` + a separate publish step).

### 3. `release.yml:105` — `releaseBodyPath` is not a tauri-action input
tauri-action@v0 accepts `releaseBody` but has no `releaseBodyPath`, so the input is ignored and the
release text is empty. The CHANGELOG notes never appear. RELEASING.md:49 promises they will.
**Fix:** in `verify`, write the notes to a multiline output
(`{ echo 'notes<<EOF'; node scripts/release-notes.mjs "$V"; echo EOF; } >> "$GITHUB_OUTPUT"`) and
pass `releaseBody: ${{ needs.verify.outputs.notes }}`. Better, create the release with
`gh release create --notes-file` as in #2 and drop the body from tauri-action.

### 4. `tauri.conf.json:63` and `:65` — the updater pubkey and endpoint are still placeholders, and nothing blocks a release
`"pubkey": "TODO_REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY"` and
`https://github.com/OWNER/REPO/...` would ship as they are. The build still succeeds, because the
pubkey is only read when the app checks for updates. Every installed copy would then fail its
update check for good, so users have to reinstall by hand. The repo has no git remote configured
yet either.
**Fix:** fill in both values as RELEASING.md §1–2 says. Then add a guard to
`scripts/release-check.mjs`: fail if `plugins.updater.pubkey` contains `TODO` or does not decode
from base64 as a minisign key, or if any endpoint contains `OWNER/REPO`. Also check that the
endpoint matches `https://github.com/${GITHUB_REPOSITORY}/releases/latest/download/latest.json`
when `GITHUB_REPOSITORY` is set.

### 5. `ci.yml:37` — the Linux apt line installs two packages that conflict
On Ubuntu, `libappindicator3-dev` and `libayatana-appindicator3-dev` cannot be installed together
(the ayatana package conflicts with or replaces the old one), so `apt-get install` fails and the
whole `check` job is red. The app has no tray icon, so it needs neither package. `patchelf` is only
needed for AppImage bundling, which CI does not do.
**Fix:** use the list from the Tauri 2 prerequisites:
`sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev
libayatana-appindicator3-dev librsvg2-dev`. Drop `libappindicator3-dev` and `patchelf`.
(Run it once on a PR to confirm.)

---

## Medium

### 6. `scripts/release.mjs:53`, `docs/RELEASING.md:45` — `git push --follow-tags` does not push lightweight tags
`git tag v0.2.0` creates a lightweight tag. `--follow-tags` only pushes annotated tags, so the tag
stays local and the Release workflow never starts.
**Fix:** use `git tag -a v0.2.0 -m "v0.2.0" && git push --follow-tags` (or
`git push origin main v0.2.0`) in both places.

### 7. `ci.yml` (top level) — no `permissions` block
Without one, `GITHUB_TOKEN` gets the repository default, which can be read/write. In a public repo
the CI token should be read-only.
**Fix:** add at the top level:
```yaml
permissions:
  contents: read
```

### 8. `release.yml:7-8` — `contents: write` is granted to every job
The `verify` job only reads.
**Fix:** set `permissions: contents: read` at workflow level, and give `contents: write` only to the
jobs that create, upload or publish the release (`build`, and `create-release`/`publish` from #2).

### 9. `release.yml:108` — `uploadUpdaterJson` is not a tauri-action input
The real name is `includeUpdaterJson`. It defaults to `true`, so `latest.json` is uploaded anyway.
But the line does nothing and suggests a setting that is not there.
**Fix:** rename it to `includeUpdaterJson: true`. `updaterJsonPreferNsis: true` is valid; keep it.

### 10. `ci.yml` — CI never runs what the release runs, so release-only failures (like #1) first show up at tag time
CI never calls `tauri build`, never checks crate/npm version pairs, and never runs
`release-check.mjs`. Note: `npx tauri info` exits **0** even when it reports the mismatch (checked
locally), so it cannot serve as a gate.
**Fix:**
(a) Add `scripts/check-tauri-versions.mjs`. It reads Cargo.lock and package-lock.json and, for each
pair (`tauri`/`@tauri-apps/api`, `tauri-plugin-X`/`@tauri-apps/plugin-X`), fails when major.minor
differ. Run it in `check` and in `npm run check`.
(b) Add a "no tag" mode to `release-check.mjs`: compare package.json, Cargo.toml, Cargo.lock and
tauri.conf.json with each other, and run the placeholder check from #4. Call it in CI.
(c) Optionally, in the macOS matrix leg, run `npx tauri build --debug --no-bundle` (or a full
`tauri build --bundles app` without signing on `push` to main only).

### 11. `ci.yml:57-76` — macOS and Windows code is only `cargo check`ed, never linted or tested
Clippy and `cargo test` run only on Linux, which is not a ship target. The keyring stores under
`[target.'cfg(target_os = "macos"|"windows")']` (Cargo.toml:45-49) and the code that uses them
never get clippy `-D warnings` or tests.
**Fix:** in the `cargo-check` matrix, replace `cargo check` with `npm run rust:clippy` and
`npm run rust:test`, and add `components: clippy` to the toolchain step. If needed, keep Linux for
fmt, the bindings drift check and the JS steps only.

### 12. `release.yml:78` — the keychain import uses `-t cert -A` for a `.p12`
For a PKCS#12 bundle, `-t cert` tells `security` the item is a certificate only. Depending on the
macOS version this can import the certificate without its private key, and codesign then fails
with "no identity found". `-A` also lets every app use the key without asking.
**Fix:** `security import "$CERT_PATH" -k "$KEYCHAIN_PATH" -P "$APPLE_CERTIFICATE_PASSWORD" -f pkcs12
-T /usr/bin/codesign -T /usr/bin/security`. Simpler: delete the whole step and pass
`APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` in tauri-action's `env`. The Tauri 2 bundler
reads both and imports them into a temporary keychain itself. It also reads
`APPLE_SIGNING_IDENTITY`, `APPLE_API_ISSUER`, `APPLE_API_KEY` and `APPLE_API_KEY_PATH`, and those
names are already correct.

### 13. `release.yml` — anyone with write access can push a `v*` tag and run signing with production secrets
**Fix:** move the Apple and updater secrets into a GitHub environment `release` with required
reviewers (`environment: release` on `build`). Add a tag ruleset that limits who can create `v*`
tags. Optionally fail in `verify` when the tagged commit is not on `main`
(`git merge-base --is-ancestor "$GITHUB_SHA" origin/main`, after `fetch-depth: 0`).

---

## Low

### 14. `release.yml:101` — `APPLE_TEAM_ID` is not used
Tauri reads `APPLE_TEAM_ID` only for Apple-ID notarization (`APPLE_ID` + `APPLE_PASSWORD`). The API
key route (`APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH`) does not use it.
**Fix:** remove it, or leave it with a comment saying it is informational. It is harmless either
way.

### 15. `release.yml:70-91` — no fail-fast when a secret is missing
An empty `APPLE_CERTIFICATE` or `APPLE_API_KEY_P8` decodes to an empty file, and the build then
fails much later with an unclear codesign or notarytool error.
**Fix:** start each step with
`: "${APPLE_CERTIFICATE:?secret APPLE_CERTIFICATE missing}"` (and the same for the other secrets).
Do the same for `TAURI_SIGNING_PRIVATE_KEY`. No secret is echoed today; keep it that way.

### 16. `scripts/release-check.mjs:9-15` — Cargo.lock and package-lock.json are not checked
`release.mjs:40` updates Cargo.lock with a regex. If the regex misses (formatting, renamed
package), the change silently does nothing and the release builds with a stale lock.
**Fix:** in release-check, also read the `lichtwart-demo-simulator` entry in `src-tauri/Cargo.lock`
and `packages[""].version` in package-lock.json. In `release.mjs`, check that each `.replace` made a
change (compare before and after) and exit 1 if not. Consider `cargo ... --locked` in the CI
clippy/test steps so a stale Cargo.lock fails CI.

### 17. `scripts/release.mjs` — edge cases
(a) It does not check for a clean tree or for being on `main`. Add
`git status --porcelain` (empty) and `git rev-parse --abbrev-ref HEAD` (= `main`) checks.
(b) Running it twice, or with a version that already has a section, adds a duplicate
`## [x.y.z]`. Exit if `## [${version}]` already exists.
(c) An empty `[Unreleased]` section produces an empty release. Warn or fail when there is no text
between `## [Unreleased]` and the next `## [`.
(d) It allows pre-release versions (`-beta.1`), but `release.yml:107` always sets
`prerelease: false`, so a beta becomes "latest" and auto-updates every user. Use
`prerelease: ${{ contains(github.ref_name, '-') }}`, or refuse pre-release versions in the regex.
(e) The date is UTC (`toISOString`), which is fine; just know that a release cut after midnight
local time on the evening before can carry the previous day's date.

### 18. `ci.yml:52-55` — the bindings drift check
It works without a display. `tests::export_bindings` only calls specta's export and never starts a
Tauri runtime. It does need the webkit dev libraries to link, and those are installed.
Two gaps:
(a) `git diff --exit-code` ignores **untracked** files, so a missing (never committed)
`src/lib/bindings.ts` passes. Use
`test -z "$(git status --porcelain -- src/lib/bindings.ts)" || { git diff -- src/lib/bindings.ts; exit 1; }`.
(b) The step is redundant: `npm run rust:test` (line 50) already runs `export_bindings`. Either
drop the `gen:types` step or keep it only for readability.

### 19. `ci.yml:19,21,26,30`, `release.yml:16,17,27,44,46,50,55,59,93` — actions are pinned to moving tags
Major tags are acceptable, but for a public repo whose release job holds signing secrets, pin
third-party actions by SHA (at least `tauri-apps/tauri-action`, `dtolnay/rust-toolchain` and
`swatinem/rust-cache`), with the tag in a comment. Add `.github/dependabot.yml` with
`package-ecosystem: github-actions` (and `npm` / `cargo`) to keep them current.

### 20. `ci.yml:30,71`, `release.yml:59` — cache settings
(a) Every PR saves a cache of several GB, and the repo's 10 GB cache limit evicts main's cache
quickly. Add `save-if: ${{ github.ref == 'refs/heads/main' }}` to `swatinem/rust-cache`.
(b) In `release.yml`, use `save-if: false`, or drop the cache. Release builds should not write
caches, and LTO with `codegen-units = 1` gains little from a cache anyway.

### 21. `ci.yml:8-10` — concurrency also cancels runs on `main`
Two quick pushes to main cancel the first run, so that commit never gets a result.
**Fix:** `cancel-in-progress: ${{ github.event_name == 'pull_request' }}`.

### 22. `src-tauri/capabilities/default.json` — `log:default` is granted but has no JS client
`@tauri-apps/plugin-log` is not installed, so nothing in JS calls it. The permission only opens an
IPC command that nothing uses.
**Fix:** remove `log:default`, or install `@tauri-apps/plugin-log` at `~2.10` if the frontend
should log through it. If you install it, keep the minor in step with the crate (#1).

### 23. `release.yml:26` — a step output is written straight into a shell command
`${{ steps.v.outputs.version }}` is safe today only because `release-check.mjs` validates the tag
first. Best practice is to pass it through `env:` (`env: { V: ... }` and then `"$V"`).

### 24. `ci.yml:17`, `release.yml:12` — `ubuntu-22.04`
This works (webkit2gtk-4.1 is available), but 22.04 is the older LTS image and will be retired
first. Use `ubuntu-24.04`.

---

## Checked and fine
- tauri-action for Tauri 2: `args: --target universal-apple-darwin` is correct, and both Rust
  targets (`aarch64-apple-darwin,x86_64-apple-darwin`) are installed on the macOS leg. The universal
  build writes both `darwin-aarch64` and `darwin-x86_64` into `latest.json`. On Windows, `targets: ''`
  is fine.
- macOS notarization variable names (`APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH`,
  `APPLE_SIGNING_IDENTITY`) are the ones Tauri 2 reads. `APPLE_API_KEY_PATH` reaches tauri-action
  through `$GITHUB_ENV`. The `.p12` file is deleted after import. No secret is printed.
- Fork PRs: `ci.yml` uses `pull_request` (not `pull_request_target`) and no secrets, so it is safe.
  `release.yml` runs only on tag push.
- `tauri.conf.json`: `"version": "../package.json"` is supported, and both scripts handle it.
  `createUpdaterArtifacts: true` (v2 format) works with the `app`, `dmg` and `nsis` targets; the
  bundler skips targets that do not fit the current OS. `hardenedRuntime`,
  `minimumSystemVersion 11.0`, `category`, `publisher`, NSIS `currentUser` + updater
  `windows.installMode: passive`, and the `../library/` resource (PRD §11.1) are all valid.
- Icons: the desktop set is complete. `icon.ico` holds 16, 24, 32, 48, 64 and 256. `icon.icns`
  holds 16 through 1024 (ic07 to ic14). The PNG sizes match their names. This is the Lichtwart
  mark, not the Tauri default.
- `validate:library` validates `library` and the test fixtures against `library/schema` (PRD §18),
  and fails on files outside the known folders.
- `release-notes.mjs` cuts the right section, stopping at the next `## [` or the end of the file.
- `release-check.mjs` checks the tag format and that package.json, Cargo.toml and tauri.conf.json
  match the tag, and that a CHANGELOG section exists (see #16 for what it misses).

---

## Resolution

All 24 findings are addressed. actionlint is clean; `npx tauri info` shows no version mismatch.
`tauri build --debug --no-bundle` could not be verified: the Rust tree currently fails to compile
because of in-progress engine work in `src-tauri/src/` (`MAX_SEND_INTERVAL_SEC` missing in
`sim::scheduler`), not because of these changes.

| # | Status | Note |
|---|---|---|
| 1 | fixed | `tauri-plugin-dialog = "2.8"`, lock now 2.8.2 (fs 2.7.0). npm `@tauri-apps/*` runtime packages pinned with `~`. New `scripts/check-tauri-versions.mjs`, in CI and `npm run check` (`check:versions`). |
| 2 | fixed | Jobs `verify` > `create-release` (draft) > `build` (matrix, `releaseId`) > `publish` (only after all builds). |
| 3 | fixed | Notes go into the draft via `gh api ... body=@release-notes.md`; `releaseBodyPath` removed. |
| 4 | fixed (user input needed) | `release-check.mjs` fails in release mode while the pubkey is `TODO`/not a minisign key or an endpoint has `OWNER/REPO`, and checks the endpoint against `GITHUB_REPOSITORY`. Values themselves are still placeholders. No-tag mode (CI) warns; `--strict` fails. |
| 5 | fixed | apt list from the Tauri 2 prerequisites. |
| 6 | fixed | Annotated tag in script output and RELEASING.md. |
| 7 | fixed | `permissions: contents: read` in ci.yml. |
| 8 | fixed | Workflow default read; write only on `create-release`, `build`, `publish`. |
| 9 | fixed | `includeUpdaterJson: true`. |
| 10 | fixed | (a) version-pair script, (b) no-tag `release-check.mjs`, (c) macOS leg runs `npx tauri build --debug --no-bundle`; `cargo fetch --locked` guards a stale Cargo.lock. |
| 11 | fixed | macOS and Windows run clippy and tests. |
| 12 | fixed | Keychain step removed; `APPLE_CERTIFICATE` and password passed to tauri-action. |
| 13 | fixed (user setup needed) | `environment: release` on `build`, tag-on-main check in `verify`. The environment with reviewers and the tag ruleset must be created in GitHub (documented). |
| 14 | fixed | `APPLE_TEAM_ID` removed. |
| 15 | fixed | "Check secrets are set" step. |
| 16 | fixed | release-check reads Cargo.lock and package-lock.json; `release.mjs` fails when a pattern does not match. |
| 17 | fixed | (a) clean tree and `main`, (b) duplicate section, (c) empty Unreleased, (d) pre-releases become GitHub pre-releases and are never `latest`, (e) documented as UTC. |
| 18 | fixed | Drift check uses `git status --porcelain`; the redundant `gen:types` step is dropped (`rust:test` regenerates the file). Note: `src/lib/bindings.ts` must be committed. |
| 19 | fixed | tauri-action, rust-toolchain, rust-cache pinned by SHA; `.github/dependabot.yml` added. |
| 20 | fixed | `save-if` main only (CI), `save-if: false` (release). |
| 21 | fixed | `cancel-in-progress` only for pull requests. |
| 22 | fixed | `log:default` removed from the capability (log plugin stays Rust-only). |
| 23 | fixed | Tag and version passed through `env`. |
| 24 | fixed | `ubuntu-24.04`. |

Also done (frontend contract requests 8 and 9, finding 36 of phase2-frontend): capability
`core:window:allow-request-user-attention` added; CSP `img-src` now allows
`https://tile.openstreetmap.org` and `OSM_TILES` uses the bare host. Tile loading with a Referer
from `tauri://localhost` still has to be checked on a release build.
