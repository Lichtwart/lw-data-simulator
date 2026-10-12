# Phase 1 foundation review (Opus) — 2026-10-12

Status: checks pass (npm run check, npm run build). Findings to fix:

## High
1. payload/naming.rs:37-74 (+ test :122-133): `clean_title`/`di_series` build `Value03_Zapfsaeule_1` — contradicts DECISIONS D1. Delete `clean_title`; `di_value_series(i)` → `Value{i:02}`, `di_counter_series(i)` → `ImpulseCounter{i:02}`. Replace test with DI config/title mapping test. Fix stale wording in model/library.rs:40,42,251, model/session.rs:99, docs/ARCHITECTURE.md:60.
2. model/library.rs:33-48,158-187: add `customAlarm` kind (D2) with severity + text; prefer kind-tagged enum so per-kind required fields are enforced. Add `device_alarm_type(name)` → `lw_demo_<name>`; validate names so they can't collide with `^\d\d_`.
3. model/library.rs:189-208: slave `alarm` → `alarms: Vec<SlaveAlarmDefinition>` (#[serde(default)]). `FaultTarget::SlaveAlarm { slave_key, alarm_name }`; add `FaultTarget::CustomAlarm { item_id }`.
4. model/library.rs: modbus items need `register_type` (coil/holding/input; default holding; coil for switch) and deterministic register addresses; persist assigned addresses in `lw_demo` alongside `slaves` so upgrades don't renumber. Needed for writeRegisters `{name,address,type}` and matching `lw_modbusWrite` (slaveAddress+registerAddress+type).
5. model/library.rs:237-257: DigitalInputConfig must express lw_configDigitalInputs (firmware-core-v2.md §5.4): add `daliAction` type, `isActive`, `initCount`, `increment`, daliAction action/group fields, `title`. Single source of truth for DI title: demo DI entry references item id; item title (de) goes into config `title`.

## Medium
6. naming.rs:106-108 timeout text must be `Modbus {id:02}-{addr:03} Timeout Error {name}`; add test id 1 addr 5.
7. naming.rs:77-93 `assign_slaves(existing, keys)` must keep existing ids/addresses, new keys get next free id (down from 20) / address (from 240).
8. naming.rs: add all native names as constants/functions: daliBrightness `LW Dali Brightness`/`Lux<N>` unit `Lux`; daliMotion `LW Motion`/`State<N>` no unit; 4 DALI series + units; `c8y_SignalStrength.rssi` dBm; `c8y_TemperatureMeasurement.T` C. Add a system discriminator (rssi/temperature).
9. model/session.rs:90-117: `naming: Option<MeasurementNaming>`; DALI = fragment + list of series.
10. model/library.rs:268-280 scenario step → tagged enum `{kind:"value",itemId,value}|{kind:"generator",itemId,generator}|{kind:"fault",target,active}`.
11. commands.rs:615-618 don't hold `state.session` lock while awaiting task reply: SessionHandle holds cloneable Sender; clone under lock, drop guard, await.
12. sim/session.rs:61-120: detect dead session task (`is_finished()` → treat as no session; stop() ignores closed channel; task emits SessionChanged(None)/SessionError{fatal} on exit).
13. sim/session.rs + ARCHITECTURE §8: separate outbound worker (mpsc payload queue, reports results); state loop never awaits HTTP except spawned EXECUTING PUT. Add periodic lock refresh (`lw_demo.lock.since`, every ~3 min).
14. c8y/realtime.rs:18-25: subscription returns cancel-on-drop handle (JoinHandle + CancellationToken) and stream of `RealtimeMsg::{Connected, Disconnected(err), Operation(Value)}`; on every (re)connect GET PENDING ops for the device and feed them.
15. c8y/client.rs: pass AppHandle (or event sink) into C8yClient so it can emit `activity-logged` and `auth-expired`; on 401 clear AppState.auth and stop session in backend.
16. commands.rs:444-449 `set_item_value(value: f64)` → request struct with `#[specta(type=Number)]`; same for future f64 args; document in AGENTS.md.
17. Contract gap: device-local slaves/items/DI titles/group edits. DECISION (orchestrator): add `customSlaves` + `customItems` to `lw_demo`, plus patch fields and commands to edit `lw_configGroups`, `lw_configDigitalInputs`, custom slaves/items (demo editor, PRD §7.4/§11.3 "generate series" helper, export round-trip §13). Record as D6 in docs/DECISIONS.md.
18. Export folder picker: add `tauri-plugin-dialog` + `dialog:allow-open` capability.
19. commands.rs:52-82 `login` stopping a session must emit SessionChanged(None) via shared helper with logout.
20. lib.rs:132-141 + library/loader.rs:23-31: load tenants.json separately; skip bad item/demo files with a warning (surface in library_info); check `type`/`schemaVersion`.

## Low
21. `saved_logins` must not read every password (keychain prompts): store usernames in app config/store; read secret only in `login_saved`; keychain errors → empty list.
22. ActivityEntry stores base_url + user so `to_curl` needs no client; mask `-u 'tenant/user:****'`; never log headers.
23. LoginScreen.tsx:62 `dark:invert` → use lichtwart-white.svg in dark mode.
24. Add src/i18next.d.ts CustomTypeOptions so typo keys fail tsc; `errorKey` passes `details` as interpolation values.
25. `.lucide { stroke-width: 1.5 }`; dark palette: redefine status `*-bg` and destructive.
26. Split commands.rs into commands/{auth,library,devices,session,raw,activity,app}.rs, each exporting its own list for collect_commands!, so parallel agents don't collide.
27. Add .gitkeep to library/items, demos, floorplans.

## Resolution

All 27 findings are addressed. `npm run check` and `npm run build` pass. Generated bindings were regenerated (`npm run gen:types`).

1. Fixed. `clean_title` / `di_series` deleted. `di_value_series(i)` gives `Value{i:02}`, `di_counter_series(i)` gives `ImpulseCounter{i:02}`. The old test is replaced by `di_series_have_no_title` and the DI config / title mapping test `di_config_and_title_mapping` (`payload/config.rs`, matches the firmware §5.4 example). Stale wording fixed in `model/library.rs`, `model/session.rs`, ARCHITECTURE, `payload/mod.rs`.
2. Fixed. `LibraryItem` is now a `kind`-tagged enum with struct variants, so each kind has exactly its own required fields; unknown fields are rejected. New kind `customAlarm` (`severity`, `text`). Added `device_alarm_type(name)` (`lw_demo_<name>`), `is_valid_alarm_name` and `is_valid_device_alarm_name` (rejects `^\d\d_`). Changed approach on the shape: serde newtype variants with `tag` generate a wrong TypeScript type in specta rc.25, and `flatten` is not allowed on tuple variants, so the variants carry their fields inline instead of wrapping a shared struct.
3. Fixed. `SlaveDefinition.alarms: Vec<SlaveAlarmDefinition>` (default empty). `FaultTarget::SlaveAlarm { slaveKey, alarmName }` and `FaultTarget::CustomAlarm { itemId }`.
4. Fixed. `ModbusRegisterType` (coil / holding / input) on Modbus items; default holding, coil for a switch control (`LibraryItem::register_type()`). `assign_registers` gives deterministic addresses (coil from 1, input from 30001, holding from 40001). `SlaveAssignment.registers` (`RegisterAssignment`) persists them in `lw_demo.slaves`. Helpers map to the `writeRegisters` type (`coil` / `holding`) and the `lw_modbusWrite` type (`coil` / `register`). Chosen address bases are a convention (firmware takes any raw 0-65535).
5. Fixed. `DigitalInputType` has `daliAction`. `DigitalInputSettings` carries `isActive`, `initCount`, `increment`, `unit`, `highLevelAction`, `lowLevelAction`, `groups`. Demo `DigitalInputConfig` references the item by `itemId` (single source of truth), and the device-side `DigitalInputEntry` has `title` = item title (de). `digital_inputs_fragment` writes the firmware shape. `highLevelAction` / `lowLevelAction` are free strings because only `on` / `off` are evidenced in the firmware docs.
6. Fixed. Timeout text is `Modbus {id:02}-{addr:03} Timeout Error {name}`; test for id 1 / address 5 added.
7. Fixed. `assign_slaves(existing, keys)` keeps existing ids / addresses and registers, new keys take the next free id (down from 20) and address (down from 240). Entries not in `keys` are dropped and free their slot. Tests added.
8. Fixed. Constants and functions for all native names (DALI series + units, `Lux<N>` unit `Lux`, `LW Motion` / `State<N>` without unit, rssi `dBm`, temperature `T` / `C`), plus `item_naming()` for every kind. `SystemKind { Rssi, Temperature }` is the system discriminator on `system` items.
9. Fixed. `ItemRuntimeState.naming: Option<MeasurementNaming>`; `MeasurementNaming` = type + fragment + `series: Vec<SeriesNaming { name, unit? }>` (DALI has four).
10. Fixed. `ScenarioStep` is the tagged enum `value` / `generator` / `fault`.
11. Fixed. `SessionSender` (cloneable) is cloned under the lock and the guard dropped before awaiting (`AppState::session_sender`). Stop takes the handle out and awaits outside the lock.
12. Fixed. `SessionHandle::is_finished()`; `AppState` prunes dead handles (treated as no session, emits `session-changed(null)`); `stop()` ignores a closed channel; `announce_exit` is the exit hook for the task (`session-error` fatal + `session-changed(null)`). `start()` itself is still a stub, so the task calling `announce_exit` is documented, not yet exercised.
13. Fixed (shape and docs; behavior stubbed with the session). New `sim/outbound.rs` worker: bounded queue, non-blocking `enqueue`, ordered results channel; the EXECUTING `PUT` exception is documented. ARCHITECTURE §8 rewritten. `LOCK_REFRESH_INTERVAL` (3 min) constant added for the periodic `lw_demo.lock.since` refresh.
14. Fixed. `RealtimeSubscription` (cancel-on-drop via `CancellationToken` + task abort; added `tokio-util`) yields `RealtimeMsg::{Connected, Disconnected(err), Operation(Value)}`. The contract (GET PENDING ops on every (re)connect and feed them) is in the module docs; `subscribe_operations` stays a stub.
15. Fixed. `C8yClient` takes an `Arc<dyn EventSink>`; `TauriSink` emits `activity-logged` and on 401 clears `AppState.auth`, stops the session and emits `auth-expired`. Changed approach: an `EventSink` trait instead of passing `AppHandle` directly, so the client stays testable (`NoopSink`). The client exposes `record()` / `check_unauthorized()` for the HTTP implementation to call.
16. Fixed. `set_item_value(request: SetItemValueRequest)` with a `Number`-annotated `value`. Rule documented in AGENTS.md, ARCHITECTURE §9 and `commands/mod.rs`.
17. Fixed. Added D6 to DECISIONS.md. `lw_demo.customSlaves` / `customItems`; `DemoSetupPatch` fields for them; new commands `get_device_config`, `set_dali_groups`, `set_digital_inputs`, and `generate_item_series` (the "generate series" helper, stub). `DaliGroup` gained the firmware's optional fields. PRD §6.3 / §11 marked. Commands that write to the device remain stubs.
18. Fixed. `tauri-plugin-dialog` (Rust) and `@tauri-apps/plugin-dialog` (JS) added, plugin registered in `lib.rs`, capability `dialog:allow-open`.
19. Fixed. `login`, `logout`, `stop_session` and the device commands all use `AppState::stop_session` / `stop_session_for`, which emit `session-changed(null)`.
20. Fixed. Tenants load separately (`load_tenants`, stored in `AppState.tenants`; `Library` no longer holds tenants). Bad item / demo files are skipped with a warning shown in `library_info`; `type` and `schemaVersion` are checked. Tests cover bad, wrongly typed and future-version files, and tenants loading while items are broken.
21. Fixed. User names go to `<app config dir>/saved-logins.json`; the keychain secret is only the password and is read only in `login_saved`. `saved_logins` reads the file only (a problem gives an empty list), `forget_login` removes both.
22. Fixed. `ActivityEntry` now has `baseUrl` and `user`; `to_curl(entry)` needs no client, masks `-u 'tenant/user:****'`, never writes `Authorization`. `activity_as_curl` no longer needs a login. Test added. Limitation: bodies are cut at 2 kB, so a curl for a long body is incomplete (noted in the doc comment).
23. Fixed. Login screen shows `lichtwart-blue.svg` in light mode and `lichtwart-white.svg` in dark mode.
24. Fixed. `src/i18next.d.ts` types the keys from `de.json` (a typo fails `tsc`, verified). `errorKey` returns a typed `ErrorKey`; new `errorValues` passes `details` as interpolation values. The `errors.*` texts for http, network, notFound, invalidInput, library, sessionActive and deviceLocked now use them (de + en).
25. Fixed. `.lucide { stroke-width: 1.5 }`; the dark palette redefines `--success-bg`, `--warning-bg`, `--danger-bg`, `--info-bg`, `--destructive` and its foreground, derived from tokens with `color-mix` (design review of these values is still open).
26. Fixed. `commands.rs` split into `commands/{app,auth,library,devices,session,raw,activity}.rs`. Changed approach for registration: tauri-specta's `Commands` cannot be merged, so each file ends with a `<area>_commands!` macro and `commands::collect()` chains them into one `collect_commands!`. Agents add commands only in their own file.
27. Already satisfied: `library/items`, `library/demos` and `library/floorplans` each contain a `.gitkeep`. No change needed.
