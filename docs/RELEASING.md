# Releasing

The app version is also the library version (PRD 11.1). One SemVer number covers both.

## One-time setup

### 1. Repository URL

Replace `OWNER/REPO` in `src-tauri/tauri.conf.json` (`plugins.updater.endpoints`) with the real
GitHub path. Until then the in-app update check finds nothing.

**This is a placeholder today.** `node scripts/release-check.mjs vX.Y.Z` (run first in the Release
workflow) fails while `OWNER/REPO` or the `TODO` public key is still in `tauri.conf.json`, and
when the endpoint does not match `GITHUB_REPOSITORY`. A tag cannot produce a release before both
are filled in. CI (no tag) only prints a warning; `node scripts/release-check.mjs --strict` fails
on it too.

### 2. Updater signing keys

```sh
npm run tauri signer generate -- -w ~/.tauri/lichtwart-simulator.key
```

- Put the **public key** (the `.key.pub` content) into `plugins.updater.pubkey` in
  `src-tauri/tauri.conf.json`. It must be the base64 text of the minisign public key as printed
  in the file (the release check decodes it and fails on the `TODO` placeholder).
- Keep the private key and its password safe. If you lose them, installed apps can no longer
  update and users must install a new build by hand.

### 3. GitHub secrets (Settings > Secrets and variables > Actions)

| Secret | Content |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | content of the private updater key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password |
| `APPLE_CERTIFICATE` | "Developer ID Application" certificate exported as `.p12`, base64: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | password of the `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Lichtwart GmbH (XD59B34DMF)` |
| `APPLE_API_KEY` | App Store Connect API key id |
| `APPLE_API_ISSUER` | App Store Connect issuer id |
| `APPLE_API_KEY_P8` | the `AuthKey_<id>.p8` file, base64 |

The six `APPLE_*` secrets are optional as a set: without any of them the macOS build is ad-hoc
signed and not notarized (the workflow prints a warning). macOS then blocks the first open of the
downloaded app: System Settings > Privacy & Security > "Open Anyway" (or
`xattr -dr com.apple.quarantine "/Applications/Lichtwart Demo Simulator.app"`). In-app updates
still work, they are verified with the updater key. Setting only some of them fails the build.

`GITHUB_TOKEN` is provided by GitHub. The Apple Team ID is part of `APPLE_SIGNING_IDENTITY`; the
workflow does not need it separately (API-key notarization does not use it).

### 4. Protect the signing secrets

- Create a GitHub environment named `release` (Settings > Environments), add **required
  reviewers**, and store the secrets above in that environment. The `build` job uses it, so a
  pushed `v*` tag waits for approval before any secret is available.
- Add a tag ruleset (Settings > Rules) that restricts who can create `v*` tags.
- The workflow also refuses tags whose commit is not on `main`.
- The workflow stops early with a clear message when a secret is empty.

## Local builds and the release overlay

`createUpdaterArtifacts` is `false` in `src-tauri/tauri.conf.json`, so `npm run tauri build` works on
a laptop without `TAURI_SIGNING_PRIVATE_KEY`. The Release workflow passes
`--config src-tauri/tauri.release.conf.json` (`bundle.createUpdaterArtifacts: true`) to get the signed
updater files and `latest.json`. `release-check.mjs` fails a tag when the overlay or that flag in the
workflow is missing. To try a signed build locally, export `TAURI_SIGNING_PRIVATE_KEY` (and its
password) and add the same `--config` argument.

## Cut a release

1. Move your notes into `## [Unreleased]` in `CHANGELOG.md`.
2. `node scripts/release.mjs 0.2.0` (for the first release: `0.1.0`, same version is fine). It
   must run on `main` with a clean working tree, and fails when `[Unreleased]` is empty or the
   version already has a section. It bumps `package.json`, `package-lock.json`,
   `src-tauri/Cargo.toml` (+ `Cargo.lock`), `tauri.conf.json` (if it holds a literal version) and
   turns "Unreleased" into a dated section (UTC date). It stops when a version pattern does not match.
3. Run `npm run check` and `node scripts/release-check.mjs v0.2.0`, then commit:
   `git commit -am "chore: release v0.2.0"`.
4. Tag with an **annotated** tag (`--follow-tags` skips lightweight tags):
   `git tag -a v0.2.0 -m "v0.2.0" && git push --follow-tags`.
5. The `Release` workflow:
   1. `verify`: the tag is on `main`, `release-check.mjs` passes (versions in all files, CHANGELOG
      section, no updater placeholders), and the notes are cut from the CHANGELOG.
   2. `create-release`: one **draft** release with the notes.
   3. `build` (needs approval of the `release` environment): universal macOS `.dmg` (signed and
      notarized, or ad-hoc signed without the Apple secrets) and Windows NSIS installer, signed updater artifacts and `latest.json`, all
      uploaded into the draft.
   4. `publish`: only when all builds succeeded, the draft is published (and marked latest).
   If a build fails, the draft stays unpublished: fix the problem, delete the draft release and the
   tag, and tag again. Versions with a pre-release suffix (`0.2.0-beta.1`) are published as
   pre-releases and never become "latest", so the updater ignores them.

CI also runs `node scripts/check-tauri-versions.mjs` (Tauri crate and npm package must match in
major.minor, otherwise `tauri build` refuses to run) and `node scripts/release-check.mjs` without a
tag. Both are part of `npm run check` (`npm run check:versions`). The `@tauri-apps/*` runtime
packages are pinned with `~` in `package.json`; update crate and package together.

Installed apps read `releases/latest/download/latest.json` on start and offer the update.

## Windows SmartScreen

The Windows installer is not code-signed. SmartScreen shows "Windows protected your PC". Users
click "More info", then "Run anyway". This is accepted for now (PRD 15). Updates installed by the
in-app updater are verified by the updater signature and do not show this warning.
