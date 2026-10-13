# AGENTS.md — conventions for coding agents

Read `docs/PRD.md` (what) and `docs/ARCHITECTURE.md` (how, incl. the full command/event
contract) before changing code.

## Where things live

| Thing | Location |
|---|---|
| Tauri commands (all) | `src-tauri/src/commands/{app,auth,library,devices,session,raw,activity}.rs` — thin adapters, logic goes in the module |
| Emitted events (all) | `src-tauri/src/events.rs` |
| Types crossing IPC + library file format | `src-tauri/src/model/` |
| The one error type | `src-tauri/src/error.rs` (`AppError`, `{ code, details? }`) |
| Cumulocity HTTP / realtime / keychain / activity log / client event sink | `src-tauri/src/c8y/` |
| Library loading, validation, item selection | `src-tauri/src/library/` |
| Measurement naming + payload builders (pure) | `src-tauri/src/payload/` |
| Session task, outbound worker, generators, operations, faults | `src-tauri/src/sim/` |
| Demo device CRUD, `lw_demo` (+ `lw_demoLock` / `lw_demoSnapshot`, D7), apply, floorplans, export, upgrade | `src-tauri/src/device/` |
| Raw fire | `src-tauri/src/raw.rs` |
| Frontend routes / layout | `src/app/` |
| Feature UI | `src/features/<feature>/` (auth, devices, control, demo = Setup tab, session, raw-fire, activity, settings, updates) |
| Typed backend API | `src/lib/api.ts` (import types and `api`/`events` from here) |
| Generated bindings (do not edit) | `src/lib/bindings.ts` |
| UI primitives | `src/components/ui/` (shadcn/ui style, `radix-ui` package) |
| Translations | `src/i18n/de.json`, `src/i18n/en.json` |
| Design tokens / fonts | `src/styles/tokens.css` (copied, do not edit values), `src/styles/globals.css`, `src/assets/fonts/` |
| Bundled demo library | `library/` |

## Rules

- **All Cumulocity I/O in Rust.** The frontend never calls Cumulocity or `fetch`es external URLs.
- **Contract changes**: add/change a command in the `commands/<area>.rs` file it belongs to and
  list it in that file's `<area>_commands!` macro (end of the file) — **do not** touch `lib.rs` or
  another area's file; events in `events.rs` + `events::collect()`. Then `npm run gen:types` and
  update the tables in `docs/ARCHITECTURE.md`. A new area = a new file plus one line in
  `commands::collect()`.
- **IPC types**: `#[serde(rename_all = "camelCase")]`, `specta::Type`; no `u64/i64/usize`;
  annotate `f64` **fields** with `#[specta(type = specta_typescript::Number)]` (also
  `Option<f64>`, `Vec<f64>`, maps — see ARCHITECTURE §9); **never use a bare `f64` as a command
  argument or return type** — wrap it in a request struct (e.g. `SetItemValueRequest`);
  timestamps are ISO-8601 strings; Cumulocity ids are strings.
- **Errors**: return `AppError`; add a variant only with matching `errors.<code>` keys in
  both i18n files (typecheck enforces it).
- **i18n**: no hard-coded UI strings. Every visible string goes through `t()`; add the key to
  **both** `de.json` and `en.json` (`src/i18n/i18n.test.ts` checks parity; `src/i18next.d.ts`
  makes a mistyped key a type error). Backend errors:
  `t(errorKey(e), errorValues(e))`. German first.
  Technical values (ids, fragment names, units) are not translated.
- **Design system**: use the semantic Tailwind colors (`bg-background`, `bg-card`,
  `bg-primary`, `text-strong`, `text-muted-foreground`, `bg-signal`, status colors) — never
  raw hex or new fonts. One orange (`signal`) element per view. `font-display` (FT Polar
  Light) only for headings ≥ 28 px; everything else is Manrope (`font-sans`), data in
  `font-mono`. No emoji, short declarative copy, numbers with units.
- **Session state** lives in the session task (`sim`); commands talk to it via a cloned
  `SessionSender` (`state.session_sender(&app)`). Never hold the `AppState.session` lock while
  awaiting the task; stop sessions only through `AppState::stop_session` / `stop_session_for`
  (they emit `session-changed(null)`). The state loop never awaits HTTP: it enqueues into
  `sim::outbound`. Don't add live values to `AppState`.
- **Cumulocity client**: `C8yClient` reports through an `EventSink` (activity log entries,
  401 → `auth-expired`); never log headers or passwords. Credentials: password in the keychain
  only, user names in the app config file; only `login_saved` reads the secret.
- **Firmware facts** (alarm texts, units, value ranges, operation payloads) must be checked against
  `docs/research/firmware-core-v2.md` and get a unit test in `payload`.
- **Tests**: pure logic (payload naming, generators, operation handling, export↔apply) gets
  Rust unit tests next to the code. A behavioural fix (Rust or UI) comes with a test that fails
  without it: cargo for the backend (paused-time session tests in `sim/tests.rs`, the fake
  Cumulocity for `device/`), vitest for UI logic. Keep UI decisions in small pure functions or
  hooks (`operationToast`, `sessionErrorText`, `savedGenerators`, `useDeviceConfig`, …) so they can
  be tested without a window.
- **Session-visible state**: anything the UI shows from a running session must reach it through
  the snapshot or an event that carries it (`item-values-changed` has `value` / `manualOverride`
  only; a generator, `paused` or fault change sets `emit_changed`). A setup edit the session
  cannot take over sets `restartNeeded`.
- **Texts**: prompts are worded in the UI from structured data (`OperationPrompt.detail`). Warnings
  built in Rust (apply / upgrade / export) are English technical diagnostics and are shown as they
  are; do not add more prose of that kind to events the presenter reads.
- Don't touch `docs/research/` unless asked; don't commit unless asked.

## Checks (run before handing back)

```sh
npm run typecheck
npm run lint                 # biome check . (npm run format to fix)
npm run test
npm run rust:fmt             # cargo fmt --check
npm run rust:clippy          # cargo clippy --all-targets -- -D warnings
npm run rust:test            # cargo test (also regenerates src/lib/bindings.ts)
npm run validate:library     # JSON-schema check of library/ (ajv)
npm run build
npm run tauri build -- --debug --no-bundle   # also before a release-related change
# or: npm run check  (everything except the two builds)
```

A local `npm run tauri build` needs no signing keys: `createUpdaterArtifacts` is switched on only
by `src-tauri/tauri.release.conf.json`, which the release workflow passes with `--config`. Node 24
(`engines`), Rust 1.88 or newer (`rust-version`; the code uses let-chains).

## Library format

Spec: `library/schema/*.schema.json` (draft 2020-12); serde mirror: `src-tauri/src/model/library.rs`;
worked example: `src-tauri/tests/fixtures/library/`. Check with `npm run validate:library`
(schema) and `npm run rust:test` (schema + semantic rules, `library/validate.rs`). Every file has
`"type"` and `"schemaVersion": 1`; unknown fields are errors (also in demo `digitalInputs[]` and the
device's `lw_demo.digitalInputs`). Firmware length limits count UTF-8 **bytes** (`"Kühlraum Tür
links"` is 20 bytes): validation is the byte gate, the schema's `maxLength` counts characters.

```
library/items/<topic>.json                  lw-demo-items   {slaves?, items}
library/demos/<demoId>.json                 lw-demo
library/floorplans/<demoId>/<name>.json     lw-demo-floorplan (+ <image>.png|jpg|svg next to it)
```

**Items** (one item = one thing; reused across demos by tags). Common: `id` (globally unique,
`[A-Za-z0-9_-]` ≤ 19, = Modbus register name), `kind`, `title {de, en}`, `tags`. Optional
`control` (`slider {min,max,step,default}` | `switch {default 0|1}` | `select {options:[{value,
label{de,en}}],default}` | `counter {default}`), `generator` (`randomWalk {center,spread,step?}` |
`sine {period s,amplitude,center?}` | `dailyProfile {points: 24}` | `counter {ratePerHour}`),
`effects` (`event {type,text}` with `{value}`/`{label}`, `inventory {property}`, `alarm {type,
severity,text,threshold}`).
- `dailyProfile` hours (00:00…23:00, linearly interpolated) are in the **device's time zone**
  (`lw_demo.timeZone`, default `Europe/Berlin`, DST-aware), not the laptop's.
- `counter` generators are monotonic: allowed only with a `counter` control or on an
  `impulseCounter`; the start is the control default (= DI `initCount`), at session start the
  newest stored value wins, and manual / scenario values below the current value are rejected.
  `randomWalk` / `sine` / `dailyProfile` are not allowed on switch / select / counter controls,
  DI values or motion; their band (`center ± spread` / `± amplitude` / profile min-max) must stay
  inside the slider range. A `randomWalk` resumed outside its band drifts back by `step` per tick.
- Values are quantized to what the firmware sends: integers for DALI brightness / runtime, lux,
  motion, DI value (0/1), `rssi` and counters (`initCount + k·increment`), `T` to 0.1; Modbus raw.
- `effects` must not touch reserved names: inventory `lw_config*`, `lw_demo`, `lw_floorplan`,
  `c8y_*`, `name`/`type`/`id`/`owner`/`description`; events `lw_daliGroupChange`, `lw_alarm`,
  `lw_modbus`; native alarm types. Use a `lw_demo*` prefix for properties / events / alarm types.

Measurements are named from the app's config, not after the firmware (DECISIONS D10): **fragment =
the item's German title** (`.` / `$` → `_`, trimmed; a DI's device title wins), **series = the item
id**, e.g. `Attika 1.light-5`. The simulator writes and reads **no `lw_config*` fragment**: groups,
DI config and the time zone live in `lw_demo`, apply / upgrade remove old `lw_config*` fragments.

| kind | own fields | Cumulocity (type / `fragment.series`) |
|---|---|---|
| `dali` | `index` 1–64 | `daliControlGear` / `<title>.<id>` (brightness) + `<title>.<id> Lamp Status` / `<id> Converter Status` / `<id> Converter Runtime`; a grouped lamp without item: `DALI Leuchte NN.lamp-NN` |
| `daliBrightness` / `daliMotion` | `index` 1–10 | `daliBrightness` / `daliMotion`, `<title>.<id>` |
| `digitalInput` / `impulseCounter` | `index` 1–10 (counter: `unit` ≤ 19 bytes, must equal the demo DI entry's `unit`; `control.default` must equal its `initCount`) | `digitalInput` / `impulseCounter` (by the device DI type), `<DI title>.<id>` |
| `system` | `system: rssi \| temperature` | `c8y_SignalStrength` / `c8y_TemperatureMeasurement`, `<title>.<id>` |
| `modbus` | `slave` (key), `registerType?` (`coil`\|`holding`\|`input`; default `holding`, `coil` for a switch), `unit?` ≤ 7 bytes, `datatype?` (`int16`\|`uint16`\|`int32`\|`uint32`\|`float32`; default `uint16` for switch / select / writable holding, else `float32`), `writable` (coil/holding only; a writable **holding** register is one 16-bit word: no `float32` / `*32`, control with integer min/max in 0–65535 and integer step, or integer select values) | `modbus`, `<title>.<id>` |
| `customAlarm` | `severity`, `text`; id must not start with `NN_` | alarm `lw_demo_<id>` |

Native kinds take no `unit` (fixed by the firmware). Units are German/metric; US variants are
separate items. Tags: client tags (`gas-station`, `retail`, `restaurant`), `category:*`,
`zone:*`, all `[a-z0-9-]`.

**Slaves** (in item files): `{key, name ≤ 19 bytes, requestInterval s, alarms: [{name, text,
severity}]}`; slave alarm type `lw_demo_<id:02>_<name>`. Ids / addresses (20/240 counting down)
and register addresses (coil 1…, input 30001…, holding 40001…) are assigned per device, never in
the library. Register addresses are a simulator convention (raw 0-based, 32-bit registers are not
spaced by two words), see DECISIONS D6.

**Demo**: `id` (`[a-z0-9-]`), `name`/`description {de,en}`, `tags`, `includes`/`excludes` (item
ids; selection = tags ∪ includes − excludes), `daliLampCount` 1–64 (selected DALI items ≤ it and
in a group), `daliGroups[]` ≤ 16 `{id 1–16, name, converters:[lamps], minLevel, maxLevel 0–100,
luxThreshold?, slaveOf?, dynamicControl?, automaticSwitch?}`, `digitalInputs[]` `{index?, itemId,
type: measurement | daliAction (+highLevelAction/lowLevelAction/groups) | impulseCounter
(+initCount/increment/unit), isActive?}` (item = DI item with the same index; `index` may be
omitted and is taken from the item; its German title becomes `lw_demo.digitalInputs[].title`;
actions are exactly `on` | `off` | `auto`, lowercase; every selected DI item needs an active
entry, like every selected lamp needs a group), `daliLampCount` may be omitted (default: highest
converter / selected lamp index), `sendIntervalSec` 5–899, `scenarios[]` `{id, name,
start? (≤ 1), set: [{kind:"value",itemId,value} | {kind:"generator",itemId,generator} |
{kind:"fault",target,active}]}` (`active: true` fires the alarm, `false` clears it in Cumulocity; DECISIONS D9) with targets `daliLamp {lamp,alarmType}`, `daliBus {alarmType}`,
`slaveTimeout {slaveKey}`, `slaveAlarm {slaveKey,alarmName}`, `customAlarm {itemId}`;
`floorplans: ["floorplans/<demoId>/<name>.json"]` (a layout no demo references, or a folder
without a demo, is reported as a load warning). Max 20 slaves / 40 registers per slave among the
selected items.

**Floorplan layout** = plugin `lw_floorplan` with item references: `{name, description?, icon?,
image, imageWidth, imageHeight (px), imageOpacity?, widgets[]}`. Widgets `{id, kind, x, y, locked?,
icon}` plus:
- `sensor`: `measurement {itemId, series?}` (series only for DALI lamps: `Brightness` (default) /
  `Lamp Status` / `Converter Status` / `Converter Runtime`, sent as `<id>` / `<id> Lamp Status` …),
  `title?` (default item title de / DI device title, fallback `Eingang <N>`), `unit?`,
  `decimals?` (0–6), `display?`, `ranges[]` (must not overlap; `min` inclusive, `max` exclusive),
  `autoAlarms` (default true: the item's own alarms), `alarms[]`, `staleAfterMin?` (≥ 1);
- `alarm`: `text`, `note?`, `alarms[]` (≥ 1);
- `control`: `title?`, `confirm?`, `control` = `{type:"dali", group, modes}` or `{type:"modbus",
  itemId (writable), readback? (default true), valueType: boolean (onLabel, offLabel, invert?) |
  number (integer min, max, step; 0–65535, unit?) | multi (options [{value,label,icon}], holding
  only)}`; a DALI control whose `group` is missing on the device fails the apply. DALI / Modbus
  controls do not work in the plugin until it reads `lw_demo` instead of `lw_config*` (D10).

Alarm refs: `{kind:"item",itemId}` (DALI lamp alarms / slave timeout + all slave alarms /
`lw_demo_<id>`), `{kind:"slaveTimeout",slaveKey}`, `{kind:"slaveAlarm",slaveKey,alarmName}`,
`{kind:"type",alarmType}` (literal; `lw_*` types must be real firmware types, e.g.
`lw_daliBusPowerFail`, `lw_ntpSyncFailed`, or `lw_demo_*`: the plugin matches exactly).
Brightness / motion sensor widgets watch (and the presenter can raise) the sensor's own alarms.
"Generate series" names native kinds `<id>-<n>` with `index` = `n` (`light-5` is lamp 5), others
`<id>_<n>`.

**Exporting a device by hand** (PRD §13; the advisory lock and the saved snapshot are the own
fragments `lw_demoLock` / `lw_demoSnapshot`, not part of `lw_demo` (DECISIONS D7), and not exported):
`lw_demo.tags/includes/excludes/daliLampCount/
sendIntervalSec` → demo; `lw_demo.daliGroups` → `daliGroups` (`slaveOf` absent → omit);
`lw_demo.digitalInputs` → `digitalInputs` (`title` → the DI item's `title.de`); a device without
them (older app version) uses the library demo's; `lw_demo.customSlaves/customItems` → an
item file; `lw_demo.generators` → item `generator`s or a `start` scenario; widgets: replace
`measurement {fragment, series}` by `{itemId}` (the series is the item id, D10; a DALI
`<id> Lamp Status` etc. becomes `series: "Lamp Status"`), Modbus `target`/`readback` by `itemId`,
`alarmTypes` by alarm refs (drop the ones `autoAlarms` adds); drop `deviceId`; drop a DI
widget's `title` when it equals the DI device title (otherwise later title edits never reach the
floorplan). Widgets that still use the old firmware names (`LW Dali Control Gear 1-NN`, …) cannot
be mapped: re-apply the demo first.

## Shipped demos

| Demo id | Lamps / groups | Digital inputs | Selected items | Scenarios | Floorplan |
|---|---|---|---|---|---|
| `gas-station` | 17 of 22 / 6 | 9 | 79 | normal (start, full reset), night, cooling-fault, comm-loss, peak-load, lamp-fault, water-damage, tank-fault | `floorplans/gas-station/ground-floor.json` |
| `retail` | 13 of 22 / 5 | 9 | 72 | normal (start, full reset), night, cooling-fault, comm-loss, peak-load, lamp-fault, water-damage | `floorplans/retail/ground-floor.json` |
| `restaurant` | 10 / 4 | 7 | 75 | normal (start, full reset), night, kitchen-fault, cooling-fault, comm-loss, lunch-peak, lamp-fault, water-damage | `floorplans/restaurant/ground-floor.json` |

Library content rules:
- **The library is the source of truth** for item titles (`library/items/*.json`, e.g. the digital input titles). `docs/research/prototype-inventory.md` is a snapshot of the old prototype and may word some titles differently.
- Sensors: the library ships one `daliBrightness` item (`lux-1`) and no `daliMotion` item; the code, fault menu and floorplan support both kinds, add a motion item when a demo needs one.
- **Slave names are ASCII** (`Kuehlung`, `Ladesaeule 1`, `Klima Kueche`; they appear in the timeout alarm text and a real Core may read the config); titles and alarm texts use umlauts.
- The `normal` (start) scenario is a **full reset**: it sets every value another scenario sets (a generator step for items with a library generator, a value step otherwise) and clears every fault another scenario raises. Applied by hand it first clears every open alarm of the device ("Clear all", D9); at session start only its steps run, so the explicit `active: false` steps are what resets restored faults.
- A fault scenario must not hide its own effect: never time out the slave whose value it shows (communication loss is its own `comm-loss` scenario).
- Floorplans: widgets are fixed screen size and do not scale with the image; lay them out in room "cells" (circle 130 x 140, control 260 x 56 image px), room labels top-left, widgets below the label.

Item files in `library/items/`: `dali-lights`, `dali-sensors`, `digital-inputs`, `system`, `cooling`, `hvac`,
`energy`, `ev-charging`, `fuel` (gas station only), `custom-alarms`. Lamps 15–18 (gas-station canopy) and 19–22 (sales floor) are tagged; 23–64 carry no client tag (free for custom demos).
