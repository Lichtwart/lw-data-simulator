# Architecture — Lichtwart Demo Simulator

Companion to [`PRD.md`](./PRD.md). This file is the **contract** between the backend
(Rust, `src-tauri/`) and the frontend (React, `src/`) and the map of who owns what.
Feature work fills in function bodies behind this contract; changing the contract
(command names, argument/return types, event names) means updating this file too.

## 1. Principles

- **All Cumulocity I/O happens in Rust** (reqwest + realtime). React never talks to
  Cumulocity; it calls Tauri commands and listens to Tauri events (PRD §4).
- **One IPC surface**: every command lives in `src-tauri/src/commands/` (one file per area,
  each registering its own commands), every event in `src-tauri/src/events.rs`, every type
  crossing IPC in `src-tauri/src/model/`.
- **Generated TS types**: `src/lib/bindings.ts` is generated from Rust by tauri-specta. The
  frontend imports through `src/lib/api.ts` only.
- **Commands are thin adapters**: they resolve `AppState` and call into the owning module.
  Business logic lives in the module (`c8y`, `library`, `payload`, `sim`, `device`, `raw`).
- **Pure where possible**: naming/payload rules (`payload`) and generators (`sim::generators`)
  are pure functions with unit tests (PRD §18).

## 2. Repository layout

```
docs/                 PRD.md, ARCHITECTURE.md (this), research/ (other agents)
library/              bundled demo library (Tauri resource) — PRD §11
  items/ demos/ floorplans/<demoId>/
  schema/             JSON schemas (draft 2020-12; `npm run validate:library` + `cargo test`)
src/                  React + TypeScript frontend
  app/                router, AppShell (sidebar layout), DeviceWorkspace (tabs), PageHeader
  features/<name>/    one folder per feature: auth, devices, control, demo (Setup tab: apply,
                      upgrade, editors, export), session (session state, prompts, global
                      toasts), raw-fire, activity, settings, updates
  components/ui/      shadcn/ui components (themed via Lichtwart tokens)
  lib/                api.ts (typed backend API), bindings.ts (GENERATED), library.ts,
                      errors.ts, format.ts, useAppEvent.ts, useAsync.ts, theme.ts, utils.ts
  i18n/               index.ts, de.json, en.json (+ parity test)
  styles/             tokens.css (copied from the design system), fonts.css, globals.css
  assets/             fonts/ (FT Polar), logos/
src-tauri/            Rust backend (crate `lichtwart-demo-simulator`, lib `lw_sim_lib`)
  src/                see §3
  capabilities/       Tauri permissions for the main window
  tauri.conf.json     app/bundle/updater config
  tauri.release.conf.json  overlay for the release build only (`createUpdaterArtifacts`)
```

## 3. Rust modules and ownership

| Module | Owns | Notes |
|---|---|---|
| `lib.rs` | `run()`, plugin setup (updater, process, dialog, log), `specta_builder()` (= `commands::collect()` + `events::collect()`), `export_bindings()` | In `setup` loads the library (`load`) from the resource dir and `manage`s `AppState`. No tenants are bundled (PRD §5). |
| `commands/` | every `#[tauri::command]`, one file per area: `app`, `auth`, `library`, `devices`, `session`, `raw`, `activity` | Thin adapters. All return `AppResult<T>`. Each file ends with a `<area>_commands!` macro listing its own commands; `commands::collect()` chains them, so two agents working on different areas never touch the same file. |
| `events.rs` | every emitted event (struct + name) and `collect()` | Name = kebab-case struct name unless `#[tauri_specta(event_name = …)]`. |
| `model/` | every serde type crossing IPC + library file format | `auth`, `library`, `device`, `session`, `ops`, `raw`, `activity`, `app`. camelCase JSON. |
| `error.rs` | `AppError` (single serializable error), `AppResult<T>` | Serialized as `{ code, details? }`; frontend maps `code` → `errors.<code>` i18n key. |
| `state.rs` | `AppState`, `Authenticated`, shared session helpers (`stop_session`, `stop_session_for`, `session_sender`, `session_snapshot`, `expire_auth`) | See §7. |
| `c8y/client.rs` | `C8yClient`: Basic auth (`UseXBasic: true`, always `tenantId/user` — `basic_auth_user`; `parse_tenant` turns the entered tenant id + URL into a `Tenant`), transport, error mapping, activity logging (`record`), 401 handling (`check_unauthorized`), generic `get_json` / `post_json` / `put_json` / `delete` / `request_json`, `current_user`, `now_iso` | **Implemented + tested.** Takes an `Arc<dyn EventSink>`. 30 s request (`with_request_timeout` overrides it; per-request `RequestSpec::timeout` wins: realtime, binaries 120 s) / 10 s connect timeout; cookie store on (server affinity). Errors: transport → `network`, 401 → `auth-expired` (only if it really ended the login) + `notAuthenticated` (`current_user`: `authFailed` with the server's reason), other non-2xx → `http {status, message}` (message from the Cumulocity error body). POSTs to measurements / alarms / events get `time = now` if missing (also per element of a collection). |
| `c8y/api.rs` | typed helpers on `C8yClient` (see §15) | **Implemented + tested.** Bodies / results are `serde_json::Value`. |
| `c8y/media.rs` | `application/vnd.com.nsn.cumulocity.*` media types; `for_request(method, path, body)` picks `Accept` / `Content-Type` | **Implemented + tested.** Used by every request and by `to_curl`. |
| `c8y/fake.rs` | `#[cfg(test)]` fake Cumulocity HTTP server (axum) | See §15. |
| `c8y/sink.rs` | `EventSink` trait, `TauriSink` (emits `activity-logged`; on 401 clears `AppState.auth`, stops the session, emits `auth-expired`), `NoopSink` | **Implemented.** The client never knows Tauri. |
| `c8y/credentials.rs` | "Remember me": password in the OS keychain (`keyring-core` + native store), tenant id, URL and user name in `<app config dir>/saved-logins.json` | **Implemented + tested.** Keychain service `io.lichtwart.demo-simulator`, account = tenant key (`<tenantId>@<host>`), secret = the password. Entries of the 0.1 format (`{ key: username }`) are skipped. Listing logins reads only the file (no keychain prompt); only `login_saved` reads the secret. |
| `c8y/realtime.rs` | Bayeux/CometD long-polling on `/notification/realtime`, subscriptions to `/operations/<deviceId>` and `/alarms/<deviceId>` (one client) | **Implemented + tested.** `subscribe_device` spawns the task and returns at once; `RealtimeSubscription` (cancel-on-drop, `recv()` → `RealtimeMsg::{Connected, Disconnected(err), Operation(json), PendingOperation(json), Alarm(json), OpenAlarms {alarms, fetched_at}}`). handshake → subscribe both channels → GET PENDING ops + open alarms → `Connected` + ops + `OpenAlarms` → connect loop (D9: an alarm DELETE arrives as `CLEARED`). `402::Unknown client` / `advice.reconnect=handshake` → silent re-handshake (first one at once, repeats with backoff; then `Connected` + PENDING again); errors → `Disconnected` + backoff 1 s … 30 s; 401 / 403 / `reconnect: none` → `Disconnected` and the task ends. See §15 "Realtime". Realtime payload `{realtimeAction, data}` is unwrapped; `DELETE` skipped. **Ops may arrive more than once** (PENDING list + CREATE, UPDATEs) — the worker drops repeats and status regressions per id (`OpDedup`); the session must still ignore the UPDATE echo of its own status changes. Quiet long-polls are not logged. |
| `c8y/activity.rs` | `ActivityLog` ring buffer (500) + `to_curl(entry)` | **Implemented.** Entries carry `baseUrl` + `user`, so `to_curl` needs no client; password masked as `-u 'tenant/user:****'`, headers are never logged (`to_curl` derives `Accept` / `Content-Type` again via `media::for_request`; a binary upload becomes `-F` parts). Bodies cut at 2 kB; binary responses logged as `[N bytes]`. |
| `library/` | `Library` (immutable, loaded once; items / slaves / demos + referenced floorplan layouts + `origins` per file), `loader` (`load` skips bad files with a warning, check `type` + `schemaVersion`), `validate` (`validate`, `validate_custom` for D6; findings with file + JSON path), `select` (`select_items` / `select_by` = tags ∪ includes − excludes, `search`, `filter_chips` / `matches_chips`), `series` ("generate series"), `schema_tests` (files ⇄ schema ⇄ serde) | **Implemented + tested** (fixture library `src-tauri/tests/fixtures/library/`). |
| `payload/naming.rs` | measurement names for every item kind (`item_naming` / `item_naming_with_di`: fragment = German title via `fragment_name`, series = item id, D10; DALI lamp series `<id>` + `<id> Lamp Status` …, `dali_series_name` / `dali_library_series`; lamps without an item `hidden_lamp_naming`), types + units, slave id/address assignment (`assign_slaves`, keeps existing), register address assignment (`assign_registers`), alarm types / texts, alarm-name rules (D2) | **Implemented + tested**; firmware facts (units, alarms) checked against `research/firmware-core-v2.md`. |
| `payload/measurement.rs` | `item_values` / `dali_lamp_values` → `MeasurementValue`s (quantized per measurement type), `build_measurement_bodies` (collections in firmware order, ≤ 6 DALI lamps each, one object per lamp even when two share a title, `time` on all, empty units omitted) | **Implemented + tested.** |
| `payload/alarm.rs` | `AlarmSpec`, native lamp / bus / brightness / motion alarms (firmware texts + severities), slave timeout / custom alarms, `device_custom_alarm`, `fault_catalog` + `resolve_fault` (`FaultSetup`), `build_alarm`, `build_alarm_clear` | **Implemented + tested.** |
| `payload/event.rs` | `build_event`, `build_group_change` (capitalised modes), `build_alarm_cleared`, `build_modbus_timeout_event` | **Implemented + tested.** |
| `payload/floorplan.rs` | `floorplan_body(layout, FloorplanContext, imageBinaryId)` → `lw_floorplan` MO (item refs → `{fragment: title, series: id}`, library DALI series names → wire names, alarm types, Modbus targets), `item_alarm_types` | **Implemented + tested.** |
| `payload/config.rs` | `demo_device_body(NewDemoDevice, lw_demo?)` (§6.1, no `lw_config*`, D10), `missing_required_fragments`, `digital_input_entry` (demo DI → device entry with the item title), `truncate_bytes` | **Implemented + tested.** |
| `sim/session.rs` | `SessionHandle` + `SessionSender` (cloneable), `prepare` (device, lock, resolve, counter seeds, group modes, open alarms minus the ones the firmware already recovered, written registers), the task loop, `SessionCommand` (incl. internal `SetupChanged`), `announce_exit` | **Implemented + tested.** Stop: open prompts → FAILED "Simulator stopped", lock released, queue drained (≤ 5 s). |
| `sim/outbound.rs` | the outbound worker: two lanes (normal; priority = operation statuses, in order, never behind measurements), results channel, `test_pair()` double | **Implemented + tested.** |
| `sim/generators.rs` | `next_value(cfg, state, Tick, base, &mut dyn RandomSource)`, `SeededRng`, `local_hour` | **Implemented + tested** (deterministic). |
| `sim/core.rs` | `SessionCore`: all live state + mutations without I/O (values, generators, effects, groups, DI actions, alarms, snapshot, `finish()` → tagged outbound + events + revisioned snapshot) | **Implemented + tested.** |
| `sim/emit.rs` | `SessionEmitter` (`TauriEmitter`, `RecordingEmitter` for tests), `error_code` | **Implemented.** |
| `sim/{scheduler,operations,faults,scenario}.rs` | send timing (periodic `sendIntervalSec`, generator tick 5 s, lock refresh 3 min), op triage (de-dup, final ignored, stale → offline) + prompt flow (60 s), faults (D9: fire once, effect until the alarm is CLEARED in Cumulocity — realtime `Alarm`, `OpenAlarms` re-sync, "Clear all", scenario `active: false`; ACKNOWLEDGED only changes `ActiveFault.status`; bus cascade D8, slave pause, lamp status, realistic mode for derived alarms), scenarios (start scenario by hand = Clear all + steps; device edits taken over by `SetupChanged`; "Reset to defaults", D11) | **Implemented + tested** (paused-time tests in `sim/tests.rs`). |
| `device/` | `crud`, `demo_state` (`lw_demo` + its own fragments `lw_demoLock` / `lw_demoSnapshot` (D7), lock TTL 10 min, refresh 3 min, groups / inputs get/set in `lw_demo`), `setup` (resolve library + custom items, slave/register assignment, `device_config` (groups / inputs from `lw_demo`, else the library demo), `stale_config_fragments` (old `lw_config*` → `null`, D10), digests, versions), `apply`, `floorplan`, `export`, `upgrade` | **Implemented + tested** (`device/tests.rs` against the fake, incl. export ↔ apply round trip). |
| `raw.rs` | raw fire `build()` (pure) + `fire()` + `RawHistory` (50) | **Implemented + tested.** Always sets `source.id` and `time` (raw JSON: only if missing). |

## 4. IPC contract — commands

TS name = camelCase of the Rust name (`login_saved` → `api.loginSaved()`); argument names
are camelCase in TS. Every command returns `Result<T, AppError>`; `api.*` unwraps and throws
`ApiError`. "Status": **impl** = works now, **plumb** = wired end-to-end but depends on a
stubbed module function, **stub** = module function returns an error until implemented. Today every
command is **impl**.

Files: `app`, `auth`, `library`, `devices`, `session`, `raw`, `activity` in
`src-tauri/src/commands/`; a new command is registered in the macro at the end of its own file.

### App
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `app_info` | — | `AppInfo` | commands/app | impl |

### Auth (PRD §5)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `login` | `request: LoginRequest` (`tenantId`, `baseUrl`, `username`, `password`, `remember`) | `AuthState` | c8y | impl; tenant from `parse_tenant` (bad id / URL → `invalidInput`); validates via `GET /user/currentUser` (401 → `authFailed`, empty fields → `invalidInput`); stops a running session and emits `session-changed(null)` |
| `login_saved` | `tenantKey` | `AuthState` | c8y | impl (reads the keychain secret here, and only here) |
| `logout` | — | `null` | state | impl |
| `current_auth` | — | `AuthState \| null` | state | impl |
| `saved_logins` | — | `SavedLogin[]` (`{ tenant, username }`) | c8y::credentials | impl (reads the config file only; any problem = empty list; the login screen's tenant choice) |
| `forget_login` | `tenantKey` | `null` | c8y::credentials | impl (keychain secret + stored user name) |

### Library (PRD §11)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `library_info` | — | `LibraryInfo` | library | impl (Settings shows its load `warnings`) |
| `list_demos` | — | `DemoPack[]` | library | impl |
| `get_demo` | `demoId` | `DemoPack` | library | impl |
| `list_library_items` | — | `LibraryItem[]` | library | impl |
| `list_slaves` | — | `SlaveDefinition[]` | library | impl |
| `generate_item_series` | `request: GenerateSeriesRequest` | `LibraryItem[]` | library::series | impl (pure; "generate series" helper, PRD §11.3; used by the item editor) |

### Demo devices (PRD §6, §12–§14)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `list_demo_devices` | `query: string \| null` | `DemoDeviceSummary[]` | device::crud | impl |
| `list_device_groups` | — | `GroupRef[]` | device::crud | impl |
| `get_demo_device` | `deviceId` | `DemoDevice` | device::crud | impl |
| `create_demo_device` | `request: CreateDeviceRequest` | `CreateDeviceResult` (`device`, `warnings` of an applied pack) | device::apply (`create_device`, +crud) | impl (a failed pack deletes the new device again) |
| `update_demo_device` | `deviceId, request: UpdateDeviceRequest` | `DemoDevice` | device::crud | impl |
| `reset_demo_device` | `deviceId, request: ResetDeviceRequest` | `null` | device::crud | impl (stops session first) |
| `delete_demo_device` | `deviceId` | `null` | device::crud | impl (stops session first) |
| `preview_apply_demo` | `deviceId, request: ApplyDemoRequest` | `ApplyDemoPreview` | device::apply | impl |
| `apply_demo` | `deviceId, request: ApplyDemoRequest` | `ApplyDemoResult` | device::apply | impl (stops session first; removes old `lw_config*` fragments, D10) |
| `update_demo_setup` | `deviceId, patch: DemoSetupPatch` | `LwDemoState` | device::demo_state | impl (`generators` replaces the whole map; a running session on the device picks up interval / realistic mode / generators / scenarios; patch also carries `customSlaves` / `customItems`, D6, and `customScenarios` / `hiddenScenarios` (optional), D11: custom scenarios are validated against the device's items and faults) |
| `get_device_config` | `deviceId` | `DeviceConfig` (`groups`, `digitalInputs`) | device::demo_state | impl (`lw_demo.daliGroups` / `digitalInputs`, else the library demo's; D6, D10) |
| `set_dali_groups` | `deviceId, groups: DaliGroup[]` | `DaliGroup[]` | device::demo_state | impl (writes `lw_demo.daliGroups`, D6, D10) |
| `set_digital_inputs` | `deviceId, inputs: DigitalInputEntry[]` | `DigitalInputEntry[]` | device::demo_state | impl (writes `lw_demo.digitalInputs` incl. titles = measurement fragments, D6, D10) |
| `fault_catalog` | `deviceId` | `FaultDefinition[]` | sim::faults | impl (computed from the device's saved setup; while a session runs with an older selection `SessionSnapshot.restartNeeded` says so) |
| `export_demo_pack` | `request: ExportRequest` | `ExportResult` | device::export | impl |
| `check_upgrade` | `deviceId` | `UpgradePreview \| null` | device::upgrade | impl |
| `apply_upgrade` | `deviceId, force: bool` | `DemoDevice` | device::upgrade | impl (stops session first; a fresh lock of another presenter → `deviceLocked` unless `force`, like `apply_demo`) |

### Session (PRD §7–§9)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `start_session` | `request: StartSessionRequest` | `SessionSnapshot` | sim::session | impl (`sessionActive` / `deviceLocked`; counters seeded from the newest stored measurement) |
| `stop_session` | — | `null` | sim::session | impl |
| `get_session` | — | `SessionSnapshot \| null` | sim::session | impl |
| `set_item_value` | `request: SetItemValueRequest` (`itemId`, `value`) | `null` | sim | impl (a struct, because a bare `f64` argument cannot be typed as `number`) |
| `resume_generator` | `itemId` | `null` | sim | impl |
| `set_item_generator` | `itemId, generator: GeneratorConfig \| null` | `null` | sim | impl |
| `apply_scenario` | `scenarioId` | `null` | sim::scenario | impl (the device's scenarios: library demo's as edited in `lw_demo`, D11) |
| `reset_to_defaults` | — | `null` | sim::scenario | impl ("Clear all" + every item back to its default value and effective generator, manual overrides dropped; counters keep their value, D11) |
| `set_group_mode` | `groupId, mode: GroupMode` | `null` | sim | impl |
| `fire_fault` | `target: FaultTarget` | `null` | sim::faults | impl (posts the alarm, again if open; the effect lasts until the alarm is cleared in Cumulocity, D9) |
| `clear_all_alarms` | `deviceId` | `null` | sim::faults / c8y | impl (every ACTIVE / ACKNOWLEDGED alarm of the device → CLEARED, bulk update with fallback; through the session when one runs on the device (all effects end), else a plain REST call; D9) |
| `set_send_interval` | `seconds` | `null` | sim::scheduler | impl |
| `set_auto_accept` | `enabled` | `null` | sim::operations | impl |
| `send_all_now` | — | `null` | sim::scheduler | impl |
| `save_snapshot` | — | `null` | sim + device::demo_state | impl |
| `answer_operation` | `operationId, decision: OperationDecision` | `null` | sim::operations | impl (`abort.reason` optional, from the prompt's reason field; default `Aborted by user`) |

"plumb" session commands send a `SessionCommand` to the task and return its reply; without a
session (or when the task has already ended) they fail with `noSession`. They never hold the
`AppState.session` lock while waiting for the reply (§7).

### Raw fire (PRD §10)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `fire_raw` | `deviceId, request: FireRequest` | `FiredRecord` | raw | impl (also emits `raw-fired`; a failed POST is an error and not stored) |
| `raw_history` | — | `FiredRecord[]` | raw | impl |
| `resend_raw` | `recordId` | `FiredRecord` | raw | impl (fresh `time`, new history entry) |

### Activity log (PRD §16)
| Command | Args | Returns | Owner | Status |
|---|---|---|---|---|
| `activity_log` | — | `ActivityEntry[]` | c8y::activity | impl |
| `clear_activity_log` | — | `null` | c8y::activity | impl |
| `activity_as_curl` | `entryId` | `string` | c8y::activity | impl (needs no login) |

### Not commands
- **App updates** use the updater JS plugin directly (`@tauri-apps/plugin-updater` `check()`,
  `@tauri-apps/plugin-process` `relaunch()`), see `src/features/updates/useUpdateCheck.ts`.
- **Language / theme** are frontend-only (localStorage).

## 5. IPC contract — events

Subscribe with `events.<name>.listen(cb)` or `useAppEvent(events.<name>, cb)`.

**Revisions.** The events of the session (`session-changed`, `item-values-changed`,
`fault-changed`, `operation-prompt`, `operation-resolved`, `realtime-status`) carry `revision`
(and, except `session-changed`, `deviceId`); the UI drops an event at or below the snapshot's
revision. `session-error`, `activity-logged`, `raw-fired` and `auth-expired` are not tied to a
session snapshot and carry none.

| Event (Tauri name) | TS accessor | Payload | Emitted by | When |
|---|---|---|---|---|
| `session-changed` | `sessionChanged` | `SessionSnapshot \| null` | commands, sim::session | session start/stop, config change (auto-accept, interval, group modes, prompts, realtime), after every periodic send (`lastSentAt`), a generator change and a fault change (they change `paused` / `generator`, which `item-values-changed` does not carry). `SessionSnapshot.revision` is monotonic per session. `SessionSnapshot.restartNeeded` is set when the device's setup changed in a way the running session cannot take over (item selection, custom items / slaves, a digital input switched between value and impulse counter) |
| `item-values-changed` | `itemValuesChanged` | `{ revision, deviceId, values: ItemValue[] }` | sim | generator tick, manual change, scenario, op effect (batched per tick) |
| `fault-changed` | `faultChanged` | `{ revision, deviceId, target, fault: ActiveFault \| null }` | sim::faults | fault fired, its alarm id / status changed (acknowledged in Cumulocity), or ended (`null`: alarm cleared in Cumulocity, D9) |
| `operation-prompt` | `operationPrompt` | `OperationPrompt` (incl. `revision`, `deviceId`, structured `detail` that the UI words per language; `summary` is the English fallback) | sim::operations | visible op needs Confirm/Abort (60 s) |
| `operation-resolved` | `operationResolved` | `OperationResolved` (incl. `revision`, `deviceId`) | sim::operations | op SUCCESSFUL/FAILED (auto, confirmed, aborted, timed out, offline, withdrawn, error; `stopped` = open prompts failed by a deliberate stop, no toast) |
| `realtime-status` | `realtimeStatus` | `{ revision, deviceId, connected, error }` | sim (from `RealtimeMsg::Connected` / `Disconnected`) | realtime connection up/down |
| `session-error` | `sessionError` | `{ code, message, fatal }` | sim | problem inside the session task; `fatal` also when the task dies (`sim::session::announce_exit`) |
| `activity-logged` | `activityLogged` | `ActivityEntry` | c8y::sink (`TauriSink`) | after each request |
| `raw-fired` | `rawFired` | `FiredRecord` | commands/raw | after `fire_raw` / `resend_raw` succeeded (also returned) |
| `auth-expired` | `authExpired` | `{ tenantKey }` | c8y::sink (`TauriSink`) | 401 on an authenticated call. The backend has already cleared `AppState.auth` and stopped the session (and emitted `session-changed(null)`); the UI only shows the login screen. |

## 6. Errors

`AppError` (`src-tauri/src/error.rs`) is adjacently tagged: `{ "code": "<variant>", "details"?: … }`.
Codes: `notAuthenticated`, `authFailed`, `http`, `network`, `notFound`,
`invalidInput`, `library`, `keychain`, `noSession`, `sessionActive`, `deviceLocked`, `io`,
`internal`. `details` is diagnostic only; UI text comes from `errors.<code>` in
`src/i18n/*.json`. `src/i18n/i18n.test.ts` is typed against the generated `AppError` union,
so adding a variant without i18n keys fails `npm run typecheck`.

## 7. State management and concurrency

`AppState` (managed by Tauri, injected as `State<'_, AppState>`):

| Field | Type | Rule |
|---|---|---|
| `library` | `Result<Arc<Library>, AppError>` | Immutable after startup; no lock. Bad item / demo files are skipped and listed in `LibraryInfo.warnings`. |
| `auth` | `tokio::RwLock<Option<Authenticated>>` | Written on login/logout. Clone the client (`state.client().await?`) — never hold the guard across network I/O. |
| `stop_gate` | `tokio::Mutex<()>` | Held for the whole of a stop (the handle is out of `session` while the task drains for up to 5 s). `start_session` waits for it (`wait_for_stops`), so the old task cannot clear the new lock. |
| `session` | `tokio::Mutex<Option<SessionHandle>>` | At most one session. Live values are owned by the session task, not by `AppState`. **Never await the task while holding the guard**: clone the `SessionSender` (`state.session_sender(&app)`), or take the handle (`stop_session`), drop the guard, then await. A handle whose task has finished (`is_finished()`) counts as "no session" and is pruned on access. Stopping always goes through `AppState::stop_session` / `stop_session_for`, which emit `session-changed(null)` — used by `login`, `logout`, `stop_session`, device reset / delete / apply / upgrade and the 401 handler. |
| `activity` | `Arc<ActivityLog>` | Shared with every `C8yClient`; internally a `std::Mutex` (never held across `.await`). |
| `raw_history` | `tokio::Mutex<RawHistory>` | Small, in memory. |

Frontend state: React state + context only (`AuthProvider`); backend is the source of truth.
Feature screens fetch via `api.*` and stay current via events. No global store library yet.

## 8. Session on a tokio task

```
start_session ─▶ SessionHandle::start(app, client, library, req)
                   1. GET device + lw_demo, check lock (fresh foreign lock → deviceLocked
                      unless force), write own lock
                   2. subscribe: c8y::realtime::subscribe_device(deviceId) → RealtimeSubscription
                      (operations + alarms; on every (re)connect it GETs the PENDING ops and the
                      open alarms and feeds them; the session fails PENDING ops older than 60 s
                      with "Simulator was offline", ignores ops that are already FAILED, and ends
                      the effect of a known alarm missing from the open list, D9)
                   3. resolve items (library + lw_demo.customItems, selection, overrides) → naming
                      (payload::naming::item_naming, with lw_demo.slaves assignments)
                   4. outbound::spawn(client, deviceId) → OutboundWorker { queue, results }
                   5. tauri::async_runtime::spawn(task); apply start scenario
                 ◀─ SessionHandle { device_id, sender: SessionSender (Clone),
                                    snapshot: watch<SessionSnapshot>, task: JoinHandle }

state loop (tokio::select!) — owns ALL live state, never awaits HTTP:
  • cmd = rx.recv()           → mutate live state, reply Ok/Err, enqueue changed values
  • interval.tick()           → generators step, periodic full enqueue (sendIntervalSec)
  • msg = realtime.recv()     → Connected/Disconnected → realtime-status event;
                                Operation → sim::operations (auto-answer | prompt + 60 s timer);
                                Alarm / OpenAlarms → sim::faults (cleared → effect ends,
                                acknowledged → status)
  • result = outbound.results → bookkeeping: lastSentAt, alarm ids, session-error on failures
  • prompt timeout            → FAILED "Timed out" (enqueued)
  • lock tick (every ~3 min, `LOCK_REFRESH_INTERVAL`) → enqueue the lock refresh (`lw_demoLock.since = now`)
  after each change: publish snapshot (watch), emit item-values-changed / fault-changed /
  session-changed as appropriate
  Stop → release lock, drop realtime (cancel-on-drop), drop outbound, exit

outbound worker (separate task): drains a bounded queue (256) of OutboundPayload
  (measurements, alarm, alarm clear, event, operation status, inventory patch) in order and
  reports one OutboundResult per item. The state loop uses `OutboundQueue::enqueue`
  (non-blocking). One exception: when a visible operation arrives, its EXECUTING `PUT` is
  spawned on its own task so it reaches Cumulocity within the plugin's 10 s PENDING timeout
  even if the queue has a backlog.
```

The task runs independently of the window (keeps going when hidden) and ends when the app
quits. Commands never touch live values directly — they go through a `SessionSender`.

**Dead task.** If the task ends without a `Stop` command (fatal error, panic), it calls
`announce_exit` (`session-error { fatal: true }` + `session-changed(null)`). Independently,
`AppState` checks `SessionHandle::is_finished()` on every access and treats a finished handle as
"no session" (`start_session` is allowed again, `get_session` returns `null`, commands fail with
`noSession`). `SessionHandle::stop()` ignores a closed channel.

## 9. TypeScript type generation

- Tool: **tauri-specta `=2.0.0-rc.25`** + `specta =2.0.0-rc.25` + `specta-typescript 0.0.12`
  (pinned with `=`, as tauri-specta requires during its RC phase).
- `npm run gen:types` runs the Rust test `tests::export_bindings`, which writes
  `src/lib/bindings.ts` (commands with typed args/results, events, all types). The test also
  runs in `cargo test`, so CI can enforce `git diff --exit-code src/lib/bindings.ts`.
- **No bare `f64` command arguments or results** (`u64`/`i64`/`usize` neither): specta cannot
  annotate them there. Wrap them in a request struct whose field carries the annotation
  (e.g. `SetItemValueRequest`).
- `f64` fields are annotated `#[specta(type = specta_typescript::Number)]` so they become
  `number` (specta's default is `number | null` because of NaN). Do the same for new `f64`
  fields (`Option<…>`, `Vec<…>`, `BTreeMap<String, …>` analogously).
- Avoid `u64`/`i64`/`usize` in IPC types (they become `bigint`, export fails).
- Types with serde defaults / `skip_serializing_if` get phased aliases: `Foo_Serialize`
  (from Rust) and `Foo_Deserialize` (to Rust). Command signatures already use the right one.

## 10. Frontend structure

- `src/app/router.tsx` — hash router: `/login`, `/devices`, `/devices/:deviceId/:tab`
  (`control | setup | raw | activity`), `/settings`; all but `/login` inside `AppShell`
  (redirects to `/login` when logged out).
- `src/features/<feature>/` — screens/panels per feature; feature agents work inside their
  folder and add shared UI to `src/components/ui`.
- `src/lib/api.ts` — `api` (unwrapped commands), `events`, `ApiError`, `errorKey` (typed
  `errors.<code>` key) + `errorValues` (error `details` as interpolation values:
  `t(errorKey(e), errorValues(e))`), `isTauri`, all types re-exported.
- i18n: every string through `t()`, keys in both `de.json` and `en.json`. `src/i18next.d.ts`
  types the keys from `de.json`, so a typo in `t("…")` fails `tsc`.
- Styling: Tailwind semantic colors mapped to `--lw-*` tokens in `src/styles/globals.css`
  (`bg-background`, `bg-card`, `bg-primary`, `bg-signal` (one per view), `text-strong`,
  `text-muted-foreground`, `bg-success-bg`, …). `font-display` (FT Polar Light) only for
  headings ≥ 28 px. Dark mode via `.dark` class (derived from tokens, incl. dark status
  backgrounds and a lighter `destructive`; design review TODO). Icons: `.lucide` strokes are
  1.5 px. The login logo switches to `lichtwart-white.svg` in dark mode.

## 11. Plugins and configuration

| Concern | Choice |
|---|---|
| Credentials | `keyring-core 1` + `apple-native-keyring-store` (macOS) / `windows-native-keyring-store` (Windows), Rust-side (`c8y::credentials`); store installed in `setup`. |
| Updater | `tauri-plugin-updater 2` (+ JS `@tauri-apps/plugin-updater`), endpoint GitHub Releases `latest.json`. **Pubkey and repo URL are placeholders until the user creates the repository and the signer key** (`docs/RELEASING.md`; `release-check.mjs` blocks a tag until then). `createUpdaterArtifacts` is `false` in `tauri.conf.json`, so a local `npm run tauri build` needs no keys; the release workflow passes `--config src-tauri/tauri.release.conf.json`, which turns it on (needs `TAURI_SIGNING_PRIVATE_KEY`). |
| Restart after update | `tauri-plugin-process 2` (`process:allow-restart`). |
| Folder picker (export) | `tauri-plugin-dialog 2` + JS `@tauri-apps/plugin-dialog` `open({ directory: true })`, capability `dialog:allow-open`. |
| Logging | `tauri-plugin-log 2` (Info). |
| Library | `bundle.resources: { "../library/": "library/" }`, resolved via `BaseDirectory::Resource`. |
| HTTP | `reqwest 0.13` (rustls, multipart), 30 s request / 10 s connect timeout; `chrono` for ISO timestamps. Dev: `axum` + `base64` (fake Cumulocity). |

## 12. Library and device data model (contract summary)

Details live in `model/library.rs`, `model/device.rs`, `model/session.rs`; decisions D1–D10 in
`DECISIONS.md`. Lock and snapshot are their own fragments `lw_demoLock` / `lw_demoSnapshot`
(D7), not fields of `lw_demo`.

- **Items** (`LibraryItem`, tagged by `kind`; every kind carries exactly its own fields):
  `dali`, `daliBrightness`, `daliMotion`, `digitalInput`, `impulseCounter` (native `index`),
  `system` (`system: rssi | temperature`), `modbus` (`slave`, `registerType?` — default
  `holding`, `coil` for a switch control — `writable`), `customAlarm` (`severity`, `text`).
- **Slaves** (`SlaveDefinition`): `key`, `name`, `requestInterval`, `alarms[]` (any number).
- **Naming** (`payload::naming`, D10): every measurement is `<fragment>.<series>` with fragment =
  the item's German title (`.` / `$` → `_`, trimmed) and series = the item id (a DALI lamp also
  sends `<id> Lamp Status` / `<id> Converter Status` / `<id> Converter Runtime`); a DI's device
  title wins over the item title. Alarm types `lw_demo_<id:02>_<name>` (slave) and
  `lw_demo_<name>` (device level, name must not start with two digits plus `_`) (D2); timeout text
  `Modbus <id:02>-<addr:03> Timeout Error <name>`.
- **No `lw_config*` fragments** (D10): nothing writes or reads them; apply / upgrade set the ones
  an older app left to `null`.
- **Demo digital inputs** (`DigitalInputConfig`): `index`, `itemId` (the item is the single
  source of truth for the title) + `DigitalInputSettings` (`type` daliAction | impulseCounter |
  measurement, `isActive`, `initCount`, `increment`, `unit`, `highLevelAction`,
  `lowLevelAction`, `groups`). On the device it becomes `DigitalInputEntry` (same settings +
  `title` = item title de) → `lw_demo.digitalInputs[]`.
- **`lw_demo`** (`LwDemoState`) adds, next to `slaves`: per slave `registers[]` (assigned register
  `address` + `registerType`, so upgrades never renumber), `customSlaves[]`, `customItems[]` (D6),
  `daliGroups[]`, `digitalInputs[]`, `timeZone` (D10; absent on devices from older app versions:
  the library demo's groups / inputs apply). The matching `lw_modbusWrite` has `slaveAddress`,
  `registerAddress`, `type: coil | register` (`ModbusRegisterType::write_operation_type`).
- **Device-local edits** (D6): `update_demo_setup` (custom slaves / items), `set_dali_groups`,
  `set_digital_inputs`, read back with `get_device_config`. Export (PRD §13) reads the same data.
- **Faults** (`FaultTarget`): `daliLamp`, `daliBus`, `slaveTimeout`, `slaveAlarm {slaveKey,
  alarmName}`, `customAlarm {itemId}`, `sensor`. A fault in effect is an `ActiveFault` with the
  alarm's `status` (`OpenAlarmStatus`: `ACTIVE` | `ACKNOWLEDGED`); it ends when the alarm is
  CLEARED in Cumulocity (D9).
- **Runtime naming** (`MeasurementNaming`): `type`, `fragment`, `series[] {name, unit?}`
  (a DALI lamp has four series); `ItemRuntimeState.naming` is `None` for `customAlarm`.
- **Scenario steps** (`ScenarioStep`): `{kind:"value",itemId,value}`,
  `{kind:"generator",itemId,generator}`, `{kind:"fault",target,active}`. A `Scenario` has an
  optional `description {de, en}`.
- **Device scenarios** (D11): `lw_demo.customScenarios[]` (a scenario with a library id replaces
  it in place, others follow) and `lw_demo.hiddenScenarios[]` (deleted library ids);
  `setup::merge_scenarios` / `effective_scenarios` (TS mirror `mergeScenarios` in
  `src/lib/library.ts`). Validated by `validate::validate_device_scenarios`; export writes the
  merged list and warns when the start scenario does not reset the others.
- **Floorplan layouts** (`FloorplanLayout`, library-only, not IPC): plugin widgets with item
  references (`measurement {itemId, series?}`, `alarms[]` refs `item | slaveTimeout | slaveAlarm |
  type`, Modbus control `itemId`); resolved by `payload::floorplan`. `DaliGroup` levels are
  integers 0–100.

## 13. Checks

```
npm run typecheck     # tsc --noEmit
npm run lint          # biome check .
npm run test          # vitest
npm run build         # tsc + vite build
npm run rust:fmt      # cargo fmt --check
npm run rust:clippy   # cargo clippy --all-targets -- -D warnings
npm run rust:test     # cargo test (also regenerates bindings)
npm run validate:library  # ajv schema check of library/ + test fixtures
npm run check         # all of the above except build (also runs check:versions)
npm run tauri build -- --debug --no-bundle   # what CI runs on macOS; no keys needed
```

## 14. Open TODOs

- Updater: create the GitHub repository, generate the signer key, replace `OWNER/REPO` and the
  pubkey in `tauri.conf.json`, add the secrets (`docs/RELEASING.md`; README "Before the first
  release"). Only the user can do this.
- App icons: `src-tauri/icons/icon.png` is a plain Night Blue placeholder; generate the set with
  `npm run tauri icon <logo.png>`.
- `digitalInputs[].highLevelAction` / `lowLevelAction` are validated: exactly `on`, `off` or `auto`.
- Dark-mode palette review with design (`TODO(design)` in `src/styles/globals.css`).
- Texts built in Rust that the UI shows as they are: the English `warnings` of apply / upgrade /
  export (technical diagnostics naming library ids). Operation prompts are already worded in the
  UI from structured `detail`.

## 15. Cumulocity client API and test harness

### Client API (`c8y::C8yClient`, `src-tauri/src/c8y/{client,api}.rs`)

All methods are `async`, return `AppResult<…>`, log to the activity log and report 401.
Ids are strings; bodies and results are `serde_json::Value` (`c8y::id_of(&v)` reads `id`).

| Area | Methods |
|---|---|
| Generic | `get_json::<T>(path)`, `post_json::<B,T>(path, &body)`, `put_json`, `delete(path)`, `request_json(Method, path, Option<Value>) -> C8yResponse {status, body}`, `get_collection(path, params, key, page_size, limit)` (first page `withTotalPages=true`, stops at `totalPages`, else at a short page; max 50 pages, `warn!` if cut), `current_user()` |
| Inventory | `get_managed_object(id)`, `find_managed_object(id) -> Option` (404 → `None`), `query_managed_objects(&ManagedObjectQuery {query, object_type, fragment_type, text, with_parents, extra, page_size, limit})`, `create_managed_object(&v)`, `update_managed_object(id, &partial)`, `delete_managed_object(id)` |
| Child additions | `add_child_addition(parent, child)`, `list_child_additions(parent) -> Vec<managedObject>` (no server-side type filter: filter `type` yourself), `remove_child_addition(parent, child)` |
| Binaries | `upload_binary(name, mime, bytes) -> MO` (multipart `object` / `filesize` / `file`), `download_binary(id) -> Vec<u8>`, `delete_binary(id)` |
| Measurements | `create_measurement(&v)` (`…measurement+json`), `create_measurements(&[v])` (`{measurements:[…]}`, `…measurementcollection+json`; `post_json` picks the collection type automatically when the body has `measurements`) |
| Alarms | `create_alarm(&v)`, `query_alarms(source, &[AlarmStatus], &[types])`, `open_alarms(source, Option<type>)` (ACTIVE + ACKNOWLEDGED), `update_alarm_status(id, AlarmStatus)`, `clear_alarm(id)`, `clear_open_alarms(source) -> count` (lists the open alarms, one bulk `PUT /alarm/alarms?source&status=…` per status; after `200` one re-check, what is left: one `PUT` each; after `202` / a failed bulk update every listed alarm gets one `PUT` at once, no polling) |
| Events | `create_event(&v)`, `query_events(source, Option<type>, limit)` (newest first) |
| Operations | `query_operations(device, Option<OperationStatus>)`, `pending_operations(device)`, `set_operation_status(id, OperationStatus, Option<reason>)` (`failureReason` only with FAILED) |
| Realtime | `c8y::realtime::subscribe_device(&client, device_id)` (see §3) |
| Helpers | `c8y::now_iso()`, `c8y::is_http_status(&err, 404)`, `c8y::client::basic_auth_user(tenant_id, user)` |

### Realtime (`c8y::realtime`)

`subscribe_device(&client, device_id)` is sync and spawns the task; `RealtimeConfig`:

| Field | Default | Meaning |
|---|---|---|
| `poll_timeout` | 50 s | long-poll length asked via `advice.timeout` |
| `poll_grace` | 15 s | added to `max(poll_timeout, server advice.timeout)` = HTTP timeout of a connect (65 s). This per-request timeout **overrides the client's 30 s REST timeout** (`RequestSpec::timeout`); dropping it would kill every long-poll at 30 s |
| `backoff_min` / `backoff_max` | 1 s / 30 s | retry delay after `Lost`, and between repeated re-handshakes; reset only after a successful `/meta/connect` reply (or a phase that lived longer than `backoff_max`) |

Duties of the consumer (the sim session): (1) **de-duplication** is done by the worker per id
(PENDING < EXECUTING < SUCCESSFUL < FAILED, FAILED wins), but the UPDATE echo of the session's
own PUTs is a new status and is delivered: ignore it. (2) After `Disconnected` the task keeps
retrying, except for terminal errors (401, 403, `reconnect: none`) after which `recv()` returns
`None`. (3) A 401 also calls `auth_expired`: the backend clears the login and stops the session
(`AppState::expire_auth`), and the `auth-expired` event is sent only if that call really ended a
login; the UI then shows the login screen. (4) Alarm messages are not de-duplicated: `Alarm`
carries the current status (the session ignores ids it does not track and repeats of a known
status); `OpenAlarms` follows every `Connected` and carries `fetched_at` (taken before the GET), so
only alarms known before that instant may be judged as cleared. Dropping the subscription sends a
best-effort `/meta/disconnect` (2 s).

### Fake Cumulocity for tests (`c8y::fake`, `#[cfg(test)]`)

An axum server on `127.0.0.1:<random>` with in-memory state, so tests run the real client
(headers, auth, paging, multipart, realtime). Use it from any unit test in the crate:

```rust
use crate::c8y::fake::FakeC8y;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)] // multi_thread when realtime is involved
async fn my_test() {
    let fake = FakeC8y::start().await;
    let client = fake.client();                       // tenant t100, user demo → `t100/demo`
    let dev = fake.insert_managed_object(json!({ "name": "d", "c8y_IsDevice": {} }));
    // … call your module with &client …
    assert_eq!(fake.state().alarms.len(), 1);         // inspect / seed state (don't hold across .await)
    let req = fake.requests().pop().unwrap();         // method, path, query, accept, content_type,
                                                      // use_xbasic, auth_user, body (`req.json()`)
    fake.create_operation(&dev, json!({ "lw_fetchCore": {} })); // PENDING + realtime CREATE
    fake.fail_next("PUT", "/devicecontrol/operations", 500);    // fault injection (once)
    fake.drop_realtime_clients();                     // next connect → 402 → re-handshake
    fake.set_alarm_status(&id, "CLEARED", true);      // the presenter in Cumulocity (false = silently)
    fake.sink.entries / fake.sink.expired              // activity entries / auth_expired calls
}
```

Behaviour: managed objects (CRUD, paging, `type` / `fragmentType` / `text` filters, `query`
subset `has(x)` and `x eq 'v'` (arrays: contains; trailing `*`) joined by `and`), child
additions, binaries, measurements (single / collection), alarms (de-duplication of open alarms
by source + type, realtime CREATE / UPDATE on `/alarms/<source>`, bulk `PUT` by `source` / `status`,
knob `bulk_alarm_update_accepted` = answer `202` and never apply, helpers `set_alarm_status`,
`alarm_id`), events (newest first), operations (PUT pushes realtime UPDATE; `create_operation` panics and
`try_create_operation` returns the 422 body when the device is stored without
`com_cumulocity_model_Agent`, like the real API; ids the fake does not store are accepted), realtime
handshake / subscribe / long-poll connect (held ≤ `state().connect_hold`, default 0.5 s) /
disconnect; fault knobs `ignore_advice_timeout`, `always_unknown_client`, `advice_timeout_ms`; `client_with_timeout(d)` builds a client with a short REST timeout. Unsupported `query` terms (`or`, `ne`, …) panic instead of matching. Measurements / alarms / events without `time` or `source.id` get 422. Default
page size of the fake is 5 when `pageSize` is absent. Tests: `c8y/integration_tests.rs`,
`raw.rs`.
