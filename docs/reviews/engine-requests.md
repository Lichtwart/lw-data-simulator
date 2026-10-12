# Requests from the domain fixer to the engine agent

Written by the agent fixing `docs/reviews/phase2-domain.md`. Everything below needs a change in a file
owned by the engine work (`sim/{session,scheduler,operations,faults,scenario,outbound,core}.rs`, `device/**`,
`commands/{devices,session}.rs`, `model/{session,device,ops}.rs`). Public payload / library APIs the engine
uses are unchanged; only additions are listed under "New API".

## New API (all additive, old signatures still exist)

- `sim::generators`
  - `GeneratorState::seeded(v)`, `seed_counter(v) -> bool` (only raises), `accepts_counter_value(v)`,
    `counter_value_problem(v) -> Option<String>`.
  - `counter_seed(default, snapshot, newest_remote) -> f64` = max of the three (never lowers).
  - `local_hour_in_zone(unix_secs, tz) -> Option<f64>` (DST-safe, `chrono-tz`), `DEFAULT_TIME_ZONE`.
  - `randomWalk` with `last` outside its band now drifts back by `step` per tick.
- `payload::measurement`: `quantize(type, series, value, Option<CounterGrid>)`, `CounterGrid {init_count,
  increment}`, `item_values_with_grid(naming, value, dali, grid)`. `item_values` now quantizes too (DI
  counters floor to integers; use the `_with_grid` variant for `initCount` / `increment`).
- `payload::naming`: `item_naming_with_di(item, slave, Option<&DigitalInputEntry>)`; `item_naming` delegates with
  `None`.
- `payload::alarm`: `sensor_alarms(kind, index)`, `resolve_sensor_alarm(kind, index, alarm_type)`,
  `sensor_fault_entries(items)`, `is_native_alarm_type(t)`, `SENSOR_BRIGHTNESS` / `SENSOR_MOTION`.
  `dali_bus_alarms()` now has 6 entries (added `lw_ntpSyncFailed`), so the fault catalog grows by one.
- `payload::config`: `config_core_fragment_for(lamp_count)`; `demo_device_body` uses it with
  `lw_demo.dali_lamp_count` (`maxControlGears = max(55, count)`).
- `model::FloorplanWidget::icon()`.
- Behaviour changes callers may notice: writable holding registers default to datatype `uint16`; a DALI control
  widget whose group is missing in `ctx.groups` now fails with `NotFound("group N")` instead of a "Group N" title;
  DI widget fallback label is `Eingang <N>`; `item_values` output is quantized; generated series ids are
  `<id>-<n>` for native kinds and `index == n`; `assign_slaves` / `assign_registers` reject duplicate keys / ids;
  `DigitalInputConfig` / `DigitalInputEntry` deserialize strictly (unknown fields are errors, e.g. a typo in
  `set_digital_inputs` input); `DemoPack.dali_lamp_count` and `DigitalInputConfig.index` may be 0 right after
  parsing, the loader fills them (library demos are always resolved).
- New dependency: `chrono-tz = "0.10"` in `src-tauri/Cargo.toml`.

## Requests

1. **#4 sensor alarms (hookup).** Add to `FaultTarget` (model/session.rs). The serde tag is `kind`, so the field
   cannot be called `kind`:
   ```rust
   /// Native alarm of a DALI brightness / motion sensor (`lw_brightness_1_NN_lost`, ...).
   Sensor {
       #[serde(rename = "sensorKind")]
       sensor_kind: String, // "brightness" | "motion"
       index: u8,           // 1..=10
       alarm: String,       // alarm type
   },
   ```
   The library schema already accepts `{"kind":"sensor","sensorKind":..,"index":..,"alarm":..}`. Then wire the
   three arms (my helpers are ready; I did not add the arms because the variant did not exist yet):
   - `payload/alarm.rs::fault_catalog`: after the lamp loop, for `(kind, index, spec) in
     sensor_fault_entries(setup.items)` push `definition(FaultTarget::Sensor { .. }, spec)`.
   - `payload/alarm.rs::resolve_fault`: `FaultTarget::Sensor { sensor_kind, index, alarm } =>
     resolve_sensor_alarm(sensor_kind, u32::from(*index), alarm)` (also check the sensor item exists in
     `setup.items`).
   - `library/validate.rs::fault_problem`: arm for `Sensor` (selected item of that kind and index, alarm type in
     `sensor_alarms`).
   - Engine: while a `_lost` / `_notExisting` fault is active the firmware stops sending that sensor's
     measurement (`:4325-4334`); optional.
   (If the variant landed with another shape, adapt the three arms above.)
2. **#1 counters.** At session start seed counter items with
   `GeneratorState::seeded(counter_seed(default, snapshot, newest_remote))` (newest remote:
   `GET /measurement/measurements?source&valueFragmentType&valueFragmentSeries&revert=true&pageSize=1`); reject
   manual and scenario values with `state.counter_value_problem(v)`. Validation already rejects scenario `value`
   steps below a counter's `control.default`.
3. **#3 quantization / #2 DI source of truth.** For DI counters call `item_values_with_grid` with the device DI
   entry's `initCount` / `increment`; for DI items use `item_naming_with_di(item, slave, matching
   DigitalInputEntry)` so series and unit follow the device config (`sim/session.rs:171`, `sim/core.rs:231`,
   `device/export.rs:204` currently call `item_naming`).
4. **#13 dailyProfile timezone.** `Tick.local_hour` should come from `local_hour_in_zone(now,
   lw_configCore.timeZone)` (fallback `DEFAULT_TIME_ZONE`); `sim/core.rs:950` currently uses
   `local_hour(wall, local_offset_secs())` (the laptop's offset).
5. **#6 DI actions typed.** `DigitalInputSettings.high_level_action` / `low_level_action` are still
   `Option<String>` (the model type is shared with `device/setup.rs::parse_digital_inputs` and the frontend).
   Schema and validation now require lowercase `on|off|auto`. Please make
   `device/demo_state.rs::normalize_digital_inputs` reject other values (or lowercase them) and then switch the
   model to `Option<GroupMode>` together with `parse_digital_inputs` and `DigitalInputsEditor.tsx` (I can do the
   model + schema side on request).
6. **#15 byte limits in engine code.** `device/demo_state.rs:321` truncates the DI unit with `chars().take(19)`;
   use a byte-safe cut (`payload::config` has the private `truncate_bytes`, I can make it `pub`).
   `device/export.rs` (`short` / `MAX_NAME_LEN`) counts chars; item ids are ASCII so only slave names matter
   (byte length).
7. **#19 export drops equal DI titles.** In `device/export.rs`, when exporting a `sensor` widget on a DI item
   (`Value<NN>` / `ImpulseCounter<NN>`), omit `title` if it equals the `lw_configDigitalInputs` title.
8. **#9 maxControlGears.** When `update_demo_setup` / apply changes `daliLampCount` on an existing device,
   rewrite `lw_configCore.maxControlGears = max(55, count)` (`payload::config::config_core_fragment_for`).
9. **#22 docs.** `model/device.rs:261` (`GenerateSeriesRequest` doc) still says "native `index` counting up from
   the template's"; now `index = n` (`light-5` is lamp 5) and the id separator is `-` for native kinds. A
   frontend preview of generated ids, if any, must match.
10. **Bindings.** `src/lib/bindings.ts` needs `npm run gen:types` after both of us landed (new doc comments, and
    `DemoPack_Deserialize.daliLampCount` becomes optional).

## Engine status (Phase 3A agent, 2026-10-12)

Done from the list above: 1 (variant `FaultTarget::Sensor { sensorKind, index: u8, alarm }` + the arms in
`payload/alarm.rs::{fault_catalog,resolve_fault}` and `library/validate.rs::fault_problem`; the engine also stops a
sensor's measurement while a `_lost` / `_notExisting` fault is active), 2 (`counter_seed` at start, newest remote
value via `revert=true&pageSize=1`; manual / scenario values below the current counter are rejected), 3
(`item_naming_with_di` / `item_values_with_grid` in session + export), 4 (device `lw_configCore.timeZone`), 5 (actions
validated and lower-cased in `normalize_digital_inputs`; the model stays `Option<String>`), 6 (`truncate_bytes`), 7, 8
(`maxControlGears` rewritten on apply and when `update_demo_setup` changes `daliLampCount`), 9, 10.

### Requests to other areas

- **c8y fake:** `POST /inventory/managedObjects/{group}/childAssets` is not supported, so `create_demo_device` with a
  `parentGroupId` is untested; `withParents=true` does not fill `assetParents` (device-list grouping untested).
  `GET /measurement/measurements` ignores `revert` (counter seeding is tested with a seeded value instead).

## Frontend follow-ups (IPC changes, `npm run gen:types` done)

`npm run typecheck` currently fails only in these frontend files because of the new fields:

1. `SessionSnapshot.revision: number` (monotonic per session). Same counter on `ItemValuesChanged.revision`,
   `FaultChanged.revision`, `OperationPrompt.revision`, `OperationResolved.revision` (contract request 7). Replace the
   cast in `revisionOf`; fix test fixtures in `src/features/session/sessionState.test.ts`,
   `OperationPromptCard.test.tsx`.
2. `OperationPrompt.deviceId`, `OperationResolved.deviceId` (request 6).
3. `DemoDeviceSummary.upgradeAvailable: boolean` (request 1) — same rule as `DemoDevice.upgradeAvailable`; fix
   `src/features/devices/groupDevices.test.ts`, drop the client-side `isOlderVersion` badge rule.
4. `OperationDecision` abort: `reason` is optional (default `Aborted by user`, request 3).
5. `FaultTarget` has a new variant `{ kind: "sensor", sensorKind: "brightness" | "motion", index, alarm }`; the
   fault catalog lists it for every brightness / motion item. `src/features/control/faults.ts:4` needs the case.
6. `session-changed` is emitted after every periodic send (requests 2 / 10): re-enable "last sent".
7. `session-error.code` is always an `AppError` code (`http`, `network`, `invalidInput`, `notFound`, `internal`, …;
   request 11). Repeated errors with the same code are throttled to one per 30 s.
8. `update_demo_setup.generators` has replace semantics (confirmed); a running session on that device picks up
   generators / interval / realistic mode at once. Item selection changes (includes / custom items) apply at the next
   session start.
9. Custom select controls: option values must be unique and `default` one of them (`update_demo_setup` rejects
   otherwise, request 5).
10. `DemoDevice.floorplans` lists **all** `lw_floorplan` child additions; app-created ones are `demo.floorplanIds`.
11. `lw_demo` gained `appliedItems` / `appliedFloorplans` (digests for the upgrade diff; ignore in the UI).
12. `apply_demo` without `replaceExistingFloorplans` skips a same-name floorplan and returns a warning; preview lists it
    in `floorplanConflicts`.


## Validation proposals (from the phase 3 library fixer)

1. **Start scenario must be a full reset.** For every demo: each `value` / `generator` target and each `fault`
   target touched by a non-start scenario must also appear in the start scenario (a `generator` step for items
   with a generator, `value` otherwise, `fault ... active:false` for faults). Report as a load warning. The
   shipped demos satisfy this (their `normal` is built that way).
2. **Fault scenarios must not hide their own effect.** Warn when one scenario sets a `value` on a Modbus item
   and activates `slaveTimeout` for that item's slave (the timeout stops the measurement).
3. **Effect alarms in floorplan `autoAlarms`.** `item_alarm_types` (payload/floorplan.rs) returns nothing for a
   digital input, so a DI `effects` alarm (`lw_demo_restroom_leak`, `lw_demo_dry_leak_alarm`) is not watched
   by its sensor widget unless listed as `{kind:"type"}`. Proposal: include the item's `ItemEffect::Alarm`
   types in `item_alarm_types`; the library then can drop the explicit refs.
4. **Slave alarms of unselected registers.** Optional: drop slave alarms whose name is an item id that is not
   selected (the library now avoids this by slave split instead).
5. **Lux logic.** `automaticSwitch` / `luxThreshold` are not simulated; outdoor lamps are set by scenarios only.


### Status of the validation proposals (engine fixer, round 2)

1. **Done** (`library/validate.rs::check_scenario_resets`): a load warning for every step of a non-start scenario
   whose item / fault the start scenario does not reset (also when a demo has scenarios but no start scenario). The
   shipped `library/` loads with zero warnings.
2. **Done** (`check_scenario_hidden_values`): a `value` on a Modbus item together with `slaveTimeout active:true` for
   its slave in the same scenario is a load warning.
3. **Done** (`payload::floorplan::item_alarm_types`): the alarms an item's `effects` raise are part of
   `AlarmRef::Item` (all item kinds), deduplicated. The explicit `{kind:"type"}` refs in the shipped floorplans still
   work (the list is deduplicated), so the library can drop them at its leisure.
4. **Not done** (optional, and a payload / alarm-catalog change, not a validation rule): no slave alarm is dropped.
5. **Not applicable** (a note: lux logic is not simulated).

Also changed in `validate.rs`: `sendIntervalSec` is now 5-899 (was 1-899, review finding 38), and `inventory` effects
must not write `lw_demoLock` / `lw_demoSnapshot` (the new lock / snapshot fragments, review finding 10). The JSON
schema `library/schema/demo.schema.json` still says `minimum: 1` for `sendIntervalSec`; raise it to 5 (not touched
here, `library/**` belongs to the library agent).

## Frontend follow-ups (round 2)

**Status: all items handled in `src/**`** (items 5, 11-13 need no UI; item 10 needs none either: the form shows the error and creates no device).

IPC changes from the phase 3 engine fixes (`npm run gen:types` done, `npm run typecheck` still passes; nothing in
`src/**` was edited). All new fields are additive; none of this blocks the build, but the UI should adopt it:

1. **[done]** `UpgradePreview` has five new (required) fields: `floorplanConflicts: FloorplanRef[]` (same-name floorplans the app
   did not create: the upgrade skips those layouts, never replaces them), `floorplansEdited: string[]` (app floorplans
   somebody edited in the plugin: kept, the new layout is created next to it as `<name> (v<version>)`),
   `demoChanges: string[]` / `keptOverrides: string[]` (demo-pack level fields taken from the new library / kept
   because the device value differs: `tags`, `includes`, `excludes`, `daliLampCount`, `daliGroups`,
   `digitalInputs`, `sendIntervalSec`), `customNowInLibrary: string[]` (custom slave / item ids that the library has
   now). Show them in `UpgradeCard.tsx`; fix any test fixtures that build an `UpgradePreview`.
2. **[done]** `OperationResolution` has a new value `withdrawn` (the prompt was withdrawn because the operation became final
   elsewhere; `status` / `reason` are what was observed). Add it to the activity / toast rendering and the i18n keys.
3. **[done]** `ApplyDemoRequest.force?: boolean` (optional): `apply_demo` / `create_demo_device { applyDemo }` now fail with
   `deviceLocked` when another presenter's fresh lock is on the device. Offer "take over" and retry with `force: true`
   (same dialog as `start_session`).
4. **[done]** `apply_upgrade` never replaces an edited or foreign floorplan any more. The result carries no warnings, so the
   preview (finding 1 above) is the only place to tell the presenter what will be skipped or kept.
5. `LwDemoState` gained `retiredSlaves?`, `appliedDemo?`, `appliedFloorplanBodies?` (internal; ignore).
   `LwDemoState.lock` / `.snapshot` are still filled by the backend, but live in the `lw_demoLock` /
   `lw_demoSnapshot` fragments on the device (a lock refresh no longer rewrites `lw_demo`).
   `LwDemoState.slaves` lists the **selected** slaves only; a deselected slave keeps its numbers in `retiredSlaves`.
6. **[done]** `sendIntervalSec` minimum is 5 s now (UI slider / input validation, `set_send_interval`, `update_demo_setup`,
   library editor); the backend rejects less with `invalidInput`.
7. **[done]** `set_dali_groups` / `set_digital_inputs` now reach a running session on that device at once (groups keep their
   mode; digital input title / unit / actions apply immediately). No "restart the session" hint is needed any more.
8. **[done]** A session start now restores state from Cumulocity: group modes (newest `lw_daliGroupChange` per group), open alarms
   (`activeFaults` may be non-empty right after the start, their `alarmId` is set), the last SUCCESSFUL
   `lw_modbusWrite` value of writable registers (those items come up with `manualOverride: true`). A fault whose alarm
   `POST` failed is re-emitted via `fault-changed` with `alarmId: null` and can be cleared or raised again.
9. **[done]** New fatal `session-error` cases (the UI already handles `fatal`): code `deviceLocked` (another presenter took the
   lock over while the session ran), `internal` "the outbound worker stopped; the session cannot send", and
   `internal` "the simulation session crashed" (a panic). The task now emits `session-changed(null)` itself in all
   of them; the snapshot shows `status: "stopping"` while the shutdown drains (at most 5 s).
10. `create_demo_device` with `applyDemo` that fails during the apply now **deletes** the new device again (like a
    failed parent link) and returns the error; the UI must not expect the device to exist.
11. Manual `set_item_value` changes are sent 300 ms after the first change of a burst (slider drags make a handful of
    requests); the UI value updates at once.
12. `OperationPrompt.by` also comes from `creationRequest.user` / `user` when the operation has no `by`.
13. Quitting the app now stops the running session first (lock released, open prompts failed with "Simulator
    stopped"), waiting at most 6 s; nothing to do in the UI.
