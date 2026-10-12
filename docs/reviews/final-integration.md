# Final integration review

Date: 2026-10-12. Scope: the whole uncommitted tree, looking for cross-cutting problems that the
per-phase reviews (`docs/reviews/*.md`) could miss. Read-only: no source files were changed.

## 1. Build and launch results

| Step | Result |
|---|---|
| `npm run check` | exit 0. Vitest 14 files / 65 tests, cargo 245 tests, clippy and fmt clean, library valid. Two expected warnings from `release-check.mjs`: updater pubkey placeholder, endpoint `OWNER/REPO`. |
| `npm run build` | exit 0 (tsc + vite, 386 ms). |
| `npm run tauri build -- --debug --no-bundle` | exit 0, binary `src-tauri/target/debug/lichtwart-demo-simulator`, resources copied to `target/debug/library/`. |
| Launch for 9 s | Started and stayed up. No panic, no library or tenant load warning, no plugin init error, stderr empty. The only log line was `tauri_plugin_updater … ERROR update endpoint did not respond with a successful status code`. That is expected while the endpoint is `OWNER/REPO`, and the UI reports it as "check failed". |
| `npx knip` | 2 unused files, 1 unused devDependency, 15 unused exports (see F24). |
| `cargo machete` | Not installed. A manual grep found every Rust dependency in use. |

## 2. PRD coverage matrix

Legend: **impl** = implemented and checked in code; **partial**; **missing**. `rs/` = `src-tauri/src/`, `ui/` = `src/`.

### §5 Login and tenants
| Requirement | Status | Evidence |
|---|---|---|
| Tenants baked in (`t819292026` test, `t134875661` prod), no passwords | impl | `library/tenants.json`; `rs/lib.rs:79-84` |
| Basic auth + `UseXBasic: true`, user always `tenantId/user` | impl | `rs/c8y/client.rs:29-33,316-317` |
| Validate via `GET /user/currentUser` | impl | `rs/c8y/client.rs:427-430`; `rs/commands/auth.rs:63` |
| Remember me in the OS keychain per tenant, otherwise memory only | impl | `rs/c8y/credentials.rs`; `ui/features/auth/LoginScreen.tsx:189` |
| Switching tenant = logout + login | impl | `rs/commands/auth.rs:82,120-123`; `ui/app/AppShell.tsx:74` |

### §6 Demo devices
| Requirement | Status | Evidence / note |
|---|---|---|
| 6.1 device body (type, `c8y_DeviceTypes: [demo]`, hardware, `DEMO-<random>`, firmware `V 03.00.02`, 19 supported ops in PRD order, top-level description) | impl | `rs/payload/config.rs:20-153` |
| 6.1 full 13-field `lw_configCore` | impl | `rs/payload/config.rs:86-101`. `maxControlGears = max(55, lamps)`; documented in AGENTS.md only (F21) |
| 6.1 no `c8y_RequiredAvailability` | impl | test `rs/payload/config.rs:561` |
| 6.1 `lw_demo: {}` on create | partial | Left out when no pack is chosen. Same behaviour, harmless |
| 6.2 list only demo devices, grouped by parent group, full-text search | impl / partial | `rs/device/crud.rs:18-51`; `ui/features/devices/groupDevices.ts`. Grouping uses the first `assetParent` without checking it is a group (F20) |
| 6.2 create (lat/lng + map picker, parent group, optional pack), reuse, edit | impl | `ui/features/devices/DeviceFormDialog.tsx`, `MapPicker.tsx`; `rs/device/crud.rs:204-268` |
| 6.2 reset (stop session, clear ACTIVE/ACK alarms, optionally delete floorplans) | impl | `rs/device/crud.rs:284-297`; `rs/commands/devices.rs:95` |
| 6.2 delete after confirmation, incl. floorplans and binaries | impl | `rs/device/crud.rs:300-306`; `rs/device/floorplan.rs:126-171` |
| 6.3 `lw_demo` fields incl. `customSlaves` / `customItems` / register assignments (D6) | impl | `rs/model/device.rs`; `rs/device/demo_state.rs` |
| 6.3 `lw_demo.lock` / `lw_demo.snapshot` | partial (deviation) | Stored in separate `lw_demoLock` / `lw_demoSnapshot` fragments (`rs/device/demo_state.rs:25-29`). Not recorded as a decision (F10) |
| 6.3 setup written on change, values only on "Save snapshot"; advisory lock, 10 min TTL, "in use by X since" | impl | `rs/device/demo_state.rs:34-37,152-160`; `ui/features/devices/DeviceListScreen.tsx:176-200`; `ui/app/DeviceWorkspace.tsx:71,106` |

### §7 Native Core v2 simulation
| Requirement | Status | Evidence |
|---|---|---|
| 7.1 `rssi`; `T` unit `"C"` | impl | `rs/payload/naming.rs:54-59` |
| 7.2 DALI naming, four series, at most 6 lamps per message, 16 groups | impl | `rs/payload/naming.rs:28-37`; `rs/payload/measurement.rs:19,258-261`; `rs/payload/config.rs:158` |
| 7.2 `lw_switchGroupMode` effect + `lw_daliGroupChange {id, previousMode, newMode, reason}` | impl | `rs/sim/core.rs:937-995`; `rs/payload/event.rs:41-63` |
| 7.2 `lw_switchConverterState`; per-lamp fault menu and bus alarms with firmware texts | impl | `rs/sim/core.rs:998`; `rs/payload/alarm.rs:59-214`; `ui/features/control/FaultSections.tsx` |
| 7.2 optional sensors Lux / Motion | impl (code) | `rs/payload/naming.rs:40-47`. The library ships one brightness item and no motion item (F23) |
| 7.3 DI `Value<NN>` / `ImpulseCounter<NN>`, title only in config (D1) | impl | `rs/payload/naming.rs:50-51,85-92`; `rs/payload/config.rs:328-360` |
| 7.4 slave ids 20↓ / addresses 240↓, limits, `isSavingOnStorage: false`, `lw-demo`, `writeRegisters[]` | impl | `rs/payload/naming.rs:11-17,208-315`; `rs/payload/config.rs:211-267` |
| 7.4 custom alarm types (D2) | impl | `rs/payload/naming.rs:317-345` |
| 7.4 slave timeout fault + recover; `lw_modbusWrite` exact echo | impl | `rs/sim/faults.rs:110-137`; `rs/sim/core.rs:908-925`; `rs/sim/operations.rs:748-753` |
| 7.5 clear alarm by default, realistic mode skips it | impl / partial | `rs/sim/core.rs:908-925`. A realistic-mode recovery is undone at the next start (F5) |

### §8 Simulation engine
| Requirement | Status | Evidence |
|---|---|---|
| One session, "stop and switch?" | impl | `rs/commands/session.rs:38-49`; `ui/features/session/useStartSession.tsx` |
| Runs in Rust while the window is hidden; stops on quit | impl | `rs/lib.rs:98-112`; `rs/state.rs:162` (with timeout) |
| Immediate send + periodic send, max 899 s; `time` on everything; `measurementCollection` | impl | `rs/sim/scheduler.rs:21-25`; `rs/c8y/client.rs:63,302`; `rs/payload/measurement.rs:223-266` |
| Controls slider / switch / select / counter; four generators; manual override + resume | impl / partial | `ui/features/control/ItemControl.tsx`; `rs/sim/generators.rs`; `rs/sim/core.rs:720-727`. Generator UI state is stale (F3, F7, F8) |
| Multi-effect items; scenarios with `start` | impl | `rs/sim/core.rs:801-838`; `library/items/hvac.json` (`ac_kit_mode`); `ui/features/control/ControlPanel.tsx:136-158` |

### §9 Operations
| Requirement | Status | Evidence |
|---|---|---|
| Realtime subscription from Rust | impl | `rs/c8y/realtime.rs` |
| Config operations auto-SUCCESSFUL (`c8y_MeasurementRequestOperation` also sends all values) | impl | `rs/sim/operations.rs:33-41` |
| Visible operations: EXECUTING at once, prompt with summary + `by`, confirm / abort with reason, 60 s → "Timed out", auto-accept off by default | impl | `rs/sim/operations.rs:835-940`; `ui/features/session/OperationPrompts.tsx`. Abort always sends the default reason (F19) |
| FAILED ignored; stale PENDING > 60 s → "Simulator was offline" | impl | `rs/sim/operations.rs:312,500` (server clock) |

### §10 Raw fire
| Requirement | Status | Evidence |
|---|---|---|
| Measurement / alarm / event forms, raw JSON tab, history + resend, usable without a pack | impl | `ui/features/raw-fire/RawFirePanel.tsx`; `rs/raw.rs`; `ui/app/DeviceWorkspace.tsx:139` |

### §11 Library
| Requirement | Status | Evidence |
|---|---|---|
| Bundled, no runtime fetch; app version = library version; CHANGELOG | impl | `tauri.conf.json` resources; `rs/library/loader.rs:35`; `CHANGELOG.md` |
| `type` + `schemaVersion`, CI validation | impl | `scripts/validate-library.mjs`; `rs/library/schema_tests.rs`; `.github/workflows/ci.yml` |
| Layout, three demos (D4), item kinds, slave definition, registerType (D6) | impl | `library/`; `rs/model/library.rs`; `rs/library/validate.rs` |
| "Generate series" helper | impl | `rs/library/series.rs:29`; `ui/features/demo/ItemEditorDialog.tsx:104` (ARCHITECTURE still says "stub", F12) |
| Selection tags ∪ includes − excludes; namespace chips; AND search over title de+en, id, tags, slave name | impl | `rs/library/select.rs`; `ui/features/control/itemFilter.ts`, `FilterBar.tsx` |
| Load warnings visible | partial | `library_info` (with `warnings`) is never called by the UI (F17) |

### §12 Floorplans
| Requirement | Status | Evidence |
|---|---|---|
| Binary upload → `lw_floorplan` MO → childAdditions (with rollback) | impl | `rs/device/floorplan.rs:81-96` |
| Widget resolution (measurement, alarmTypes incl. timeout / all custom / device-level / DALI, DI title fallback, Modbus targets) | impl | `rs/payload/floorplan.rs` |
| Re-apply replaces the same name after confirmation; floorplan optional | impl | `ui/features/demo/ApplyDemoCard.tsx:161-191` (confirmation is a checkbox); `rs/device/apply.rs:229-240,337` |

### §13 Export
| Requirement | Status | Evidence |
|---|---|---|
| Folder in library layout, widgets mapped back, images downloaded, format in AGENTS.md, round-trip test | impl | `rs/device/export.rs`; `ui/features/demo/ExportCard.tsx`; `AGENTS.md` "Exporting a device by hand"; `rs/device/tests.rs:349` |

### §14 Upgrades
| Requirement | Status | Evidence |
|---|---|---|
| Detect by `libraryVersion`; preview of items and floorplans; keeps overrides; never silent | impl / partial | `rs/device/setup.rs:448-457`; `rs/device/upgrade.rs`; `ui/features/demo/UpgradeCard.tsx`. Ignores another presenter's lock (F6) |

### §15 App updates and distribution
| Requirement | Status | Evidence |
|---|---|---|
| Actions on tag; universal signed + notarized dmg; unsigned NSIS; `latest.json` | impl (config) | `.github/workflows/release.yml`; `tauri.conf.json`; `docs/RELEASING.md`. Untested until secrets exist |
| Update check on start, badge, download + install + restart | partial | `ui/features/updates/useUpdateCheck.ts`, `UpdateBadge.tsx`. Cannot work: pubkey and endpoint are placeholders (F1) |

### §16 Activity log
| Requirement | Status | Evidence |
|---|---|---|
| Last 500 in memory, time / method / path / status / short body, copy as curl with masked password | impl | `rs/c8y/activity.rs`; `ui/features/activity/ActivityLogPanel.tsx` |

### §17 Prototype migration (spot check)
| Requirement | Status | Evidence |
|---|---|---|
| Lights → DALI, 10 binaries → DI `Value01..10`, numbers → Modbus slaves, custom alarms → `alarms[]` / `customAlarm`, AC mode multi-effect | impl | `library/items/*.json`. DI titles differ slightly from `docs/research/prototype-inventory.md` (F22) |

### §18 Testing
| Requirement | Status | Evidence |
|---|---|---|
| Payload naming, DI config/title mapping, slave assignment, custom alarm rules, generators, operations, export ↔ apply | impl | Tests in `rs/payload/*`, `rs/sim/generators.rs`, `rs/sim/operations.rs`, `rs/sim/tests.rs`, `rs/device/tests.rs` |
| CI: schema, Biome, clippy, cargo test, typecheck; fake Cumulocity integration test | impl | `.github/workflows/ci.yml`; `rs/c8y/fake.rs`, `rs/c8y/integration_tests.rs` |

### §19 Open points
`description` vs `c8y_Notes`: still open, needs a real tenant. Map tiles: OSM via Leaflet, decided. DALI brightness % vs arc level: still open, needs a real device. Analog inputs: non-goal.

### Decisions
| Decision | Status | Evidence |
|---|---|---|
| D1 native DI names | impl | `rs/payload/naming.rs:85-92`; `rs/payload/config.rs:328-353`; `rs/payload/floorplan.rs:216-222` |
| D2 several slave alarms + device-level alarms | impl | `rs/payload/naming.rs:317-345`; `rs/payload/alarm.rs:315-345` |
| D3 10 binaries → native DI | impl | `library/items/digital-inputs.json` |
| D4 `restaurant` demo | impl | `library/demos/restaurant.json` |
| D5 research-review decisions (core config, firmware version, `"C"`, `time`, 899, FAILED ignored, writeRegisters, exact echo) | impl | see §6.1, §7, §8, §9 rows |
| D6 device-local setup | impl / partial | `rs/commands/devices.rs`; `rs/payload/naming.rs:260-315`. Live-session edge cases in F4, F7, F11 |

## 3. End-to-end contract

- **Commands**: all 51 commands in `src/lib/bindings.ts` are registered in Rust and do real work. `AppError::NotImplemented` is never returned (only declared, F25). Every command except `libraryInfo` and `getDemo` is called by the UI (F17). Argument names and types match; `tsc` passes against the generated phased types.
- **Events**: all 10 events have listeners (`SessionProvider`, `GlobalEvents`, `AuthProvider`, `ActivityLogPanel`, `RawFirePanel`).
- **Revisions**: consistent. Snapshots for the same device are dropped only when older. Stop and fatal exits emit `session-changed(null)` after the task's final events, so a restart on the same device (revision back to 0) is accepted. `realtime-status`, `session-error`, `activity-logged`, `raw-fired` and `auth-expired` carry no revision, although ARCHITECTURE §5 says they all do (F12).
- **Capabilities**: `core:default`, `updater:default`, `process:allow-restart`, `dialog:allow-open` cover every plugin call the UI makes.

## 4. Flow trace

Login → device list → create with `gas-station` (+ floorplan, rollback on failure) → start session (switch / take-over dialogs) → slider → `set_item_value` → quantized native measurement → `item-values-changed` → `lw_switchGroupMode` → EXECUTING on the priority lane → prompt → confirm → `lw_daliGroupChange` + SUCCESSFUL (only after EXECUTING is acknowledged) → `operation-resolved` → fault raise / recover → save snapshot (critical lane, drained on stop) → stop → export (folder picker) → upgrade.

**No hard break.** The chain works end to end. Breaks in UI state and secondary paths are F2–F9 and F11.

## 5. Findings

Severity: **high** = wrong data written or release blocked; **medium** = visible wrong behaviour or misleading docs that will cost time; **low** = polish, hygiene.

| # | Sev | Where | Problem | Fix |
|---|---|---|---|---|
| F1 | high | `src-tauri/tauri.conf.json:63-66` | Updater `pubkey` is `TODO_REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY` and the endpoint is `OWNER/REPO`. Every update check fails (an ERROR is logged on each launch), and `release-check.mjs` blocks every tag. There is also no git remote and no commit yet. | User: create the GitHub repo, generate the signer key, fill in both values, add the secrets (`docs/RELEASING.md`). |
| F2 | high | `src/features/demo/SetupPanel.tsx:24-27` | `getDeviceConfig` is re-fetched only when `device.id` or `libraryVersion` changes. After `apply_demo` (same library version) the DALI-group and DI editors keep showing the old config, and saving one of them overwrites the config that was just applied. | Add `device.demo?.demoId` and a reload token to the dependencies, or call `config.reload()` after apply, reset and upgrade. |
| F3 | medium | `src-tauri/src/sim/core.rs:1022-1045`; `src/features/session/sessionState.ts:93-95` | `set_generator` (and scenario generator steps) emit only `item-values-changed`, which carries `value` / `manualOverride`, not `generator`. The generator badge and dialog seed stay stale until the next `session-changed`, which can be up to 899 s away. | Set `emit_changed = true` in `set_generator`, or add `generator` to `ItemValue`. |
| F4 | medium | `src-tauri/src/sim/faults.rs:144-177`; `core.rs:1374-1376` | A slave timeout or lamp-lost fault does not refresh `paused` in the UI (no `emit_changed`). Sensor-lost faults never count as paused, and a sensor is not resent on clear. | Set `emit_changed` in `set_fault`, include `sensor_lost` in `paused`, resend the sensor on clear. |
| F5 | medium | `src-tauri/src/sim/faults.rs:183-233` | In realistic mode a recovered fault leaves its alarm ACTIVE (correct). At the next start `restore_alarms` re-raises it with side effects: the slave stops sending and the lamp is marked failed. | In realistic mode skip alarms that have a later `lw_alarm "Alarm cleared: <type>"` event, or restore them without side effects. |
| F6 | medium | `src-tauri/src/device/upgrade.rs:286-326` | `apply_upgrade` never calls `check_lock` (apply does, `apply.rs:325`). It can rewrite a device another presenter is running. | Call `check_lock` with a `force` flag and reuse the take-over dialog in `UpgradeCard`. |
| F7 | medium | `src-tauri/src/sim/core.rs:1104-1113` | `setup_changed` resets every item's generator to the persisted / library value. Any setup save (interval, realistic mode, one generator) drops unsaved session generator overrides on all other items. | Only touch items whose persisted generator entry changed. |
| F8 | medium | `src/features/control/GeneratorDialog.tsx:80` | "Save" with "no generator" does not remove the persisted entry, but still shows the "saved" toast. The generator comes back at the next start. | When `generator === null`, delete the key from the map and save. |
| F9 | medium | `src/features/session/GlobalEvents.tsx:17-21` | Non-fatal `session-error` with code `http` renders `errors.http` with only `details`, so the toast shows the raw `{{status}}` / `{{message}}` placeholders. This is the most common session error. | Send `status` / `message` in `SessionError` (or the whole `AppError`), or fall back to `session.errorGeneric`. |
| F10 | medium | `src-tauri/src/device/demo_state.rs:25-29`; PRD §6.3 | Lock and snapshot live in `lw_demoLock` / `lw_demoSnapshot`, not in `lw_demo` as the PRD, ARCHITECTURE §12 and AGENTS say. The reason is only in code comments. Anyone exporting by hand or reading the device will look in the wrong place. | Add decision D7 and mark PRD §6.3 as changed. |
| F11 | medium | `src/features/control/ControlPanel.tsx:71-84`; `core.rs:1098` | After setup edits during a session, the Control tab shows the fault catalog and item ranges from the new `lw_demo`, while the running session keeps the old selection. Faults then fail with `notFound` and sliders with `invalidInput`. | Use session data when the session runs on this device, or show a "restart session to apply" banner. |
| F12 | medium | `docs/ARCHITECTURE.md` | Stale in several places. §4:114 `generate_item_series` "stub". §11:307 `createUpdaterArtifacts: false`, but it is `true`. §14 says no CI workflow and no sensor `FaultTarget`; both exist. §14 calls DI actions "free strings", but they are validated. §5 says every event carries `revision`. §2 and the AGENTS.md feature list miss `demo/` and `session/`. | Update the doc. |
| F13 | medium | `src-tauri/tauri.conf.json:32`; README "npm run tauri build" | `createUpdaterArtifacts: true` makes a local bundle build fail without `TAURI_SIGNING_PRIVATE_KEY` set. README presents it as the normal way to get an installable bundle. | Note the env var in the README, or pass `--config` to turn artifacts off for local builds. |
| F14 | low | `src-tauri/src/sim/core.rs:1120-1147` | Changing a DI's type mid-session switches the series name but not the control or counter semantics (0/1 sent as `ImpulseCounterNN`). | Require a restart, or rebuild the item on a type change. |
| F15 | low | `src-tauri/src/sim/operations.rs:962-968`; `GlobalEvents.tsx:32` | A deliberate stop shows a red "operation failed" toast for every open prompt (resolution `error`). | Add a `stopped` resolution, or suppress the toast. |
| F16 | low | `src-tauri/src/commands/devices.rs:53-55` | Apply warnings from create-with-pack are dropped (only the device is returned). | Return and toast them, as ApplyDemoCard does. |
| F17 | low | `src/lib/bindings.ts` (`libraryInfo`, `getDemo`) | Unused by the UI, so library load warnings are invisible in the app (only in the log). | Show `libraryInfo().warnings` in Settings, or drop the commands. |
| F18 | low | `src-tauri/src/state.rs:220-229` | `finish_stop` takes the handle out of the lock and then drains for up to 5 s. A start on the same device in that window can have its lock cleared by the old task, plus a late `session-changed(null)`. The UI always awaits stop, so this is backend-only. | Keep the start claim blocked until `finish_stop` completes. |
| F19 | low | `src/features/session/OperationPrompts.tsx:133-136` | Abort always sends "Aborted by user". There is no reason input (the PRD only needs "a reason text"). Line 15 has an orphan doc comment. | Optional reason field; remove the comment. |
| F20 | low | `src-tauri/src/device/crud.rs:27-38` | Grouping uses the first `assetParent` without checking it is a `c8y_DeviceGroup`. | Filter by group type. |
| F21 | low | PRD §6.1 vs `rs/payload/config.rs:86-101` | `maxControlGears` follows `max(55, lamps)`, but the PRD shows a fixed 55. | Mark it in the PRD. |
| F22 | low | `library/items/digital-inputs.json` | DI titles differ slightly from `docs/research/prototype-inventory.md` (e.g. "Bestellsäule belegt"). | Pick one source and align the other. |
| F23 | low | `library/items/dali-sensors.json` | One `daliBrightness` item, no `daliMotion` item, although the code supports both. | Add items if demos need them. |
| F24 | low | knip | Unused: `scripts/release.mjs` (false positive, used by RELEASING.md), `src/components/ui/separator.tsx`, devDependency `@testing-library/user-event`, 15 exports (`revisionOf`, `hasCode`, `LOCK_TTL_MS`, `searchHaystack`, `lampNumber`, `rowsToSlaves`, `rowsToGroups`, `ACTIVITY_LIMIT`, shadcn variants). | Remove, or un-export the ones only tests use. |
| F25 | low | `src-tauri/src/error.rs:24`; `commands/mod.rs:15`; i18n `notImplemented` | `NotImplemented` is dead now that every command works. | Remove the variant and its keys (or keep as a scaffold, documented). |
| F26 | low | `src-tauri/Cargo.toml:6` | `rust-version = "1.85"`, but the code uses let-chains (`if … && let …`), which need Rust 1.88. | Set `rust-version = "1.88"`. |
| F27 | low | README "Node 24" vs `.github/workflows/*.yml` `node-version: 22` | Mismatch. | Align them (add `engines`). |
| F28 | low | `rs/sim/operations.rs:227-251`; `rs/device/apply.rs:230` | Prompt summaries and apply / upgrade warnings are English-only strings built in Rust and shown as-is in a German UI. | Send structured data and translate in the UI. |
| F29 | low | `src/features/session/sessionState.ts` (`realtime`) | The `realtime-status` reducer does not check `deviceId` or revision. A late event from a stopping session could flip the new session's indicator. | Add `deviceId` + `revision` to `RealtimeStatus`. |

**Count: 2 high, 11 medium, 16 low.**

### TODO / FIXME classification
| Location | Class |
|---|---|
| `src-tauri/tauri.conf.json:63` updater pubkey `TODO_…` | must-fix before the first release (F1), user-only |
| `src/styles/globals.css:55` `TODO(design)` dark palette review | acceptable (design follow-up) |
| `docs/ARCHITECTURE.md:299,307,366-374` | must-fix (stale, F12) |
| `scripts/release-check.mjs:58`, `docs/RELEASING.md` | acceptable (they are the guard) |
| No `unimplemented!` / `todo!` / `FIXME` anywhere | — |

## 6. Only the user can do

1. Create the GitHub repository, add the remote, make the first commit on `main`, and replace `OWNER/REPO` in `tauri.conf.json`.
2. Generate the updater key (`npm run tauri signer generate`), put the public key in `tauri.conf.json`, and keep the private key safe.
3. Add the secrets in a `release` environment with reviewers: `TAURI_SIGNING_PRIVATE_KEY` (+ password), `APPLE_CERTIFICATE` (+ password), `APPLE_SIGNING_IDENTITY` (Team XD59B34DMF), `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_P8`. Add a `v*` tag ruleset.
4. Provide a real app icon (`npm run tauri icon <logo.png>`); the current one is a placeholder.
5. Real-tenant checks: log in on test and prod, check that `description` shows in Device Management (PRD §19), confirm the floorplan plugin shows the device, widgets, group / Modbus controls and the operation round trip, and check OSM tiles load in a release build.
6. Real-device checks: DALI `Brightness` % vs arc level, the `"C"` temperature unit, and the `lw_configDigitalInputs[].title` tolerance.
7. Windows: install the unsigned NSIS build, test the keychain (Credential Manager) and the updater.
8. Design review of the dark-mode palette.

## Resolution

Date: 2026-10-12. `npm run check` (vitest 83 tests, cargo 256 tests, clippy, fmt, library, versions), `npm run build`
and `npm run tauri build -- --debug --no-bundle` pass. A local `npm run tauri build -- --debug --bundles app` also
works without signing keys. `actionlint` is clean, `knip` reports nothing (config in `knip.json`). Bindings were
regenerated. Every behavioural fix has a test (cargo or vitest, named below).

| # | Status | What was done | Test |
|---|---|---|---|
| F1 | needs the user | Nothing invented. README has a "Before the first release" checklist that points to `docs/RELEASING.md`; ARCHITECTURE §11 / §14 say the same. `release-check.mjs` still blocks tags. | n/a |
| F2 | fixed | Setup editors load through `useDeviceConfig`, which also reloads on `configRevision`; apply and upgrade call `reloadConfig()`. | `useDeviceConfig.test.tsx` |
| F3 | fixed | `set_generator` (also used by scenario steps) sets `emit_changed`, so a snapshot with the generator follows. | `generator_change_refreshes_the_snapshot` |
| F4 | fixed | `set_fault` sets `emit_changed`; a sensor with a `_lost` / `_notExisting` fault counts as `paused`; the sensor is sent again on clear. | `faults_refresh_paused_and_sensors_come_back` |
| F5 | fixed | `open_alarms` drops an alarm whose newest `lw_alarm` "Alarm cleared: <type>" event is later than the alarm (`drop_recovered`). | `recovered_alarms_are_not_restored`, restart test with alarm A4 |
| F6 | fixed | `upgrade::apply(…, force)` calls `check_lock`; `apply_upgrade` has a `force` argument; `UpgradeCard` shows the take-over dialog (shared `deviceLockOf`). | `upgrade_respects_a_foreign_lock`, `errors.test.ts` |
| F7 | fixed | `setup_changed` touches only items whose persisted generator entry changed. | `setup_changed_keeps_unsaved_generator_overrides_of_other_items` |
| F8 | fixed | "No generator" + save removes the persisted override (`savedGenerators`); the setup is saved before the session gets the generator, so the library default does not come back in the running session. | `generatorDraft.test.ts` |
| F9 | fixed | `sessionErrorText` never picks a text that needs `status` / `message` / `user` / `since`; it falls back to the generic line. | `sessionError.test.ts` |
| F10 | fixed | DECISIONS D7 added; PRD §6.3 changed inline ("changed 2026-10-12, see DECISIONS.md D7"). | n/a |
| F11 | fixed | `SessionSnapshot.restartNeeded` (selection, custom items / slaves, lamp count, DI type change); the Control tab shows a "restart the session" banner. Reverting the edit clears it. | `setup_edits_that_need_a_restart_are_flagged`, `sessionState.test.ts` |
| F12 | fixed | ARCHITECTURE updated: layout, stub/impl table, create / upgrade / library_info rows, events and revisions, `stop_gate`, updater and overlay, checks, open TODOs, D7. | n/a |
| F13 | fixed | `createUpdaterArtifacts` is `false` in `tauri.conf.json`; `src-tauri/tauri.release.conf.json` turns it on; the release workflow passes `--config`; `release-check.mjs` verifies overlay and workflow in release mode; README / RELEASING / AGENTS explain it. | `release-check.mjs` run, `actionlint` |
| F14 | fixed | A DI change that renames the series (value <-> impulse counter) no longer switches the running item half-way; it sets `restartNeeded`. A unit-only change still applies. | `setup_edits_that_need_a_restart_are_flagged` |
| F15 | fixed | New resolution `stopped` for prompts failed by a deliberate stop; no toast (`operationToast`). | `stop_fails_open_prompts_and_releases_lock`, `operationToast.test.ts` |
| F16 | fixed | `create_demo_device` returns `CreateDeviceResult { device, warnings }` (logic in `apply::create_device`); the dialog toasts the warnings. | `create_with_pack_returns_the_result_and_rolls_back_a_failed_apply` |
| F17 | fixed | Settings shows `libraryInfo().warnings` (`LibraryWarnings`). `getDemo` stays as an API command. | `LibraryWarnings.test.tsx` |
| F18 | fixed | `AppState.stop_gate` is held during a stop; `start_session` waits for it. | `a_start_waits_for_a_running_stop` |
| F19 | fixed | Prompt cards have an optional reason field; blank means the default reason. Orphan comment removed. | `operationDecision.test.ts`, `abort_reason_is_sent_as_failure_reason`, card test |
| F20 | fixed | `parent_group` prefers the parent that is a device group (`c8y_IsDeviceGroup` or a group `type`). If the references carry no type at all, the first one is used as before (the real references may hold only id and name; not verified against a tenant). | `parent_group_skips_parents_that_are_not_groups` |
| F21 | fixed | PRD §6.1 and DECISIONS D5 record `max(55, daliLampCount)`. | n/a |
| F22 | fixed (doc) | The library is the source of truth for DI titles (AGENTS "Library content rules"); `docs/research/` was not touched. | n/a |
| F23 | no change | By design: no demo needs a motion sensor. AGENTS says the code supports it and a motion item can be added. | n/a |
| F24 | fixed | Removed `separator.tsx` and `@testing-library/user-event`; un-exported the helpers only used inside their file; `lampNumber` deleted; `knip.json` marks `scripts/*.mjs` as entries and ignores generated bindings and shadcn exports. | `knip` clean |
| F25 | fixed | `AppError::NotImplemented`, its constructor, i18n keys and the doc line are gone. | typecheck (i18n code map) |
| F26 | fixed | `rust-version = "1.88"`. This turned on clippy's let-chain `collapsible_if`; one hit in `library/validate.rs` fixed. | clippy |
| F27 | fixed | Workflows use Node 24 (as README says), `engines.node >= 24`. | `actionlint` |
| F28 | partly fixed | Operation prompts are worded in the UI from structured `OperationPrompt.detail` (German and English, item title in the UI language). The English `warnings` of apply / upgrade / export are not translated (see below). | `operationSummary.test.ts`, `detail_carries_the_title_in_both_languages` |
| F29 | fixed | `RealtimeStatus` carries `revision` and `deviceId`; the reducer ignores other devices, stale revisions and events without a session. | `sessionState.test.ts` |

### Not fixed

- **F1**: needs the repository, signer key and secrets (user only, see section 6).
- **F28 (warnings)**: about ten `warnings` strings in `device/apply.rs`, `setup.rs`, `export.rs` stay English prose. They
  are rare technical diagnostics that name library ids. Translating them needs a code-plus-parameters type on every
  result that carries warnings. Recorded in ARCHITECTURE §14 and AGENTS ("Texts").
- **F23**: left as is on purpose (no demo needs a motion item).

### Notes for the owner

- `apply_upgrade` and `create_demo_device` changed their IPC signature (`force` argument; `CreateDeviceResult`).
  `SessionSnapshot` got `restartNeeded`, `OperationPrompt` got `detail`, `RealtimeStatus` got `revision` / `deviceId`,
  `OperationResolution` got `stopped`.
- The Control tab's fault catalog and ranges still come from the saved device setup. While `restartNeeded` is set they
  can differ from the running session; the banner says so.
- The in-session "no generator" cannot be stored as an override for an item whose library entry has a generator (there is
  no "none" generator in the format); the next session starts with the library generator.
