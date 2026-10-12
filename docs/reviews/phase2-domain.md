# Phase 2 domain core review (Opus): 2026-10-12

Scope: `library/schema/**`, `scripts/validate-library.mjs`, `src-tauri/src/library/**`,
`src-tauri/src/payload/**`, `src-tauri/src/sim/generators.rs`, `src-tauri/src/model/library.rs`,
`src-tauri/tests/fixtures/**`, AGENTS.md "Library format".

Checks run:
- `cargo test --lib -- library payload generators`: 62 passed, 0 failed. The first attempt did not compile
  because of another agent's in-progress edit in `src/error.rs:32`. The retry compiled.
- `npm run validate:library`: 9 files checked, 0 invalid.

Ground truth was re-checked against the sources, not only the research docs:
- firmware `devel`: `C8Y_str_parser_getter.c:4541,4561,4577`, `C8Y_light_system.c:314-329,751`,
  `ERM_error_manager_publish.c:555-711`, `ERM_alarm_blacklist.h:44-81`,
  `MBM_modbus_master.c:732-770,1540-1570`, and the DI and register parsers `:600-700,1350-1425`;
- davaco-demo: `settings/modbus-slaves.ts`, `floorplan-device-lookup.service.ts`,
  `control-settings.component.ts:337-347`, `model/managed-object.ts`.

## Verified correct (no action)

- DALI: type `daliControlGear`, fragment `LW Dali Control Gear 1-NN`. The series order is Converter Status, Lamp Status,
  Converter Runtime (`hr`), Brightness (`%`). Empty units are omitted. Status 1 = OK. At most 6 lamps per
  collection.
- DI: `Value<NN>` / `ImpulseCounter<NN>`, fragment `LW Digital Input`, type `digitalInput`. The title is not in
  the series name (D1).
- Modbus: fragment `LW_Modbus_Slave_<id:02>_<addr:03>`, series = item id. The timeout alarm
  `lw_modbus_NN_timeoutError` is MAJOR with text `Modbus %02u-%03u Timeout Error <name>`.
- All 14 lamp alarms, 4 brightness, 2 motion and 5 bus/system alarms: types, texts and severities match
  `ERM_GetAlarmInfo` exactly, including `referenceMeasuerment`.
- Events:
  - `lw_daliGroupChange`: the text format, a 1-based `id`, and `On`/`Off`/`Auto` match the firmware.
  - `lw_alarm` and `lw_modbus` (plural case) also match.
- Config fragments:
  - `lw_configCore` has all 13 fields with the research defaults.
  - The `lw_configGroups` record shape matches.
  - `lw_configDigitalInputs` matches the doc example.
  - `lw_configModbusSlaves`: the field names (`registers[].{name,unit,address,type,datatype,scalingFactor,swap,
    changeOfValue,conversion,custom}`), coil→`bool`, `isSavingOnStorage:false`, and
    `writeRegisters {name,address,type: coil|holding}` all match.
- Floorplan plugin contract:
  - `ref = slaveAddress:registerAddress` and `holding→register` match the plugin.
  - `readback` uses the item's own measurement.
  - DALI/brightness/motion alarm derivation equals the plugin's suggestions.
  - The DI widget is a plain sensor with `alarmTypes: []`.
  - `imageBinaryId` is a string.
- Selection is `tags ∪ includes − excludes`, and excludes win. Search ANDs words over title de/en, id, tags and
  slave name, case-insensitive. This matches PRD §11.4 and §11.5.
- Schema ⇄ serde: the round-trip and breakage tests are a good design.

## High

1. **sim/generators.rs:55-60, 107-110; library/validate.rs:620 — counter monotonicity is not achievable with the
   current state model.**
   - Problem:
     - `GeneratorState.last` exists only in memory.
     - The counter start falls back to `base`, the control default (e.g. `1000`).
     - After an app restart or session switch without "Save snapshot", an `ImpulseCounter`/energy total goes
       *backwards* in Cumulocity. PRD §8 says "counter: monotonic, e.g. energy totals".
     - A manual value or a scenario `value` step lower than the current counter is also accepted
       (`value_problem` only checks ≥ 0).
   - Fix:
     - Treat counter items specially. On session start, seed `last` from the newest stored measurement of the
       series (`GET /measurement/measurements?source&valueFragmentType&valueFragmentSeries&revert=true&pageSize=1`),
       falling back to the `lw_demo.snapshot` value, then to the default.
     - In the session, reject manual or scenario values below the current counter.
     - Add a test "resume after restart never decreases".

## Medium

2. **payload/naming.rs:129-133, payload/config.rs:331-338, model/library.rs:289-307 + 567-597,
   validate.rs:817-931 — the DI impulse counter has two sources of truth.**
   - Problem: the measurement unit comes from `item.unit`, while `lw_configDigitalInputs[].unit` comes from the
     demo DI entry's `unit`. Two more pairs are duplicated the same way:
     - the start value: `control.default` vs `initCount`;
     - the mode: item kind `digitalInput`/`impulseCounter` vs DI `type`.
   - Problem: nothing checks that `unit` or `initCount` agree. After D6 the device config is edited directly
     (`set_digital_inputs`), but naming still reads the library item. The firmware takes the unit from the
     config (`C8Y_light_system.c:478-480`).
   - Fix (pick one source):
     - drop `unit` from `impulseCounter` items, take the series unit from the DI config, and let the counter
       start at `initCount`;
     - or keep both and add a validate rule `item.unit == di.unit` and `control.default == initCount`.
   - Fix: make `item_naming` for DI items take the `DigitalInputEntry`, so the type and unit follow the device.

3. **payload/measurement.rs:57-91 — values are not quantized to firmware types.**
   - Problem: the firmware only emits integers for:
     - Brightness `%u` and Converter Runtime `%u`;
     - DI `Value` (`%u`, 0/1) and `ImpulseCounter` (`%u`, `initCount + n·increment`);
     - `Lux` (`%u` int) and motion `State`;
     - rssi (int).
   - Problem: temperature is `%.1f`. `item_values` passes generator output through unchanged, so a sine on a
     lamp sends `57.349 %` and a counter generator sends `1000.1` pulses. A random walk on a DI would break the
     plugin's boolean display, which needs exactly `onValue`/`offValue`.
   - Fix: add `quantize(kind/series, v)` in `payload::measurement`:
     - round to an integer for DALI/lux/motion/rssi/DI `Value`, and clamp DI `Value` to {0,1};
     - floor a counter to `initCount + k·increment`;
     - round temperature to 0.1;
     - leave Modbus raw.
   - Fix: add a unit test per kind.

4. **payload/alarm.rs:292-335, model/session.rs:54-80, payload/floorplan.rs:61-62 — sensor alarms are watched but
   cannot be raised.**
   - Problem: a brightness or motion sensor widget automatically gets `lw_brightness_1_NN_*` /
     `lw_motion_1_NN_*` in `alarmTypes`. However, `FaultTarget` has no sensor variant and `fault_catalog`
     omits them, so the presenter can never trigger them.
   - Fix: add `FaultTarget::DaliSensor { kind: brightness|motion, index, alarmType }`. Include
     `brightness_alarms`/`motion_alarms` for the selected sensor items in the catalog and in `resolve_fault` and
     `fault_problem`. The schema `faultTarget` needs the same addition. Optionally suppress the sensor's
     measurement while `_lost`/`_notExisting` is active, as the firmware does (`:4325-4334`).

5. **payload/config.rs:170-189; validate.rs:321-363 — writable holding registers have an inconsistent data type
   and value range.**
   - Problem: `lw_modbusWrite` with `type: register` is FC6, one 16-bit word with value 0-65535 (an integer).
     A writable holding item without `datatype` is written as `float32` (2 words). The fixture `ac_setpoint`
     shows this.
   - Problem: nothing stops a writable holding item with a slider like `-30…0, step 0.5`. Its app value and
     readback (e.g. `-18.5`) can never equal what the plugin writes, so the plugin shows "mismatch".
   - Fix: in `register_datatype`, default writable holding items to `uint16`; reject or warn on `float32`/`*32`
     with `writable: true`.
   - Fix: in validate, require for writable holding items a control with integer `min ≥ 0`, `max ≤ 65535` and
     integer `step`, or select option values that are integers in 0-65535.

6. **library/schema/demo.schema.json:91-92, model/library.rs:588-593, validate.rs:894-900 —
   `highLevelAction`/`lowLevelAction` accept any string.**
   - Problem: the firmware accepts `on`/`off`/`auto`/`""`. The matching is lowercase `strstr`
     (`C8Y_str_parser_getter.c:1352-1390`), so `"On"` or `"ein"` fails `lw_fetchDigitalInputs`. The simulator
     also has to interpret the value for DI-driven group switching.
   - Fix: schema `"enum": ["on","off","auto"]`; model `Option<GroupMode>`; drop the now-redundant validate
     string checks. Document the values in AGENTS.md.

7. **validate.rs:384-426, common.schema.json:184,200 — multi-effect items can overwrite reserved fragments or fake
   native events.**
   - Problem: an `inventory` effect may name `lw_configCore`, `lw_configGroups`, `lw_demo`, `c8y_Hardware`,
     `c8y_Firmware`, `name` or `type`. Writing to them can hide the floorplan tab, hide Modbus controls, or
     corrupt setup.
   - Problem: an `event` effect may use `lw_daliGroupChange`. The plugin derives DALI state from the newest
     such event, so an event without `groupModeChange` breaks it. `lw_alarm` and `lw_modbus` are similar cases.
   - Fix: reject inventory properties matching `^(lw_config|lw_demo$|c8y_)` or the MO core fields. Reject event
     types `lw_daliGroupChange|lw_alarm|lw_modbus`. Recommend the `lw_demo*` prefix for both. Mirror the rule
     in the schema with a `not` pattern.

8. **validate.rs:767-796 vs 817-931 — the "ungrouped DALI lamp" rule is fine, but strictness is inconsistent.**
   - Opinion: the lamp rule is sensible. The firmware never sends ungrouped lamps (`:508`), so the control and
     floorplan widget would be dead and the series would never reach `supportedSeries`.
   - Problem: the same reasoning is not applied to a selected `digitalInput`/`impulseCounter` item that has no
     `digitalInputs[]` entry, or one with `isActive: false`. Those also never send, but they pass validation.
   - Fix: add the DI rule with the same severity. Improve both messages with the fix, e.g. "add lamp N to a
     group's `converters` or add `light-N` to `excludes`".

9. **payload/config.rs:73-89 — `maxControlGears` is fixed at 55 while `daliLampCount` can be 64.**
   - Problem: on a real Core, more gears than `maxControlGears` raises `lw_converterExceed`
     (`LSY_light_system_functions.c:898`). The device config of a 60-lamp demo would then describe a faulty
     installation.
   - Fix: `config_core_fragment(lamp_count)` should write `maxControlGears = max(55, daliLampCount)`. Or add a
     validate rule `daliLampCount ≤ 55` if 55 must stay. Add a test.

10. **sim/generators.rs:88-92, 116-133; validate.rs:381-383 — `randomWalk` resume from an out-of-band manual
    value.**
    - Problem: the doc says to set `last` to the manual value so the walk resumes smoothly. For a value outside
      `center ± spread`, `reflect` folds it into a near-arbitrary point; for example, manual 0 on a −18 ± 1.5
      band becomes ≈ −18 in one tick. After 8 iterations it clamps to the edge. Either way the value jumps
      instead of drifting back.
    - Problem: the band is not checked against the control range. A lamp with `center 90, spread 20` sends
      110 %.
    - Fix: when `last` is outside the band, move by `step` toward the center (mean reversion) and reflect only
      inside. In validate, warn when `center ± spread` (or `center ± amplitude`, or the dailyProfile min/max)
      leaves the slider `min…max`. Add tests for both.

## Low

11. **payload/event.rs:90-92 — the `lw_modbus` text lacks the firmware's singular form.**
    - Problem: the firmware writes ` - register N` when the count is 1 and ` - registers a-b` only when the count
      is > 1 (`MBM_modbus_master.c:756-772`). The range covers the first read bundle in *words*: a `float32` is
      2 words. The builder always writes `registers a-b`, which gives `registers 30001-30001`.
    - Fix: emit ` - register {first}` when `first == last`. Document that the caller passes the first contiguous
      bundle in words. Add a test.

12. **payload/alarm.rs:166-184 — `lw_ntpSyncFailed` is missing from the catalog.**
    - Problem: the firmware has `lw_ntpSyncFailed` ("NTP Sync Failed", MAJOR, `publish.c:708-711`). The plugin
      offers it as a system alarm, but there is no fault for it.
    - Fix: add it to `BUS_ALARMS`, or better to a renamed "system" list. Add a test.

13. **sim/generators.rs:46-53, common.schema.json:149 — the dailyProfile timezone is unspecified.**
    - Problem: the schema says "local time", and `Tick.local_hour` takes a fixed UTC offset from the caller. It
      does not say which timezone (the laptop's, or the device's `lw_configCore.timeZone` "Europe/Berlin").
      Nothing is wired yet, and `chrono` has no tz database.
    - Fix: define it as the device `timeZone`. Add `chrono-tz`, compute the offset per tick (DST-safe), and
      state it in the schema and AGENTS.md.

14. **validate.rs:381-383 — generator and control kinds are not cross-checked.**
    - Problem: `randomWalk`/`sine` on a `switch`/`select`, or on a DI `Value` or motion item, is accepted. A
      `counter` generator on a non-counter control is accepted too.
    - Fix: allow `counter` generators only with `counter` controls (and on impulse counters). Forbid continuous
      generators on switch/select/DI/motion, or require explicit quantization (see #3).

15. **validate.rs:314, 335-343, 437-447; items.schema.json:34,55,93 — firmware length limits are counted in chars,
    not bytes.**
    - Problem: the firmware buffers are 20 / 8 bytes including the NUL. `"Kühlraum Tür links"` is 18 chars but
      20 UTF-8 bytes, and the firmware truncates it.
    - Fix: use `.len()` (UTF-8 bytes) for the slave name, Modbus unit and DI unit. Note that JSON-schema
      `maxLength` counts code points, so keep the schema as is and let validate be the byte gate.

16. **library/schema/items.schema.json:44-60 — schema/validate drift on `unit`.**
    - Problem: `nativeItem` allows `unit` for `dali`/`daliBrightness`/`daliMotion`/`digitalInput`, while
      `validate.rs:300-311` rejects it. ajv passes and cargo fails, so an author gets two different answers.
    - Fix: split `nativeItem` and allow `unit` only on `impulseCounter`, as AGENTS.md:113 says.

17. **model/library.rs:606-628; schema_tests.rs:218-310 — DI entries accept unknown fields in serde.**
    - Problem: `#[serde(flatten)]` disables `deny_unknown_fields`, so `"groupz": [1]` in a demo DI entry or in
      device-read `lw_configDigitalInputs` is silently ignored. AGENTS.md:87 says "unknown fields are errors",
      and `schema_and_serde_reject_the_same_breakage` has no DI typo case.
    - Fix: either inline the settings fields into both structs with `deny_unknown_fields`, or document the
      exception. Add the typo case to the test (expected: rejected by the schema, and by serde after the fix).

18. **validate.rs:1133-1141 — literal alarm refs are not checked against known types.**
    - Problem: `{kind:"type",alarmType:"lw_daliBusPowerFailure"}` passes. The plugin matches exactly, so the
      widget never alarms.
    - Fix: when the literal starts with `lw_`, require it to match the native catalog (bus/system list,
      `lw_(lampFailure|converter|…)_1_NN_*`, `lw_modbus_NN_(timeoutError|configError)`) or a demo `lw_demo_*`
      type. Otherwise report a finding.

19. **payload/floorplan.rs:191-199 + AGENTS.md:139,150-157 — DI label fallback and the export.**
    - The fallback `DI 02` meets D1 ("input number"), but a German presenter UI probably expects `Eingang 2`.
      Decide and document it.
    - Problem: an explicit `title` on a DI widget overrides the config title permanently. After an export it is
      always written, so later DI title edits (D6) never reach the floorplan.
    - Fix: for DI widgets, the export should drop `title` when it equals the config title. Add that to the
      AGENTS.md export steps.

20. **payload/floorplan.rs:305-310 — a missing DALI group is silently replaced by "Group N".**
    - Problem: when the group is missing on the device (groups are edited on the device per D6), the widget is
      still written with the title "Group N", while every other unresolved reference fails the apply.
    - Fix: return `AppError::NotFound("group N")`, or at least report a warning to the caller.

21. **payload/naming.rs:172-212, 230-266 — duplicate keys are not rejected.**
    - Problem: `assign_slaves(&[], &["a","a"])` creates two entries with different ids, and `assign_registers`
      does the same for duplicate item ids.
    - Fix: dedupe, or return `InvalidInput` on a duplicate key or item id. Add tests.

22. **library/series.rs:24-66 (test :94) — the "generate series" naming is surprising.**
    - Problem: the index is `template.index + i` while the suffix is `start + i`, so a template with index 5
      and start 1 gives `light_1 → lamp 5`. The test asserts exactly this.
    - Problem: the ids use `_`, while the PRD and library convention is `light-1`.
    - Fix: use `index = start + i` when the template has an index (or reject a mismatch). Make the separator a
      parameter or `-` for native kinds.

23. **validate.rs:1238-1244 — overlapping ranges are not detected.**
    - Problem: only `min < max` is checked. The plugin requires non-overlapping ranges (`model.ts:5-10`).
    - Fix: sort the ranges by min (null = −∞) and report any range that starts before the previous one ends.

24. **validate.rs (missing) — some ranges are enforced only by the schema.**
    - Problem: widget `decimals` (0-6), `staleAfterMin ≥ 1`, scenario `id` pattern and floorplan `icon`
      non-empty are only in the schema. Files that bypass CI are not covered: export output and `lw_demo`
      custom setup through `validate_custom`.
    - Fix: mirror them in validate, or document them in `schema_enforces_formats_and_ranges` as known gaps.

25. **demo.schema.json:78-98, AGENTS.md:122-128 — authoring redundancy.**
    - Problem: a DI entry needs both `index` and `itemId`, but the index is fully determined by the item and a
      mismatch is an error. `daliLampCount` must also be kept in sync with converters and selected lamps by
      hand.
    - Fix: make `index` optional (derived from the item) and `daliLampCount` optional (default = the highest
      converter or selected lamp index). This makes the format easier for Claude Code to author without
      changing the device output.

26. **AGENTS.md:96-133 — gaps in the "Library format" section.**
    - Missing items:
      - the allowed `datatype` values (`int16|uint16|int32|uint32|float32`);
      - the DI `highLevelAction`/`lowLevelAction` values;
      - which timezone dailyProfile uses;
      - that counter generators are monotonic and start at the default/`initCount`;
      - the writable-holding integer rule (#5);
      - that `{kind:"type"}` should only name real firmware or `lw_demo_*` types.
    - Fix: add one line each.

27. **payload/alarm.rs:449-497, payload/config.rs:535-538 — tests do not pin the whole firmware tables.**
    - Problem: only 5 of the 14 lamp alarms have their text asserted, and only 2 of the 13 `lw_configCore` fields
      have their values asserted. A typo in e.g. `" Load Decrease"` or `daliSchedulerDelayMilliseconds` would
      pass.
    - Fix: assert the full list of `(type, text, severity)` for lamp 5, brightness 2, motion 10 and the bus
      alarms, copied from firmware-core-v2.md §2. Assert `config_core_fragment()` equals the §5.1 JSON
      literally.

28. **library/loader.rs:89-112 — unreferenced floorplan files are never checked.**
    - Problem: a floorplan JSON that no demo references is silently skipped. It is schema-checked, but its
      semantic references are not.
    - Fix: walk `floorplans/<demoId>/*.json` and warn about orphans, or about a `<demoId>` folder that has no
      matching demo.

29. **payload/naming.rs:214-266 — register address convention.**
    - Problem: the 4xxxx/3xxxx-style numbers are stored as *raw* firmware addresses, which are 0-based PDU
      addresses. Consecutive `float32`/`*32` registers also overlap, because each takes 2 words. This is
      harmless for the simulator, which has no bus, and it is consistent across `registers`, `writeRegisters`,
      the plugin `ref` and `lw_modbusWrite`.
    - Problem: a real Core given this config would read the wrong words.
    - Fix: document it in DECISIONS D6 as a deliberate simulator convention. Optionally step `input`/read-only
      `holding` addresses by the register word count.

## Count

High 1 · Medium 9 · Low 19 (29 total).

## Resolution

Status per finding. "Engine" items are listed with exact hookup instructions in `docs/reviews/engine-requests.md`.
Checks after the fixes: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (clean for everything in
the domain scope; the only remaining clippy error is `sim/tests.rs:231`, engine-owned),
`cargo test --lib -- library payload generators` (93 passed), `npm run validate:library` (9 files, 0 invalid).

| # | Status | What |
|---|---|---|
| 1 | fixed (generator side) + handed to engine (seeding) | `GeneratorState::seeded` / `seed_counter` (never lowers) / `counter_value_problem`, `counter_seed(default, snapshot, remote)` = max; counter step is explicitly `max(last)`. Test "resume after restart never decreases". Validation rejects a scenario `value` below a counter's start. Engine: seed at session start, reject manual / scenario values. |
| 2 | fixed | Kept both sources and made them agree: validate rules `item.unit == di.unit` and `control.default == initCount`; new `item_naming_with_di` and the floorplan `measurement_ref` follow the device DI entry (series and unit). Engine: call `item_naming_with_di` at runtime. D6 documents it. |
| 3 | fixed (payload side) + handed to engine (grid wiring) | `payload::measurement::quantize` (+ `CounterGrid`, `item_values_with_grid`), applied inside `item_values`; one unit test per kind. |
| 4 | partly fixed, rest handed to engine | Helpers `sensor_alarms`, `resolve_sensor_alarm`, `sensor_fault_entries`, schema `faultTarget` variant `sensor` (`sensorKind`, `index`, `alarm`). `FaultTarget::Sensor` did not exist yet in `model/session.rs`, so the catalog / `resolve_fault` / `fault_problem` arms are specified in engine-requests (3 small arms). Measurement suppression while lost is optional and engine-side. |
| 5 | fixed | Writable holding defaults to `uint16`; validate rejects `float32` / `*32` datatypes and requires integer slider min/max in 0-65535 and integer step (or integer select values); widget `number` controls need integers. |
| 6 | partly fixed, typed model handed to engine | Schema enum `on/off/auto` (shared `groupMode`), validation rejects anything else. The model stays `Option<String>` because `device/setup.rs` and the frontend share the type; request 5 describes the follow-up. |
| 7 | fixed | Reserved inventory properties (`lw_config*`, `lw_demo`, `lw_floorplan`, `c8y_*`, MO core fields), native event types and native alarm types are rejected in validate and in the schema (`not`). |
| 8 | fixed | Selected DI item without an entry, or `isActive: false`, is a finding; both lamp and DI messages say how to fix it. |
| 9 | fixed (payload side) + handed to engine (changes after create) | `config_core_fragment_for(lamps)`: `maxControlGears = max(55, count)`; `demo_device_body` uses it. Test added. |
| 10 | fixed | Out-of-band `randomWalk` drifts back by `step`; validate flags generator bands (randomWalk, sine, dailyProfile) that leave the slider range, also for scenario generator steps. Tests for both. |
| 11 | fixed | ` - register N` when first == last; doc states the words-bundle meaning; test. |
| 12 | fixed | `lw_ntpSyncFailed` (MAJOR) added to the bus / system list; catalog test updated, validate accepts it as a bus fault. |
| 13 | fixed (library side) + handed to engine (wiring) | `chrono-tz` added, `local_hour_in_zone(unix, tz)` (DST test), schema and AGENTS.md define the device `timeZone` as the zone. `sim/core.rs` still uses the laptop offset (engine request 4). |
| 14 | fixed | `counter` generator only with a counter control or on `impulseCounter`; continuous generators rejected on switch / select / counter controls, DI values, motion (items and scenario steps). |
| 15 | fixed (library side) + handed to engine | Slave name, Modbus unit and DI unit use `str::len` (bytes); DI unit truncation in `digital_inputs_fragment` is byte-safe (`truncate_bytes`, now `pub`). Schema `maxLength` unchanged by design. Engine: `demo_state.rs:321`, `export.rs`. |
| 16 | fixed | `nativeItem` has no `unit`; new `counterItem` allows it only for `impulseCounter`. Schema and cargo agree (test cases added). |
| 17 | fixed | Hand-written strict `Deserialize` for `DigitalInputConfig` / `DigitalInputEntry` (`deny_unknown_fields` without `flatten`); typo cases added to the schema/serde test and a device-entry test. |
| 18 | fixed | Literal `lw_*` alarm types must be a real firmware type (`is_native_alarm_type`: lamp, sensors, temperature sensors, bus / system, Modbus timeout / config) or `lw_demo_*`. |
| 19 | fixed (label + docs) + handed to engine (export) | Decision: German fallback `Eingang <N>` (documented in AGENTS.md). Export step "drop DI widget `title` when equal to the config title" added to AGENTS.md; `device/export.rs` change is engine request 7. |
| 20 | fixed | A missing DALI group now fails with `NotFound("group N")`; test. |
| 21 | fixed | `assign_slaves` / `assign_registers` return `InvalidInput` on duplicate keys / item ids; test. |
| 22 | fixed | `index = n` (so `light-5` is lamp 5), id separator `-` for native kinds, `_` for the rest; tests updated. Doc in `model/device.rs` is engine request 9. |
| 23 | fixed | Overlap detection (sorted by `min`, running end; `min` inclusive, `max` exclusive); tests incl. a range swallowing two others. |
| 24 | fixed | validate mirrors `decimals` 0-6, `staleAfterMin` ≥ 1, scenario id pattern and non-empty icons (widgets and layout). |
| 25 | fixed | DI `index` and `daliLampCount` are optional in schema and model (`#[serde(default)]`, 0 = derive); the loader fills them (item index; highest converter / selected lamp, at least 1) before validation. Tests in loader and schema_tests. |
| 26 | fixed | AGENTS.md "Library format": datatype values and defaults, DI actions, timezone, counter generators, writable-holding rule, `{kind:"type"}` rule, quantization, reserved names, byte limits, optional fields, orphan warning, export title rule, series naming. |
| 27 | fixed | Full lamp (5), brightness (2), motion (10) and bus / system tables pinned as `(type, text, severity)`; `lw_configCore` compared with the §5.1 JSON literally. |
| 28 | fixed | Loader warns about floorplan JSON files no demo references and about `floorplans/<dir>/` without a matching demo. |
| 29 | fixed (documented) | DECISIONS D6 and the `register_base_address` doc state the raw 0-based, non-word-spaced convention. Word-count stepping not done (no bus in the simulator). |

Out-of-scope edits (small, needed for the fixes): `src-tauri/Cargo.toml` (`chrono-tz`), `docs/DECISIONS.md` (D6
note), `docs/reviews/engine-requests.md` (new). `cargo fmt` was run on the whole crate (formatting only).
