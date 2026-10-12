# Research: davaco-demo floorplan plugin (what the simulator must provide)

Source: `/Users/stephan/Coding/lichtwart/davaco-demo` (Angular 18 c8y plugin, `AGENTS.md:8-12`). Paths below are relative to it. Schema source of truth: `src/app/floorplan/model/floorplan.model.ts` (cited as `model.ts`). `AGENTS.md` and `CLAUDE.md` are identical (CLAUDE.md is a symlink, `AGENTS.md:3-4`). "UNVERIFIED" = not provable from the repo.

## 1. The `lw_floorplan` managed object

`FloorplanManagedObject` (`model.ts:278-286`, built by `toManagedObject`, `src/app/floorplan/model/managed-object.ts:9-27`):

```
{ type: "lw_floorplan", name: <string>, lw_floorplan: { description, icon?, imageBinaryId, imageWidth, imageHeight, imageOpacity?, widgets[] } }
```

| Field | Type / rule | Source |
|---|---|---|
| `type` | constant `"lw_floorplan"` (`FLOORPLAN_MO_TYPE`) | `model.ts:278` |
| `name` | string, list is sorted by name; first opens by default | `prd.md:60-61`, `floorplan-storage.service.ts:144` |
| `lw_floorplan.description` | string (always written, may be `""`) | `managed-object.ts:14` |
| `lw_floorplan.imageBinaryId` | **string** (c8y binary id, `String(data.id)`) | `floorplan-storage.service.ts:263-269` |
| `lw_floorplan.imageWidth` / `imageHeight` | numbers, **image pixels** of the uploaded file; widget x/y are in these pixels. SVG: width/height in px else `viewBox` | `model.ts:172-174`, `floorplan-storage.service.ts:70-113` |
| `lw_floorplan.icon` | optional c8y icon name, default `"map"` (`DEFAULT_FLOORPLAN_ICON`) | `model.ts:170,180-185` |
| `lw_floorplan.imageOpacity` | optional 0.1-1, missing = 1 | `model.ts:175,187-195` |
| `lw_floorplan.widgets` | array of widgets (below), missing tolerated (`[]`) | `managed-object.ts:30-44` |

Image: PNG, JPG or SVG only (`ALLOWED_IMAGE_TYPES`, `floorplan-storage.service.ts:53`). Uploaded with `binaries.create(file, {name: file.name, type: mime})` (`:263-269`), i.e. `POST /inventory/binaries` multipart with object `{name,type}`; the returned id is stored. `fromManagedObject` tolerates missing parts (`managed-object.ts:29-44`): defaults `locked=false`, `decimals=1`, `ranges=[]`, `alarmTypes=[]`.

## 2. How floorplans are linked and found

- **Child addition** of the device (or of a device group): create the MO, then `inventory.childAdditionsAdd(newId, ownerId)` = `POST /inventory/managedObjects/{ownerId}/childAdditions` `{managedObject:{id}}`; if linking fails the MO is deleted again (`floorplan-storage.service.ts:195-207`).
- **Lookup**: `inventory.childAdditionsList(ownerId, {pageSize: 2000})` = `GET /inventory/managedObjects/{ownerId}/childAdditions?pageSize=2000`, then **client-side filter** `mo.type === "lw_floorplan"` (no server-side type filter exists for child additions) (`:138-145`). Device floorplans: ownerId = device id; group floorplans: ownerId = `c8y_DeviceGroup` id (`prd-group-floorplans.md:29-31`).
- Delete: `DELETE /inventory/managedObjects/{id}` then delete binary (`:238-244`). Replace image: new binary, update MO, delete old binary (`:215-236`). Save = full `PUT` update, last save wins (`:209-213`, `prd.md:26`).
- The `lw_floorplan` MO has no other links (not an asset child, no external id).

## 3. Widget schema

Common (`WidgetBase`, `model.ts:18-29`): `id` string (uuid in practice; fixtures use e.g. `"w-freezer"`), `x`, `y` numbers (image pixels), `locked` boolean, optional `deviceId` (string; **missing = the device that owns the floorplan**; group floorplans always set it). `kind` discriminates: `"sensor" | "alarm" | "control"` (`model.ts:162-163`).

### 3.1 `kind: "sensor"` (`model.ts:55-73`)
| Field | Type | Notes |
|---|---|---|
| `icon` | string | c8y icon name (e.g. `"snowflake-o"`) |
| `title` | string | |
| `measurement` | `{fragment, series}` | fragment and series of the device's measurement; read live via `valueFragmentType/Series` (`floorplan-live-data.service.ts:359-362`) and realtime; value read as `m[fragment][series].value` (`:482`; the newest entry by `time` wins, `:479-489`) |
| `unit?` | string | shown unit |
| `decimals` | number (default 1) | number display only |
| `display?` | `{kind:"number"}` or `{kind:"boolean", onValue, offValue, onLabel, offLabel, onLevel?, offLevel?, stateIcons?:{on,off}\|null}` (`model.ts:36-53`) | boolean display: two readings mean on/off, anything else shown as number; `onLevel/offLevel` = `"good"\|"warning"\|"alarm"` |
| `ranges` | `Range[]` = `{min?:number\|null, max?:number\|null, level:"good"\|"warning"\|"alarm"}`; `min` inclusive, `max` exclusive, null = no limit; must not overlap | `model.ts:5-10` |
| `alarmTypes` | string[] | c8y alarm `type` values watched; ACTIVE/ACKNOWLEDGED -> bell (`prd.md:131-132`) |
| `staleAfterMin?` | number\|null | non-null = stale detection on; threshold actually used = device `lw_configCore.measurementIntervalMinutes` (default 10) + 5 min (`visual-state.ts:18-26`), the saved number is only a flag on Cores |

### 3.2 `kind: "alarm"` (`model.ts:75-81`)
`icon`, `text` (string, shown below), `note?` (string), `alarmTypes` (string[]). Green icon without alarm, bell/red with an ACTIVE or ACKNOWLEDGED alarm of a listed type (`prd.md:155-158`).

### 3.3 `kind: "control"` (`model.ts:153-160`)
`icon`, `title`, `confirm` (boolean), `control` = one of:

- **DALI** (`model.ts:143-149`): `{type:"dali", group:<number, id from lw_configGroups, 1-based>, modes:["off","on","auto"] subset (>=2, any order)}`.
- **Modbus** (`model.ts:104-139`): `{type:"modbus", target, readback?, valueType, ...}` where
  - `target` = `{source:"saved", ref:"<slaveAddress>:<registerAddress>", slaveAddress, registerAddress, type:"coil"|"register"}` (ref built at `src/app/floorplan/settings/control/control-settings.component.ts:340-347`, note ref uses the **slave Modbus address**, not the slave id) or `{source:"manual", slaveAddress, registerAddress, type}` (`model.ts:91-102`). `type` is `"coil"` (FC5) or `"register"`; the device's own word `holding` maps to `"register"` (`control-settings.component.ts:337-347`). slaveAddress 1-247, registerAddress 0-65535 (`prd.md:204-205`).
  - `readback?: {fragment, series} | null`: measurement reporting the real value, e.g. `{fragment:"LW_Modbus_Slave_10_001", series:"Mode"}` (`model.ts:107-109`).
  - `valueType:"boolean"`: `onLabel`, `offLabel`, `invert` (ON writes 0 / OFF writes 1). `valueType:"number"`: `min`, `max`, `step`, `unit?` (values clamped/rounded to 0-65535). `valueType:"multi"`: `options:[{value:int 0..65535 unique, label, icon}]` (always written as register) (`model.ts:112-137`, `control-logic.ts:13-39,72-87`).

### 3.4 How the three references are resolved (what the simulator must write)
- **Measurement fragment/series**: literal pair stored in the widget. Native names: DALI `{fragment:"LW Dali Control Gear 1-NN", series:"Brightness"}`; Modbus `{fragment:"LW_Modbus_Slave_<id:02>_<address:03>", series:<register name>}`, built by `slaveMeasurementFragment(id,address)` (`src/app/floorplan/settings/modbus-slaves.ts:33-35`); DI `{fragment:"LW Digital Input", series:"Value03"}` (native names, no title in the series; decision 2026-10-12). **There is no DI-specific code in the plugin** (grep over `src/app/floorplan`: no digital-input handling): a DI widget is an ordinary `kind:"sensor"` widget with that literal fragment/series (usually `display.kind:"boolean"` for 0/1), and **no alarm suggestions exist for DI** (`alarm-type-derivation.ts:142` area: only DALI, Modbus, motion, brightness, temperature and system groups are derived), so `alarmTypes` of a DI widget is written literally or left empty. The widget label comes from the library title (`lw_configDigitalInputs[].title`), falling back to the input number.
- **alarmTypes**: literal c8y alarm type strings. The plugin's picker derives suggestions only from measurement fragments (`src/app/floorplan/settings/alarm-types-field/alarm-type-derivation.ts:127-143`): `^lw_modbus_slave_(\d+)_\d+$` -> `lw_modbus_NN_timeoutError|configError`; `lw_dali_control_gear_\d+_(\d+)` -> converter group (`lw_controlGearFailure_1_NN`, `lw_converterFailure_1_NN`, `lw_converter_1_NN_lost|notExisting|doubleAddressing`, `lw_lampFailure_1_NN_{shortCircuit,openCircuit,loadDecrease,loadIncrease,currentProtectorActive,thermalShutdown,thermalOverload,referenceMeasuerment,generic}`, `:75-86`); `LW Dali Brightness`/`Lux<n>` -> `lw_brightness_1_NN_{lost,notExisting,wrongBitType,wrongOpType}`; `LW Motion`/`State<n>` -> `lw_motion_1_NN_{lost,notExisting}`; `LW Dali Temperatures` -> `lw_temperatureSensor_1_NN_...`; system: `lw_powerFailure, lw_daliBusPowerOff|Fail|Wrong, lw_converterExceed, lw_ntpSyncFailed` (`:54-61`). Custom alarm types (e.g. `lw_demo_19_co2Leakage`) are never suggested; they must be written literally by the simulator (the fixtures use custom alarm types such as `lw_sensorLost`, `lw_freezerDoorOpen`, `lw_hoodFanFailure`, `lw_co2Leak` the same way: `model/fixtures.ts:52,141,203`). Note the typo `referenceMeasuerment` is intentional to match the firmware (`:17-18`).
- **Modbus control target**: literal `slaveAddress`/`registerAddress`/`type` (copied so sending needs no lookup, `model.ts:91-92`). Resolved from the device `lw_configModbusSlaves[].address` and `writeRegisters[]` (section 4).

## 4. Device fragments the plugin reads

- **Tab visibility**: `c8y_Hardware.model` must equal `"Lichtwart Core v2"` (v2) or `"RemoteSmartInterface"` (v1); anything else = no tab (`src/app/floorplan/model/device-profile.ts:25-39`, constants in `node_modules/@lichtwart/cot-widget-types/dist/utils.js:1-2`). The tab factory calls `inventory.detail(id)` and checks the profile (`src/app/floorplan/floorplan-tab/floorplan-tab.factory.ts`).
- **Modbus enabled** only if `c8y_Firmware.version` >= `01.06.00` (`MODBUS_MIN_FIRMWARE`, `device-profile.ts:18`); parsed by `parseFirmwareVersion` which strips an optional leading `V` + spaces and splits on `.` -> `major*1e6+minor*1e3+patch` (`utils.js:23-27`). `"V 03.00.02"` and `"03.00.02"` both pass; a version that does not parse (e.g. `V 02.01.00_MS_pre`) counts as not newer (`device-profile.ts:20-23`). **`c8y_Firmware.version` must therefore stay parseable as `[V ]NN.NN.NN`, otherwise all Modbus controls are hidden.** The simulator keeps `"V 03.00.02"`, exactly like the real firmware. Without Modbus support the plugin hides all slaves (`floorplan-device-lookup.service.ts:123`), and "Get measurements" is offered only with Modbus (`device-profile.ts:14-15`).
- `lw_configGroups` (v2) / `LW_ConverterGroups` (v1): array, reads `id` (integer) and `name` (default `Group <id>`) (`floorplan-device-lookup.service.ts:107-112`).
- `lw_configCore.measurementIntervalMinutes` (also accepts `measurementsIntervalMinutes`) for the stale threshold (`:116-118`). **Stale threshold = `measurementIntervalMinutes` + 5 min** (`STALE_GRACE_MIN`, default interval 10 -> 15 min = 900 s; `model/visual-state.ts:18-27`). Consequence: the simulator's `sendIntervalSec` must stay **below 900** (max 899) or sensors with stale detection flash stale between sends; if the device's interval is changed, the limit moves with it.
- `lw_configModbusSlaves[]` (`:54-90`): slave needs integer `address` else dropped; `id` integer (else falls back to `address` and read registers are empty); `name` (fallback `Slave <address>`); `registers[]` (needs string `name` + integer `address`; reads `type`, `unit`) -> read registers with `measurement = {fragment: LW_Modbus_Slave_<id:02>_<address:03>, series: name}`; **`writeRegisters[]`** `{name, address (integer), type}` with `type` `"coil"` or `"holding"` -> the control settings "saved target" picker (`src/app/floorplan/settings/modbus-slaves.ts:5-28`; fixtures `mock/mock-seed.ts:228-275`). `writeRegisters` is not a firmware field; it is a cloud-side addition the plugin reads (matches the PRD).
  - **A writable item needs entries in BOTH arrays**: `registers[]` (read side, gives the readback measurement fragment/series) **and** `writeRegisters[]` (the saved-target picker); the two are mapped separately (`floorplan-device-lookup.service.ts:73-78`). A register only in `writeRegisters[]` has no readback series, one only in `registers[]` is not selectable as a control target. Decision 2026-10-12: writable Modbus items go into both.
- Measurement list for pickers: `inventory.getSupportedSeries(deviceId)` (`GET /inventory/managedObjects/{id}/supportedSeries`), fallback `c8y_SupportedSeries`; entries `fragment.series` split at the first dot (`floorplan-device-lookup.service.ts:40-51,103,139-141`). **Simulator consequence**: series only show up in the plugin's pickers once measurements with that fragment/series exist on the device (c8y derives supportedSeries from stored measurements; delay UNVERIFIED), so a demo device must send each measurement at least once, including before the floorplan is edited.
- Live data (section 6) uses measurements, alarms, events and operations of the device id, not these fragments.

## 5. Operation payloads the plugin creates

All via `operations.create({deviceId, description, [fragment]: payload})` = `POST /devicecontrol/operations` (`floorplan-command.service.ts:193-198`). Every payload carries `by: <user email or userName>` (`:100-107`, `AGENTS.md:66`).

| Operation | `description` | Fragment and payload | Source |
|---|---|---|---|
| DALI group mode (Core v2) | `"DALI group mode"` | `{"lw_switchGroupMode":{"group":<int 1-based>,"value":"off"\|"on"\|"auto","by":"<email>"}}` | `model/dali-switch.ts:33-37` |
| DALI group mode (Core v1) | `"Switch converter group"` | `{"LW_ServiceSwitch":{"group":n,"value":0\|1\|2,"by":..}}` (auto 0, on 1, off 2) | `dali-switch.ts:8-31` |
| Modbus write | `"Modbus write"` | `{"lw_modbusWrite":{"slaveAddress":n,"registerAddress":n,"value":<int 0-65535>,"type":"coil"\|"register","by":".."}}` | `floorplan-command.service.ts:236-243` |
| Get measurements | `"Request measurements"` | `{"c8y_MeasurementRequestOperation":{"requestName":"LOG","by":".."}}` | `live-state.ts:30-31`, `command.service.ts:85-97` |

- **`lw_switchConverterState` is not used by the plugin at all** (no hit in `src/`); only the firmware/PRD list it. Same for `lw_setGroupColor`, `c8y_Restart`, `c8y_Firmware`, `lw_fetch*`, `lw_dali*`.
- Boolean write value: `on !== invert ? 1 : 0` (`control-logic.ts:25-28`); number: clamped to 0-65535 and rounded (`:13`); multi: must be one of the option values.
- Timing the simulator must respect: the plugin tracks the op via operation realtime (`onAll$(deviceId)`), and **after 10 s (`PENDING_TIMEOUT_MS`) in status PENDING it cancels it** with `PUT` status `FAILED`, `failureReason: "Not picked up by the device within 10 seconds, cancelled."` (`floorplan-command.service.ts:24-26,164-176,201`). It only cancels while the last known status is PENDING, so setting EXECUTING immediately (PRD §9) avoids the cancel. SUCCESSFUL ends the command; FAILED shows `failureReason` (`:148-162`).
- **Operations the plugin already set FAILED must be ignored by the simulator**: either the 10 s cancel above (`:165-176`) or the supersede rule below (`:258-268`) can mark an op FAILED while the realtime event still reaches the simulator. Applying its effect afterwards would change device state although the UI reported failure. Once an op is EXECUTING the plugin has **no timeout** any more, so the PRD §9 60 s confirm timeout is the only thing that ends it.
- Before sending a group mode op the plugin sets every still-PENDING op of the same fragment and group to FAILED with `"Superseded by a newer command from the floorplan."` (`:246-270`).
- The plugin restores state from SUCCESSFUL operations: for Modbus it uses `lw_modbusWrite` ops (matching key `<type>:<slaveAddress>:<registerAddress>` and `value`) and the **creationTime**, newest wins (`live-state.ts:45-47`, `floorplan-live-data.service.ts:512-532`); so the operation fragment the simulator receives is the source of truth and the simulator must answer SUCCESSFUL for the state to stick. DALI mode state comes from the newest `lw_daliGroupChange` event (`groupModeChange.newMode`, case-insensitive; time = min(device `time`, server `creationTime`, now)), falling back to the newest SUCCESSFUL `lw_switchGroupMode` op (`dali-event.ts:36-49`, `live-state.ts:163-183`, `prd.md:245-251`). So `lw_switchGroupMode` must be followed by the event (PRD §7.2 does that).
- Modbus readback: the control shows the last written value; the readback measurement only confirms (`live-state.ts:132-160`; `src/app/floorplan/model/readback.ts`). So after `lw_modbusWrite` the simulator should send a measurement for the readback series with the new value (PRD §7.4 plans this). **The comparison is `readback.value === state.value` on raw register values and ignores `invert`** (`model/readback.ts:15-23`), so the simulator must echo **exactly the operation's `value`** (no scaling, no inversion, no rounding); anything else shows "mismatch".

## 6. Live data and alarms the plugin consumes

- Measurements: initial `GET /measurement/measurements?source=<id>&valueFragmentType=<f>&valueFragmentSeries=<s>&dateFrom=...&revert=true&pageSize=1`, then realtime (`floorplan-live-data.service.ts:355-370`).
- Alarms: `GET /alarm/alarms?source=<id>&type=<csv of watched types>&status=ACTIVE,ACKNOWLEDGED&pageSize=2000`, then realtime; `CLEARED` removes the alarm (`:374-390,492-500`). Severity strings `CRITICAL|MAJOR|MINOR|WARNING`, status `ACTIVE|ACKNOWLEDGED|CLEARED` (`model.ts:199-200`). Acknowledgements made by the plugin add a fragment `lw_acknowledgedBy` (`model.ts:202-204`).
- Events: `GET /event/events?source=<id>&type=lw_daliGroupChange` newest first, then realtime (`:392-420`).
- The plugin treats alarm `type` as an exact match to the widget's `alarmTypes`, so alarm types raised by the simulator must equal the strings stored in widgets.

## 7. Full example floorplan JSON (what the app should POST)

Created with `POST /inventory/managedObjects` (no `id`), then `POST /inventory/managedObjects/{deviceId}/childAdditions` with `{"managedObject":{"id":"<newId>"}}`. Slave id 19 / address 239, slave id 20 / address 240, lamps from `daliLampCount`:

```json
{
  "type": "lw_floorplan",
  "name": "Erdgeschoss",
  "lw_floorplan": {
    "description": "Tankstelle: Shop und Aussenbereich",
    "icon": "map",
    "imageBinaryId": "48211",
    "imageWidth": 3226,
    "imageHeight": 1626,
    "widgets": [
      {
        "id": "w-freezer", "kind": "sensor", "x": 406, "y": 430, "locked": true,
        "icon": "snowflake-o", "title": "Tiefkuehler",
        "measurement": { "fragment": "LW_Modbus_Slave_20_240", "series": "freezer_temp" },
        "unit": "°C", "decimals": 1,
        "ranges": [
          { "min": null, "max": -15, "level": "good" },
          { "min": -15, "max": -12, "level": "warning" },
          { "min": -12, "max": null, "level": "alarm" }
        ],
        "alarmTypes": ["lw_modbus_20_timeoutError", "lw_demo_20_freezerDoor"],
        "staleAfterMin": 15
      },
      {
        "id": "w-cold-door", "kind": "sensor", "x": 1900, "y": 640, "locked": true,
        "icon": "open-garage-door", "title": "Kuehlraumtuer",
        "measurement": { "fragment": "LW_Modbus_Slave_20_240", "series": "cold_door" },
        "decimals": 0,
        "display": { "kind": "boolean", "onValue": 1, "offValue": 0, "onLabel": "Offen", "offLabel": "Geschlossen",
                     "onLevel": "warning", "offLevel": "good" },
        "ranges": [], "alarmTypes": [], "staleAfterMin": null
      },
      {
        "id": "w-lamp-1", "kind": "sensor", "x": 723, "y": 236, "locked": true,
        "icon": "light-bulb", "title": "Zapfsaeulen-Licht",
        "measurement": { "fragment": "LW Dali Control Gear 1-01", "series": "Brightness" },
        "unit": "%", "decimals": 0,
        "ranges": [], "alarmTypes": ["lw_lampFailure_1_01_openCircuit", "lw_converter_1_01_lost", "lw_controlGearFailure_1_01"],
        "staleAfterMin": 15
      },
      {
        "id": "w-co2", "kind": "alarm", "x": 1560, "y": 330, "locked": true,
        "icon": "air-quality", "text": "CO2-Tank", "note": "Leck: Hintertuer oeffnen",
        "alarmTypes": ["lw_demo_19_co2Leakage"]
      },
      {
        "id": "w-hoodfan", "kind": "control", "x": 1050, "y": 1400, "locked": false,
        "icon": "fan", "title": "Abluft", "confirm": false,
        "control": {
          "type": "modbus",
          "target": { "source": "saved", "ref": "240:2", "slaveAddress": 240, "registerAddress": 2, "type": "coil" },
          "valueType": "boolean", "onLabel": "An", "offLabel": "Aus", "invert": false,
          "readback": { "fragment": "LW_Modbus_Slave_20_240", "series": "hood_fan" }
        }
      },
      {
        "id": "w-ac-set", "kind": "control", "x": 2680, "y": 760, "locked": false,
        "icon": "thermometer-half", "title": "Klima Soll", "confirm": true,
        "control": {
          "type": "modbus",
          "target": { "source": "saved", "ref": "239:40002", "slaveAddress": 239, "registerAddress": 40002, "type": "register" },
          "valueType": "number", "min": 16, "max": 28, "step": 1, "unit": "°C",
          "readback": { "fragment": "LW_Modbus_Slave_19_239", "series": "ac_setpoint" }
        }
      },
      {
        "id": "w-ac-mode", "kind": "control", "x": 2680, "y": 400, "locked": false,
        "icon": "air-conditioner", "title": "Klima", "confirm": false,
        "control": {
          "type": "modbus",
          "target": { "source": "saved", "ref": "239:40001", "slaveAddress": 239, "registerAddress": 40001, "type": "register" },
          "valueType": "multi",
          "readback": { "fragment": "LW_Modbus_Slave_19_239", "series": "ac_mode" },
          "options": [
            { "value": 0, "label": "Aus", "icon": "power-off" },
            { "value": 1, "label": "Kuehlen", "icon": "snowflake-o" },
            { "value": 2, "label": "Heizen", "icon": "fire" },
            { "value": 3, "label": "Automatik", "icon": "rotate-right" }
          ]
        }
      },
      {
        "id": "w-dali-shop", "kind": "control", "x": 2150, "y": 400, "locked": false,
        "icon": "light-bulb", "title": "Shop-Licht", "confirm": false,
        "control": { "type": "dali", "group": 1, "modes": ["off", "on", "auto"] }
      }
    ]
  }
}
```
(`widgets[].deviceId` is omitted on a device floorplan, `model.ts:25-28`. Field shapes copied from `src/app/floorplan/model/fixtures.ts:36-304`; example values are illustrative.)

## 8. Facts that affect the PRD

1. `lw_floorplan` / child addition / client-side type filter: exactly as PRD §12 steps 1-3 (`POST /inventory/binaries`, `POST /inventory/managedObjects`, `POST .../childAdditions`). Add `icon`/`imageOpacity` as optional. `imageBinaryId` is a string.
2. `ref` of a saved Modbus target is `<slaveAddress>:<registerAddress>` (address, not id) - build it that way in the export/apply mapping (`control-settings.component.ts:346`).
3. `lw_modbusWrite.type` is `coil|register` while `writeRegisters[].type` is `coil|holding`; both must be filled consistently by the apply step.
4. 10 s cancel of PENDING operations (section 5) is an issue for the PRD §9 confirm popup only if the simulator waits before setting EXECUTING; the PRD sets EXECUTING immediately, which is correct and sufficient.
5. The plugin never creates `lw_switchConverterState`; DALI "identify" from the floorplan does not exist.
6. Device firmware version `V 03.00.02` in PRD §6.1 passes the `>= 01.06.00` check (`utils.js:23-27`). The real firmware also sends `V 03.00.02` **with** the `V ` (`GD_FIRMWARE_VERSION`, `GD_global_data.c:76`; firmware report section 0). An earlier version of this section claimed the opposite; that was wrong. The value must stay parseable as `[V ]NN.NN.NN`, else Modbus controls hide.
7. `lw_configModbusSlaves[].writeRegisters` is read by the plugin, ignored by the firmware; `description` on a slave is not read by either (UNVERIFIED for Device Management display).
8. Writable Modbus items need entries in both `registers[]` and `writeRegisters[]` (section 4); readback echoes exactly the op value (section 5).
9. `sendIntervalSec` max 899 (stale threshold interval + 5 min, section 4); ignore operations the plugin already set FAILED (section 5).
10. No DI-specific plugin code and no DI alarm suggestions (section 3.4).
