# PRD — Lichtwart Demo Simulator (Tauri 2)

Status: agreed design, ready for implementation · 2026-10-12
Replaces the prototype `../davaco-demo-simulator` (browser page + Node proxy).

## 1. Problem

Sales demos need a Cumulocity device that behaves like a real Lichtwart Core v2 — DALI lights,
digital inputs, Modbus slaves, alarms, floorplan controls — without touching real hardware. The
prototype was built for one client (davaco), hard-codes one device and 135 items, uses non-native
measurement names, and has to be started from a terminal.

## 2. Goal

An installable desktop app (macOS + Windows, Lichtwart-internal) that:

1. logs into a Lichtwart tenant,
2. creates or reuses **demo devices** that look exactly like a Core v2 to Cumulocity and to the
   floorplan plugin (`../davaco-demo`),
3. applies a **demo pack** (gas station, retail, …) from a shared, versioned library — including
   Modbus slaves, DALI groups and a ready-made floorplan,
4. lets the presenter drive values, scenarios and faults live, and answers floorplan commands,
5. fires raw measurements / alarms / events on the fly,
6. exports a hand-tuned demo device back into the library format so it can be released to everyone.

Non-goals (v1): floorplan editor (done in Cumulocity), running simulations without the app open
(no microservice), Linux, TFA/SSO login, analog inputs (firmware naming not defined yet), UI e2e
tests.

## 3. Users

Lichtwart staff only. Mostly one presenter per demo device; concurrent use is rare.

## 4. Tech stack & conventions

| Area | Choice |
|---|---|
| Shell | Tauri 2 |
| Frontend | React + TypeScript, Tailwind, shadcn/ui themed with Lichtwart tokens (`../Lichtwart Design System-handoff/lichtwart-design-system/project/tokens.css`, FT Polar fonts) |
| i18n | react-i18next, German + English from day one, no hard-coded UI strings |
| Lint/format | Biome (frontend), rustfmt + clippy (Rust) |
| Package manager | npm |
| Cumulocity I/O | **All HTTP + realtime from Rust** (reqwest). No CORS, no "Allowed domain" changes on the tenant, no proxy. React calls Tauri commands and listens to Tauri events. |
| Secrets | OS keychain (macOS Keychain / Windows Credential Manager) via a Tauri keyring plugin |
| Updates | Tauri updater against GitHub Releases of this (public) repo |

## 5. Login & tenants

- **No tenant ships with the app** (the repo is public): the user enters the tenant **URL** and
  **tenant ID**. `https://` is added when missing; a path (copied browser URL) is dropped; only
  `https` is accepted.
- Login: username + password, **Basic auth** (`Authorization: Basic …`, `UseXBasic: true`). The username
  is always sent as `tenantId/user` (works on every domain). Validate with `GET /user/currentUser`.
- "Remember me" (optional) stores the password in the OS keychain per tenant (key
  `<tenantId>@<host>`) and tenant ID, URL and user name in the app config; otherwise held in memory
  only. The remembered logins are the tenant choice on the login screen.
- Switching tenant = logout + login.

## 6. Demo devices

### 6.1 Identity

Demo devices are separate managed objects — the app never simulates on real devices.

```json
{
  "name": "Tankstelle Musterstadt",
  "description": "Free text for further info",
  "type": "LW_Core_v2",
  "c8y_IsDevice": {},
  "com_cumulocity_model_Agent": {},
  "c8y_DeviceTypes": ["demo"],
  "c8y_Position": { "lat": 48.137, "lng": 11.575, "alt": 0 },
  "c8y_Hardware": { "model": "Lichtwart Core v2", "serialNumber": "DEMO-<random>", "revision": "demo" },
  "c8y_Firmware": { "name": "SIMULATOR", "version": "V 03.00.02" },
  "c8y_SupportedOperations": [
    "c8y_Restart", "c8y_Firmware", "c8y_MeasurementRequestOperation", "lw_setGroupColor",
    "lw_fetchCore", "lw_fetchGroups", "lw_fetchMotion", "lw_fetchBrightness",
    "lw_fetchDigitalInputs", "lw_fetchModbusSlaves", "lw_fetchAlarmBlacklist",
    "lw_fetchTemperatures", "lw_fetchRs485", "lw_switchGroupMode", "lw_switchConverterState",
    "lw_daliReset", "lw_modbusWrite", "lw_daliVendorScan", "lw_daliScan"
  ],
  "lw_configCore": {
    "maxControlGears": 55, "measurementIntervalMinutes": 10, "digitalInputsMsgPerMinute": 10,
    "errorManagerSetThresholdCounts": 3, "errorManagerClearThresholdCounts": 2,
    "cloudWatchdogRestartTimeoutMinutes": 120, "isDaliScanActive": false,
    "isDaliScanForNewDevicesActive": true, "daliSchedulerIntervalSeconds": 60,
    "daliSchedulerDelayMilliseconds": 500, "daliSchedulerErrorIntervalSeconds": 60,
    "timeZone": "Europe/Berlin", "ntpServer": ""
  },
  "lw_configRs485": { "isActive": true, "baudrate": 9600, "parity": "none", "stopBit": 1, "dataBits": 8 },
  "lw_configGroups": [],
  "lw_configDigitalInputs": [],
  "lw_configModbusSlaves": [],
  "lw_demo": {}
}
```

- `maxControlGears` is `max(55, daliLampCount)`, so a device with more than 55 lamps stays valid (changed 2026-10-12, see DECISIONS.md D5); the 55 above is the default.
- **`com_cumulocity_model_Agent`**: the device is its own agent. Cumulocity addresses every
  operation to an agent and rejects operations for a device without one (`422 Agent for device
  '<id>' does not exist in inventory!`). Devices created before this fragment existed are
  migrated when they are opened, a demo is applied or a session starts: a PUT adds the agent
  fragment and any missing `c8y_SupportedOperations` entry, only when something is missing
  (changed 2026-10-12).
- **No `c8y_RequiredAvailability`**, so the device never turns "unavailable".
- `lw_configCore` is always written as the **full 13-field object** (changed 2026-10-12, see DECISIONS.md): the firmware fails `lw_fetchCore` if any field except `timeZone`/`ntpServer` is missing, so a partial object is not valid.
- The firmware version stays `"V 03.00.02"` (changed 2026-10-12, see DECISIONS.md): the real firmware sends the leading `V `; it must remain parseable as `[V ]NN.NN.NN`.
- `c8y_Hardware.model` makes the floorplan tab appear; firmware ≥ 01.06.00 enables Modbus
  controls in the floorplan plugin.

### 6.2 Device list

- Shows **only demo devices** (`c8y_DeviceTypes` contains `demo`), grouped by their parent
  `c8y_DeviceGroup`. Full-text search on name/description.
- Actions:
  - **Create**: name, description, map position (lat/lng fields + map picker), optional parent group, optional
    demo pack to apply immediately.
  - **Reuse**: open an existing demo device.
  - **Edit** name, description, position.
  - **Reset**: stop the session, clear ACTIVE/ACKNOWLEDGED alarms, optionally delete floorplans the app
    created.
  - **Delete**, after confirmation. This also removes app-created floorplans and their binaries.

### 6.3 State on the device — `lw_demo`

All setup lives on the device, so any laptop restores it.

```json
"lw_demo": {
  "schemaVersion": 1,
  "demoId": "gas-station",
  "libraryVersion": "1.4.0",
  "tags": ["gas-station"],
  "includes": ["light-12"],
  "excludes": ["ev_2_power"],
  "daliLampCount": 24,
  "slaves": [{ "key": "kuehlung", "id": 20, "address": 240,
               "registers": [{ "itemId": "freezer_temp", "registerType": "input", "address": 30001 }] }],
  "customSlaves": [],
  "customItems": [],
  "generators": { "freezer_temp": { "kind": "randomWalk", "center": -18, "spread": 1.5 } },
  "sendIntervalSec": 30,
  "realisticMode": false,
  "floorplanIds": ["123456"]
}
"lw_demoSnapshot": { "savedAt": "…", "values": { "freezer_temp": -18.2 } },
"lw_demoLock": { "user": "sl", "since": "2026-10-12T12:00:00Z" }
```

- The snapshot and the lock are **own top-level fragments** next to `lw_demo`, not fields inside it (changed 2026-10-12, see DECISIONS.md D7): Cumulocity replaces a fragment as a whole, so a lock refresh must not rewrite the setup.
- `slaves[].registers` persist the assigned register addresses, `customSlaves` / `customItems` hold slaves and items created in the demo editor on this device (changed 2026-10-12, see DECISIONS.md D6).
- Setup is written when it changes. Live values are written only on **"Save snapshot"**.
- The lock is advisory only. It shows "in use by X since …" and is ignored after 10 min. There is no hard
  locking.

## 7. Native Core v2 simulation

Every demo device simulates the standard Core v2 set exactly as the firmware sends it
(`../firmware-core-v2`).

### 7.1 System

- `c8y_SignalStrength.rssi` (dBm)
- `c8y_TemperatureMeasurement.T` (unit `"C"` in the payload (changed 2026-10-12, see DECISIONS.md))

### 7.2 DALI (1 bus, up to 64 lamps)

- **Measurement** type `daliControlGear`, fragment `LW Dali Control Gear 1-NN` (NN = 01–64). Series:

  | Series | Unit |
  |---|---|
  | `Brightness` | `%` |
  | `Lamp Status` | — |
  | `Converter Status` | — |
  | `Converter Runtime` | `hr` |

  At most 6 lamps per message.
- **Lamps per demo**: the pack sets `daliLampCount` (1–64). Tags decide which lamps are visible in the
  control panel.
- **Groups**: 16 groups in `lw_configGroups`, taken from the pack.
- **`lw_switchGroupMode` on/off/auto**:
  1. Sets the group's lamps to their max level, to 0, or to the generator value.
  2. Sends event `lw_daliGroupChange` with `groupModeChange {id, previousMode, newMode, reason: "cloud-operation"}`.
- **`lw_switchConverterState`** switches a single lamp (identify), as on the real device.
- **Fault menu per lamp**: native alarm types only; each entry fires its alarm once, and the effect lasts until the
  alarm is cleared in Cumulocity (changed 2026-10-12, see DECISIONS.md D9), e.g.
  - `lw_lampFailure_1_NN_openCircuit`
  - `lw_converter_1_NN_lost`
  - `lw_controlGearFailure_1_NN`
  - bus-level alarms `lw_daliBusPower*`: like the firmware, every grouped lamp is lost too (`lw_converter_1_NN_lost` + `lw_controlGearFailure_1_NN`, no measurements) and every brightness / motion sensor (`_lost`); bus recovery clears them (DECISIONS D8)

  Text and severity match the firmware.
- **Optional sensors**: `LW Dali Brightness.LuxN` and `LW Motion.StateN`, as items.

### 7.3 Digital inputs (10) (changed 2026-10-12, see DECISIONS.md)

- Type `digitalInput`, fragment `LW Digital Input`, **native series names** exactly as the firmware sends them:
  - `Value<NN>` (0/1), NN = 01-10
  - `ImpulseCounter<NN>` (counter; unit from `lw_configDigitalInputs[].unit`)
- The human title (e.g. "Zapfsäule 1") is **not** part of the series name. It is stored in
  `lw_configDigitalInputs[].title` (extra field, ignored by the firmware) and in the library.
- The app shows the title as the selectable label. When a floorplan is applied, the widget label uses the title from
  the config, falling back to the input number if there is no title.
- `lw_configDigitalInputs` is written to match (`id`, `isActive`, `type`, `title`, and `unit`/`initCount`/`increment` for counters).

### 7.4 Modbus (everything that is not DALI / DI / system)

- **Slaves**:
  - A slave is one functional group, e.g. "Kuehlung", "Klima Zone 1", "Ladesaeule 1". It is defined in the library and
    can be created in the app's demo editor.
  - Assigned **IDs count down from 20, addresses from 240** (max 20 slaves, 40 registers per slave).
  - Slave name ≤ 19 chars. Register name = item id, ≤ 19 chars. Unit ≤ 7 chars.
  - Written to `lw_configModbusSlaves` with `isSavingOnStorage: false` and `description: "lw-demo"`.
  - Writable items go into **both** `registers[]` (readback series) **and** `writeRegisters[]` (the floorplan's "saved target" picker) (changed 2026-10-12, see DECISIONS.md).
- **Measurements**: type `modbus`, fragment `LW_Modbus_Slave_<id:02>_<address:03>`, series = register
  name.
- **Custom alarms** (non-DALI) (changed 2026-10-12, see DECISIONS.md):
  - A slave can carry **several** custom alarms (`alarms[]`): type `lw_demo_<id:02>_<name>`, with text and severity
    from the pack. Example: `lw_demo_19_co2Leakage`.
  - **Device-level custom alarms** (item kind `customAlarm`) are not bound to a register or slave: type
    `lw_demo_<name>`. `name` must not start with two digits plus `_`.
- **Slave timeout fault**:
  1. Stops that slave's measurements.
  2. Raises `lw_modbus_<id:02>_timeoutError` with text `Modbus <id>-<addr> Timeout Error <name>`, MAJOR.
  3. Sends event `lw_modbus`.

  **Recover** (when the alarm is cleared in Cumulocity, by the presenter or "Clear all") (changed 2026-10-12, see DECISIONS.md D9):
  1. Sends event `lw_alarm` "Alarm cleared: …".
  2. Resumes the measurements.
- **`lw_modbusWrite`**: after confirmation (§9), updates the register value and sends a readback
  measurement that echoes **exactly the operation's `value`** (changed 2026-10-12, see DECISIONS.md) (the plugin compares raw values with `===` and ignores `invert`).

### 7.5 Alarm clearing (changed 2026-10-12, see DECISIONS.md D9)

The real firmware never clears cloud alarms; it only sends `lw_alarm` "Alarm cleared: <type>".

- **Faults are fired once** (a button per fault in the alarms panel and in the lamp / sensor menus; firing an open
  one again re-posts it) and stay in effect until their alarm is **cleared in Cumulocity**: by the presenter in the
  Cumulocity UI, or with **Clear all** in the alarms panel, which clears every open alarm of the device (also ones
  the simulator did not raise). Acknowledging only changes the status the app shows (*Aktiv* / *Quittiert*). When
  the effect ends the app sends the "Alarm cleared" event. The session follows clears / acknowledgements over
  realtime and re-syncs after a reconnect.
- Alarms the simulator recovers itself (the DALI bus cascade, item threshold alarms): the app sends the event
  **and** clears the alarm (`PUT /alarm/alarms/{id}` → `CLEARED`). Per-demo **realistic mode** skips that clear.

## 8. Simulation engine (Rust)

- **One active session at a time.** Opening another device while running asks "stop and switch?".
- The session runs in the Rust backend, so it keeps going when the window is hidden. It stops when the app quits.
- Sending:
  - immediately on value change
  - plus a periodic send of all values, every `sendIntervalSec`, adjustable as in davaco; **max 899 s** (changed 2026-10-12, see DECISIONS.md) (the floorplan plugin marks values stale after `measurementIntervalMinutes` + 5 min = 900 s)
  - every measurement, alarm and event carries `time` = now (ISO 8601) (changed 2026-10-12, see DECISIONS.md); the plugin shows the newest by `time`. Values are JSON numbers; series with an empty firmware unit omit `unit`
  - native fragments are batched into one `measurementCollection` where possible
- **Controls per item**:

  | Control | Use |
  |---|---|
  | slider | number |
  | switch | 0/1 |
  | select | enumeration, e.g. AC mode |
  | counter | monotonic, e.g. energy totals |

  An optional **generator** can run on top:

  | Generator | Settings |
  |---|---|
  | `randomWalk` | center, spread, step |
  | `sine` | period, amplitude |
  | `dailyProfile` | 24 hourly points |
  | `counter` | rate per hour |

  Manual movement overrides the generator until "resume generator".
- **Multi-effect items**: an item can emit several outputs (measurement, event, inventory property,
  alarm), e.g. AC mode = measurement code + event + device property.
- **Scenarios**: named presets in the pack. Each is a list of `{itemId, value | generator | fault}`;
  one can be marked `start`. Applying one is a single click. A fault step fires the alarm (`active: true`) or clears
  it in Cumulocity (`active: false`); applying the start scenario by hand first clears all open alarms of the device
  (changed 2026-10-12, see DECISIONS.md D9).

## 9. Operations (commands from Cumulocity / floorplan)

- Subscribe to operations for the device via Cumulocity realtime notifications, from Rust.
- **Config operations are auto-answered SUCCESSFUL**, with no popup: `lw_fetch*`, `c8y_Restart`,
  `c8y_MeasurementRequestOperation` (this one also triggers an immediate send of all values), `c8y_Firmware`,
  `lw_daliScan`, `lw_daliVendorScan`, `lw_daliReset`, `lw_setGroupColor`.
- **Visible operations** (`lw_switchGroupMode`, `lw_switchConverterState`, `lw_modbusWrite`):
  1. Set EXECUTING immediately.
  2. Show an in-app notification: "Confirm / Abort", with the operation summary and `by` user.
  3. Confirm → apply the effect → SUCCESSFUL. Abort → FAILED with a reason text.
  4. With no answer within 60 s → FAILED "Timed out".
  5. An optional **auto-accept** switch per session, off by default.
- **Operations that already have status FAILED are ignored** (changed 2026-10-12, see DECISIONS.md): the floorplan plugin cancels a PENDING op after 10 s and supersedes older group-mode ops, so a late realtime event must not be applied. Once an op is EXECUTING the plugin has no timeout, so the 60 s above is what ends it.
- **On session start**, any PENDING operation older than 60 s → FAILED "Simulator was offline".

## 10. Raw fire panel

Usable with or without a demo pack, on the selected demo device:

- **Measurement**: type, fragment, series, value, unit.
- **Alarm**: type, severity, text.
- **Event**: type, text, extra JSON.
- A raw-JSON tab to POST any body to measurement/alarm/event.
- A recent-history list of fired items; click one to resend it.

## 11. Demo library

### 11.1 Location & versioning

- Lives in **this repo** under `library/`, is validated in CI, and is **bundled into each app release**. New demos
  arrive as an app update. There is no runtime fetch from GitHub.
- App version = library version (SemVer, CHANGELOG.md).
- Each file has `"type"` and `"schemaVersion"`. Invalid files fail CI.

### 11.2 Layout

```
library/
  items/            # reusable item libraries — "type": "lw-demo-items"
    dali-lights.json      # light-1 … light-64 (native DALI), tagged per demo
    digital-inputs.json
    system.json
    custom-alarms.json    # device-level `customAlarm` items (changed 2026-10-12, see DECISIONS.md)
    cooling.json          # Modbus slaves + their registers + per-slave alarms[]
    ev-charging.json
  demos/            # "type": "lw-demo"
    gas-station.json
    retail.json
    restaurant.json       # (added 2026-10-12, see DECISIONS.md) the davaco kitchen/dining demo
  floorplans/<demoId>/
    <name>.json     # floorplan layout, widgets reference item ids
    <name>.png|jpg|svg
```

### 11.3 Item (sketch)

```json
{
  "id": "freezer_temp",
  "kind": "modbus",
  "slave": "kuehlung",
  "title": { "de": "Tiefkühler Temperatur", "en": "Freezer temperature" },
  "tags": ["retail", "gas-station", "category:cooling", "zone:shop"],
  "unit": "°C",
  "datatype": "float32",
  "writable": false,
  "control": { "kind": "slider", "min": -30, "max": 0, "step": 0.5, "default": -18 },
  "generator": { "kind": "randomWalk", "center": -18, "spread": 1 }
}
```

- `kind`: `dali`, `daliBrightness`, `daliMotion`, `digitalInput`, `impulseCounter`, `system`, `modbus`, `customAlarm` (changed 2026-10-12, see DECISIONS.md).
  Native kinds carry `index` instead of `slave`; a `customAlarm` carries neither (device-level), only `title`, `severity` and `text`; a `digitalInput` also carries its `title` for `lw_configDigitalInputs[].title`.
- Item ids are globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars (they become Modbus register names).
- **One item = one thing** (light-1 … light-64 exist once and are tagged into many demos). The editor
  has a "generate series" helper.
- **Units: German/metric by default.** US variants are separate items (e.g. `freezer_temp_f`).

Modbus items carry `registerType` (`coil` | `holding` | `input`; default `holding`, `coil` for a switch control) (changed 2026-10-12, see DECISIONS.md D6). Slave definition (changed 2026-10-12, see DECISIONS.md): `{ "key", "name" (≤19), "requestInterval", "alarms": [ { "name", "text", "severity" } ] }` (any number of alarms, type `lw_demo_<id:02>_<name>`).

### 11.4 Demo (sketch)

```json
{
  "type": "lw-demo",
  "schemaVersion": 1,
  "id": "gas-station",
  "name": { "de": "Tankstelle", "en": "Gas station" },
  "description": { "de": "…", "en": "…" },
  "tags": ["gas-station"],
  "includes": [],
  "excludes": [],
  "daliLampCount": 24,
  "daliGroups": [{ "id": 1, "name": "Shop", "converters": [1, 2, 3], "minLevel": 0, "maxLevel": 100 }],
  "digitalInputs": [{ "index": 3, "itemId": "di-3", "type": "measurement" }],
  "sendIntervalSec": 30,
  "scenarios": [{ "id": "night", "name": { "de": "Nacht", "en": "Night" }, "start": false, "set": [] }],
  "floorplans": ["floorplans/gas-station/ground-floor.json"]
}
```

A digital input references its item by `itemId`; the item's German title is written to `lw_configDigitalInputs[].title`. `type` is `daliAction` | `impulseCounter` | `measurement`, plus `isActive`, `initCount`, `increment`, `unit`, `highLevelAction`, `lowLevelAction`, `groups` as the firmware config defines them (changed 2026-10-12, see DECISIONS.md D1, D6).

Demo selection = items matching `tags` ∪ `includes` − `excludes` (8c).

### 11.5 Tags, filters, search

- One tag system with namespaces: plain client tags (`retail`, `gas-station`, …), `category:*`
  (`category:ev-charging`, `category:tank`, …) and `zone:*`.
- The client tags shipped as demos are `gas-station`, `retail` and `restaurant` (changed 2026-10-12, see DECISIONS.md).
- Filter chips are built from the namespaces.
- Full-text search covers all items: title (de+en), id, tags, slave name. Multiple words are combined with AND, as in davaco.

## 12. Floorplans

- Editing happens **only in Cumulocity** (davaco floorplan plugin). There is no editor in the app.
- **Apply** (part of applying a demo, optional):
  1. `POST /inventory/binaries` uploads the image.
  2. `POST /inventory/managedObjects` creates `{type: "lw_floorplan", name, lw_floorplan: {imageBinaryId, imageWidth, imageHeight, widgets}}`.
  3. `POST /inventory/managedObjects/{device}/childAdditions` links it to the device.

  Widget item references are resolved at apply time:
  - The `measurement {fragment, series}` comes from the resolved native or Modbus naming.
  - `alarmTypes` gets the slave's timeout alarm, **all** its custom alarms (`lw_demo_<id:02>_<name>`), referenced device-level custom alarms (`lw_demo_<name>`) and the DALI lamp alarms filled in automatically (changed 2026-10-12, see DECISIONS.md). Custom types are written literally (the plugin never suggests them).
  - A digital-input widget is an ordinary sensor widget with `{fragment: "LW Digital Input", series: "Value<NN>"}`; its label is the title from `lw_configDigitalInputs[].title`, falling back to the input number (changed 2026-10-12, see DECISIONS.md).
  - Modbus control targets come from the assigned slave address.
- **Re-applying** the same demo replaces an existing floorplan of the same name, after confirmation.
- A floorplan is never required: raw simulation works without one.

## 13. Export device → demo pack

- **"Export as demo pack"** reads a demo device and writes a folder in the `library/` layout:
  - `lw_demo` setup, slaves and items
  - DALI groups, DI titles, scenarios
  - floorplans, with widget fragments mapped back to item ids, and their images downloaded
- Workflow:
  1. Tune a device by hand.
  2. Export it, or ask Claude Code to do it with the same format (documented in `AGENTS.md`).
  3. Open a PR, cut a release.
  4. Everyone receives it as an app update.

## 14. Upgrades

When a device's `lw_demo.libraryVersion` is older than the bundled library, show "Upgrade
available". The preview lists added, removed and changed items and floorplans. Upgrade applies the changes and keeps local
overrides where possible. Never upgrade silently.

## 15. App updates & distribution

- GitHub Actions on tag:
  - universal macOS `.dmg`, signed and notarized (Team ID `XD59B34DMF`, App Store Connect API key as repo secret)
  - Windows installer, unsigned for now; the SmartScreen warning is accepted
  - Tauri updater metadata (`latest.json`) published to GitHub Releases
- On start, the app checks for a newer release and shows a "New version" badge. A click downloads, installs and restarts.

## 16. Activity log

A panel with the last 500 requests (time, method, path, status, short body) and "copy as curl" (with the
password masked). In memory only.

## 17. Migration of the prototype

The prototype's 135 items are migrated into the first library, in **native format**. Existing davaco
dashboards built on the old `<name>.value` fragments will break; this is accepted.

| Prototype items | Become |
|---|---|
| lights | native DALI lamps |
| the 10 binaries | the 10 **native digital inputs** (`Value01`..`Value10`, titles in config) (changed 2026-10-12, see DECISIONS.md) |
| other numbers / modes | Modbus registers on slaves per prototype group |
| custom alarms | slave `alarms[]` (`lw_demo_<id:02>_<name>`) or device-level `customAlarm` (`lw_demo_<name>`) (changed 2026-10-12, see DECISIONS.md) |
| AC mode | a multi-effect item |

## 18. Testing

- Rust unit tests: payload builders (native names, DI config/title mapping (changed 2026-10-12, see DECISIONS.md), slave/address assignment, custom alarm type rules),
  generators, operation handling, export ↔ apply round trip.
- CI: JSON schema validation of `library/**`, Biome, clippy, `cargo test`, frontend type check.
- An integration test against a recorded/fake Cumulocity API.
- No UI e2e tests in v1.

## 19. Open points (small, decide during build)

- `description`: plain top-level property as requested. Check that it shows in Device Management
  (otherwise also mirror it to `c8y_Notes`).
- Map picker tiles: OpenStreetMap via Leaflet, unless a Lichtwart-preferred provider exists.
- DALI `Brightness`: the firmware might send the raw arc level (0–254) rather than %. Check on a real device; v1 sends %.
- Analog inputs: add once the firmware defines their naming (4 inputs, `lw_fetchAI`).
