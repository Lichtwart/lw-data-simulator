# Lichtwart Demo Simulator

Desktop app (Tauri 2, macOS + Windows) that simulates Lichtwart Core v2 devices in
Cumulocity for sales demos: demo devices, demo packs from a bundled library, live value /
scenario / fault control, operation handling, raw fire and export back to the library.

- Spec: [`docs/PRD.md`](docs/PRD.md)
- Architecture + IPC contract: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- Conventions for contributors and coding agents: [`AGENTS.md`](AGENTS.md)

## Requirements

- Node 24 or newer, npm (`engines` in `package.json`; CI uses Node 24)
- Rust 1.88 or newer (stable, edition 2024; the code uses let-chains) — `cargo`, `rustfmt`, `clippy`
- Platform prerequisites for Tauri 2: <https://v2.tauri.app/start/prerequisites/>

## Develop

```sh
npm install
npm run tauri dev        # app with hot reload (Vite on :1420)
npm run gen:types        # regenerate src/lib/bindings.ts after changing Rust IPC types
npm run check            # typecheck, lint, test, cargo fmt/clippy/test
npm run build            # frontend production build
npm run tauri build      # installable bundle for this machine (no signing keys needed)
npm run tauri build -- --debug --no-bundle   # quick compile check of the whole app
```

A local `tauri build` makes an unsigned bundle without updater artifacts. Signed, notarized
installers and the updater files (`latest.json`) come only from the release workflow, which adds
`--config src-tauri/tauri.release.conf.json` (see [`docs/RELEASING.md`](docs/RELEASING.md)).

## Before the first release

The updater is not usable yet: `src-tauri/tauri.conf.json` still holds a placeholder public key and
the repository path `OWNER/REPO`, and the project has no git remote. Until this is done every
update check fails (it is logged once on start and shown as "check failed" in Settings) and
`node scripts/release-check.mjs` refuses every tag. Only the repository owner can do these steps;
[`docs/RELEASING.md`](docs/RELEASING.md) has the details.

- [ ] Create the GitHub repository, add the remote, first commit on `main`.
- [ ] Replace `OWNER/REPO` in `plugins.updater.endpoints` (`src-tauri/tauri.conf.json`).
- [ ] Generate the updater key (`npm run tauri signer generate`) and put the public key into
      `plugins.updater.pubkey`. Keep the private key and its password safe.
- [ ] Add the signing and Apple secrets to a `release` environment with required reviewers, and a
      tag rule for `v*`.
- [ ] Replace the placeholder app icon (`npm run tauri icon <logo.png>`).
- [ ] Check on real tenants and devices (PRD §19): `description` in Device Management, floorplan
      plugin, DALI brightness %, the `"C"` temperature unit, Windows keychain and updater.

## Layout

```
src/         React + TypeScript UI (Tailwind, shadcn/ui, react-i18next)
src-tauri/   Rust backend: Cumulocity I/O, simulation engine, library, device management
library/     bundled demo library (items, demos, floorplans, schemas)
docs/        PRD, decisions, architecture, releasing, reviews, research
```
