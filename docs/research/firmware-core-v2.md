# Research: what Core v2 firmware sends to / expects from Cumulocity

Source: `/Users/stephan/Coding/lichtwart/firmware-core-v2` (branch `devel`, CHANGELOG top entry `[pre release / BETA] [03.00.02]`, `CHANGELOG.md:3`). `firmware-core-v2-beta` is an older tree (no `Value%02` / `LW Dali Control Gear` strings found in `Modules/`, and no digital-input measurement code at all), so it was not used.
All paths below are relative to `firmware-core-v2/` unless absolute. "UNVERIFIED" = not provable from the repo.

## 0. Transport (important for the simulator)

- The firmware talks **SmartREST 2.0 over MQTT**, not the REST JSON API. Measurements use the Cumulocity static template `201` (custom measurement), alarms `301-304`, events `400` / custom `3200` (`Modules/C8Y_cumulocity/inc/C8Y_smart_rest.h:71-82`, topic `s/us` at `:37`).
- The JSON the tenant stores is therefore what the static templates produce. Static template semantics (`201,type,time,fragment,series,value,unit,...`; `210` -> `c8y_SignalStrength.rssi` dBm; `211` -> `c8y_TemperatureMeasurement.T` C; `301..304` = CRITICAL/MAJOR/MINOR/WARNING `type,text,time`; `400,type,text,time`) are Cumulocity-documented, not in this repo -> **UNVERIFIED here**, but the firmware's own format strings below match them.
- Time is left empty by the firmware (`201,<type>,,<fragment>...`), so the tenant stamps the receive time. Only event `3200` carries a device ISO time (`C8Y_light_system.c:725-760`).
- Alarm severity ids: 301 CRITICAL, 302 IMPORTANT(=MAJOR), 303 LESS_IMPORTANT(=MINOR), 304 WARNING, 306 CLEAR (`Modules/ERM_error_manager/inc/ERM_error_manager.h:159-165`; `C8Y_smart_rest.h:75-79` names 302 `MAJOR`, 303 `MINOR`).
- Device registration (`Modules/C8Y_cumulocity/src/C8Y_cumulocity.c:547-557`, `inc/C8Y_smart_rest.h:47-52`): MO name `Lichtwart Core v2 Setup {serial}`, `type: "LW_Core_v2"`, supported ops string (identical to PRD §6.1 list, `C8Y_smart_rest.h:49`), required interval 60 (`:52`, i.e. `c8y_RequiredAvailability`; the PRD deliberately omits it), serial/model/hardware `c8y_Hardware.model = "Lichtwart Core v2"` (`:51`), `c8y_Firmware.name = "LW_Core_v2_M2M"` (`:50`), version string **`V 03.00.02` with the leading `V `**: `GD_FIRMWARE_VERSION "V 03.00.02"` (`Modules/GD_global_data/src/GD_global_data.c:76`), copied unchanged by `GD_GetFirmwareVersionStr` (`:641-643`) and sent via SmartREST template 115 `SMART_REST2_SET_FIRMWARE` (`C8Y_cumulocity.c:556-557`, id at `C8Y_smart_rest.h:68`). The simulator keeps `"V 03.00.02"`. The floorplan plugin accepts both `V 01.06.00` and `01.06.00`, but the value must stay parseable as `[V ]NN.NN.NN` (see floorplan-plugin.md section 4). An earlier version of this report claimed there is no `V`; that was wrong.
- The firmware only **reads** the `lw_config*` inventory fragments (via `lw_fetch*` operations) and only **writes** `lw_connected*` fragments (`Modules/C8Y_cumulocity/src/C8Y_cumulocity.c:928,1044,1101,1158,1214`). It never writes `lw_config*` back.

## 1. Measurements

All lines are produced in `Modules/C8Y_cumulocity/src/C8Y_str_parser_getter.c` unless noted. **Each CSV line is one measurement object**; one DALI lamp = one measurement with 4 series.

### 1.1 System

| Measurement | Format | Source | Resulting JSON (per static template, UNVERIFIED) |
|---|---|---|---|
| Signal strength | `210,<rssi int>` | `:4584-4587`, id `C8Y_smart_rest.h:72` | `c8y_SignalStrength.rssi {value, unit:"dBm"}`; doc `Documentation/Core_v2-Documentation/02-Messwerte.md:37-40` |
| Device temperature | `211,<%.1f>` (rounded to 0.1) | `:4422-4428`, id `C8Y_smart_rest.h:73` | `c8y_TemperatureMeasurement.T {value, unit:"C"}`; doc `02-Messwerte.md:44-47` |

Both are sent in every periodic cycle together (`C8Y_light_system.c:458-462`).

### 1.2 DALI control gear (lamps)

Format string, `C8Y_str_parser_getter.c:4541`:
`201,daliControlGear,,LW Dali Control Gear 1-NN,Converter Status,<0|1>,,LW Dali Control Gear 1-NN,Lamp Status,<0|1>,,LW Dali Control Gear 1-NN,Converter Runtime,<hours>,hr,LW Dali Control Gear 1-NN,Brightness,<level>,%`

- type `daliControlGear`; fragment `LW Dali Control Gear 1-NN` (spaces in the fragment name; `1` = channel, `NN` = `%02u` short address + 1, i.e. 01-64; `LSY_light_system_data.h:35` max 64 devices).
- Series and units: `Converter Status` (unit empty), `Lamp Status` (unit empty), `Converter Runtime` (unit `hr`, integer hours = `runtime_s / 3600`, `C8Y_light_system.c:515-516`), `Brightness` (unit `%`).
- **Status encoding: 1 = OK, 0 = failure** (`!isControlGearFailure`, `!isLampFailure`, `C8Y_light_system.c:513-514`; doc `02-Messwerte.md:30-32`).
- **Lamps per message: max 6** (`CLOUD_SEND_LIMIT 6`, `C8Y_smart_rest.h:33`; chunking `C8Y_light_system.c:520-528`). A message = several CSV lines in one MQTT publish.
- A lamp is only sent if its address is valid, gear present, **it is assigned to a valid group** (`LSY_IS_VALID_GROUP(config.groupNr)`, `:508`), and no active `ERR_CONTROL_GEAR_LOST`/`NOT_EXISTING` alarm for that lamp (`:95-117`). Simulator consequence: a lamp that is not in any `lw_configGroups[].converters` never sends measurements on a real device; faulting a lamp as "lost" stops its measurements.
- Temperature DALI gears (Lunatone) use other fragments: type `daliTemperatures`, fragment `LW Dali Temperatures`, series `Intern<n>` / `Extern<n>`, unit `C`, `%.2f` (`:4446-4466`). Not in PRD scope.

### 1.3 DALI brightness sensor and motion

- Brightness: `201,daliBrightness,,LW Dali Brightness,Lux<N>,<int>,Lux` (`:4379-4380`), N = 1-10 (`MAX_BRIGHTNESS_SENSOR`, `LSY_light_system_data.h:37`), unit string is `Lux` (not `lx`). Suppressed on lost/not-existing/conversion/wrong-type alarms (`:4325-4334`). Doc `02-Messwerte.md:5-12`.
- Motion: `201,daliMotion,,LW Motion,State<N>,<state>` (**no unit field**), N = 1-10 (`:4403-4404,4412-4413`). Doc `02-Messwerte.md:14-21`; value semantics beyond 0/1 UNVERIFIED (`live.state`).
- Both are sent in the periodic cycle (`C8Y_light_system.c:458-459`); motion/brightness are additionally sent right after a successful `lw_fetchMotion`/`lw_fetchBrightness` activation (`C8Y_str_parser_getter.c:4738-4789`).

### 1.4 Digital inputs (max 10)

- `201,digitalInput,,LW Digital Input,Value<NN>,<0|1>` (`:4561`); NN = `%02u` of index+1 (01-10).
- `201,digitalInput,,LW Digital Input,ImpulseCounter<NN>,<uint32>,<unit>` (`:4577`); value = `impulseCounter + initCount` (`C8Y_light_system.c:478-480`), unit = `lw_configDigitalInputs[].unit` (max 19 chars, `04-Konfigurationen.md:235`).
- `Value` is sent for `type` `daliAction` and `measurement` (cyclic, `C8Y_light_system.c:486-488`, plus on change `Modules/IO_input_output/src/IO_application.c:742-755`, rate-limited by `lw_configCore.digitalInputsMsgPerMinute` [1,20], `04-Konfigurationen.md:36`); `ImpulseCounter` only for `impulseCounter` (doc `02-Messwerte.md:66-69`). Only `isActive` inputs are sent (`C8Y_light_system.c:478-486`).
- **CONFLICT with the original PRD §7.3 (resolved 2026-10-12, DECISIONS.md D1: native names)**: the firmware does **not** put the input title into the series name. It emits only `Value%02u` / `ImpulseCounter%02u` on every branch (`git grep` over `devel`, `main` and all local/remote branches; the beta tree has no digital-input code at all). There is no `ValueNN_<title>` and no title-cleaning (umlaut/length) code anywhere in `Modules/`. Further evidence:
  - The handoff documents list only `Value01..Value10` / `ImpulseCounter01..10`: `handoffs/core_v2_ontologie.html:736` ("Value01…Value10 bei daliAction und measurement") and `handoffs/core_v2_review.html:206` ("Messwerte `Value01…Value10` bei `type = daliAction` oder `measurement`").
  - Commit `d50730a8` (Klaus Kupfer, 2026-09-02) "The parsing of analog input names has been removed, and the transmission format has been aligned with that of the digital inputs" **removed** name-based analog series, i.e. the firmware team deliberately moved away from name-derived series.
  - The DI config template has no `title` field (`Documentation/SmartREST 2.0 Templates/SmartREST template LW-ConfigDI-V02.json`, pattern: `id,isActive,type,highLevelAction,lowLevelAction,initCount,increment,unit,groups[*]`). CHANGELOG only mentions the rename `ValueN` -> `ValueNN` (`CHANGELOG.md:30`).
  - Consequence for the simulator: series are `Value03` / `ImpulseCounter04`. The human title goes into `lw_configDigitalInputs[].title` (extra field, ignored by the firmware parser, harmless) and into the library; no title cleaning into series names.

### 1.5 Modbus

- `201,modbus,,LW_Modbus_Slave_<id:02>_<address:03>,<registerName>,<value>,<unit>` (`Modules/MBM_modbus_master/src/MBM_modbus_publish.c:524`, line terminator `\n`). `<id>` = slave id 1-20, `<address>` = slave Modbus address 1-247 zero-padded to 3 (doc `02-Messwerte.md:73-81`). Series = register `name` (<=19 chars, no spaces recommended), unit = register `unit` (<=7 chars, `MBM_modbus_master.h:32-33` incl. NUL: sizes 20 / 8).
- One measurement per register (a line per register). Registers with `conversion`/`custom` set are NOT sent this way but as a JSON measurement (`MBM_CreateJsonMeasurement`, `MBM_modbus_publish.c:561-700`: `{type:"modbus", time, "LW_Modbus_Slave_NN_AAA":{<name>:{unit,value}}, custom:{conversion,custom}}`); out of scope.
- Value formatting: the publisher passes `MBM_GLOBAL_DECIMAL_FACTOR` (100) to `MBM_FormatValueWithDecimals` **whenever the register `factor != 0`** (`MBM_modbus_publish.c:509-519`, constant `MBM_modbus_master_internal.h:29`), which then prints the stored integer divided by 100 with 2 decimals (fixed-point, `:181-215`); only a factor of 0/empty prints the raw integer without decimals. (The doc `02-Messwerte.md:96-99` says factor 0 or 1 -> raw; the publisher never hands factor 1.0 to the formatter, so that branch is dead in practice.) Irrelevant for the simulator over REST: it sends the final JSON number and no scaling.
- Only if `lw_configRs485.isActive` and the slave `isActive`, slave is connected, and the register was not blocked by a failed read (`MBM_modbus_publish.c:469-512`; doc `02-Messwerte.md:83-92`).
- Slave timeout behaviour: after `MBM_SLAVE_TIMEOUT_THRESHOLD` consecutive timeouts of the first bundle the slave is marked `isConnected = false`, the `lw_modbus_NN_timeoutError` alarm is raised and a `lw_modbus` event sent, and the slave's values are not published (`MBM_modbus_master.c:1545-1570`, threshold 3 at `:61`; doc `02-Messwerte.md:105-117`, `07-Ereignisse.md:115-117`).

### 1.6 Cycle / cadence

- Periodic cycle every `lw_configCore.measurementIntervalMinutes` (1-255, default 10; `C8Y_light_system.c:352-355`, doc `04-Konfigurationen.md:33`), plus on request (`c8y_MeasurementRequestOperation`).
- Order inside `C8Y_SendAllMeasurements` (`C8Y_light_system.c:435-540`): lux, motion, rssi, device temp, DALI temp sensors (one payload); digital inputs (one payload); DALI lamps in chunks of 6. Modbus measurements are published separately by `MBM_StartPublishRun` after the slave read round.

### 1.7 PRD §19 open point: is DALI `Brightness` % or raw arc level?

**Percent**, with moderate confidence. Evidence: `live.brightnessLevel` documented `[%]` (`Modules/LSY_light_system/inc/LSY_light_system_data.h:304`); doc says `[%]` (`02-Messwerte.md:30`); the DAPC query is routed through `MBS_CMD_QUERY_GEAR_DAPC_LEVEL`, whose result the API scales by `MBS_DALI_FACTOR_LEVEL = 1000` and rounds to an integer (`Modules/MBS_Dali_Stack/src/MBS_dali_api.c:228-233`, `inc/MBS_dali.h:31,317`); and the light-system compares the queried value directly with group `maxLevel` in percent (`Modules/LSY_light_system/src/LSY_light_system_functions.c:714-716`) and sends `maxLevel` via `DAL_SetActualLevel`. The conversion arc->percent itself happens inside the closed DALI library (`DALILIB_ACT_CTRL_GEAR_QUERY_ACTUAL_LEVEL`) -> exact curve **UNVERIFIED on hardware**. Simulator v1 sending `%` is consistent.

## 2. Alarms

Wire format: `<severityId>,<alarmType>,<text>\r\n`, up to 4 records per publish (`ERM_CLOUD_SEND_LIMIT = (500-1)/(120-1)`, `Modules/ERM_error_manager/src/ERM_error_manager_publish.c:51-55`; records built at `:164-185`; corrected 2026-10-12, this said 6). `<alarmType>` = alarm identifier; text/identifier built in `ERM_GetAlarmInfo` (`:538-731`); prefixes/suffixes in `Modules/ERM_error_manager/inc/ERM_alarm_blacklist.h:44-81`. NN = `%02u` of the detail code (01-64, modbus 01-20), channel is always `1`. A raise needs `errorManagerSetThresholdCounts` (default 3, `04-Konfigurationen.md:34`; `06-Alarme-und-Status-LED.md:28` still says 5) consecutive detections unless "Forced". The firmware **never clears the Cumulocity alarm**: `C8Y_SmartClearMultiAlarms` is defined (`C8Y_smart_rest.c:946`) but has no caller; instead it sends the event in section 3.1 (PRD §7.5 confirmed).

| Alarm type | Text | Severity id (name) | Raised at |
|---|---|---|---|
| `lw_controlGearFailure_1_NN` | `Control Gear Failure 1-NN` | 302 MAJOR (forced) | `ERM_..._publish.c:563-569`; `LSY_light_system_functions.c:1055` |
| `lw_converterFailure_1_NN` | `Converter Failure 1-NN` | 302 MAJOR | `publish.c:570-576`; `LSY_..._functions.c:934` |
| `lw_lampFailure_1_NN_shortCircuit` | `Lamp Failure 1-NN Short Circuit` | 302 MAJOR | `publish.c:577-579`; `LSY:962` |
| `lw_lampFailure_1_NN_openCircuit` | `Lamp Failure 1-NN Open Circuit` | 302 | `:581-583`; `LSY:956` |
| `lw_lampFailure_1_NN_loadDecrease` | `Lamp Failure 1-NN Load Decrease` | 302 | `:585-587`; `LSY:950` |
| `lw_lampFailure_1_NN_generic` | `Lamp Failure 1-NN Generic` | 302 | `:589-591`; `LSY:944` |
| `lw_lampFailure_1_NN_loadIncrease` | `Lamp Failure 1-NN Load Increase` | 302 | `:593-595`; `LSY:953` |
| `lw_lampFailure_1_NN_currentProtectorActive` | `Lamp Failure 1-NN Current Protector Active` | 302 | `:597-599`; `LSY:947` |
| `lw_lampFailure_1_NN_thermalShutdown` | `Lamp Failure 1-NN Thermal Shutdown` | 302 | `:601-603`; `LSY:968` |
| `lw_lampFailure_1_NN_thermalOverload` | `Lamp Failure 1-NN Thermal Overload` | 302 | `:605-607`; `LSY:965` |
| `lw_lampFailure_1_NN_referenceMeasuerment` (sic, typo is real; doc says `referenceMeasurement`) | `Lamp Failure 1-NN Reference Measuerment` | 302 | `:609-611`; `LSY:959` |
| `lw_converter_1_NN_lost` | `Converter 1-NN Lost` | 302 | `:625-627`; `LSY:458` |
| `lw_converter_1_NN_notExisting` | `Converter 1-NN Not Existing` | 302 | `:629-631`; `LSY:1025` |
| `lw_converter_1_NN_doubleAddressing` | `Converter 1-NN Double Addressing` | 302 (forced) | `:669-672`; `DAL_dali_initialise_functionality.c:1018,1348` |
| `lw_converterExceed` | `Converter Exceed` | 304 WARNING | `:673-676`; `LSY:898` |
| `lw_powerFailure` | `Power Failure` | 301 CRITICAL | `:677-680`; `LSY:1125` |
| `lw_daliBusPowerOff` / `...Fail` / `...Wrong` | `DALI Bus Power Off` / `Fail` / `Wrong` | 301 CRITICAL | `:681-695`; `DAL_dali_user_function.c:913,919,925` |
| `lw_daliBusPowerUnknown` | `DALI Bus Power Unknown` | n/a (documented "currently not used", `06:140`) | `:692-693` |
| `lw_temperatureSensor_1_NN_lost` | `TemperatureSensor 1-NN Lost` | 302 | `:613-615`; `LSY:480` |
| `lw_temperatureSensor_1_NN_externalNotConnected` | `TemperatureSensor 1-NN External Sensor Not Connected` | 302 | `:617-619`; `DAL_dali_temperature_sensor.c:156` |
| `lw_temperatureSensor_1_NN_notExisting` | `TemperatureSensor 1-NN Not Existing` | 302 | `:621-623`; `LSY:476` |
| `lw_brightness_1_NN_lost` / `notExisting` / `wrongBitType` / `wrongOpType` | `Brightness 1-NN Lost` / `Not Existing` / `Wrong Bit Type` / `Wrong Op Type` | 302 | `:633-655`; `DAL_dali_input_lightsensor_function.c:277` (wrongBitType), `:296` and `:303` (wrongOpType), `:307` and `:453` (`_lost`, `:453` in `DAL_HandleSensorLostError`), `:523` (notExisting) |
| `lw_brightness_1_NN_conversion` / `noConfig` | `Brightness 1-NN Conversion` / `No Config` | not raised ("currently not used", `06:80,89`) | `:633-647` |
| `lw_motion_1_NN_lost` / `notExisting` | `Motion 1-NN Lost` / `Not Existing` | 302 | `:657-667`; `DAL_dali_input_mose_function.c:339,344` |
| `lw_motion_1_NN_noConfig` | `Motion 1-NN No Config` | not raised | `:657-659` |
| `lw_modbus_NN_timeoutError` | `Modbus %02u-%03u Timeout Error <slaveName>` = `Modbus <NN>-<slaveAddress 3 digits> Timeout Error <name>` | 302 MAJOR (forced) | `publish.c:702-707`; `MBM_modbus_master.c:1557` |
| `lw_modbus_00_configError` | global Modbus config error (slave number 00, not tied to a configured slave); raised once at Modbus master init when the RS485 setup is invalid (unknown parity, `:694`, or `eMBMSerialInitExt` fails, `:708`), with detail code 0 | 302 (forced) | `MBM_modbus_master.c:694,708` |
| `lw_modbus_NN_configError` | `Modbus %02u-%03u Config Error <slaveName>` | 302 (forced) | `publish.c:696-701`; `MBM_modbus_master.c:417,454` |
| `lw_ntpSyncFailed` | `NTP Sync Failed` | 302 (forced) | `publish.c:708-711`; `MOD_ntp.c:1461` |

Notes:
- `lw_modbus_NN_*`: NN is the slave array index + 1 which equals the configured slave `id` because the parser stores the slave at index `id-1` (`C8Y_str_parser_getter.c:959`, `MBM_modbus_master.c:411,452`). Example text: `Modbus 20-240 Timeout Error Kuehlung`. The PRD text `Modbus <id>-<addr> Timeout Error <name>` is right but the numbers are zero-padded (`%02u-%03u`).
- Modem/SIM/RSSI/USB/NOR alarms exist internally (`ERM_error_manager.h`) but are **not published** (default branch of `ERM_GetAlarmInfo` returns false unless `ERM_UNDEFINED_SEND_ALARM`, `publish.c:712-728`).
- **No custom Modbus alarms exist in the firmware.** Only `timeoutError` and `configError`. `lw_demo_<id>_<name>` (PRD §7.4) is a pure simulator convention; the firmware has an alarm blacklist (`lw_configAlarmBlacklist`, patterns incl. wildcards, `04-Konfigurationen.md:239-303`) which would not know them.
- Alarm-blacklist patterns document the full type list (`04-Konfigurationen.md:260-298`); the floorplan plugin builds the same list (see floorplan-plugin.md section 5).

### 2.1 DALI bus power failure and what follows from it (researched 2026-10-12)

Bus alarm:
- libdali reports bus power only on a change: `DALILIB_BUSPOWER_OFF` (interruption 45-500 ms) and `DALILIB_BUSPOWER_SYSTEMFAILURE` (> 500 ms) (`Middlewares/Third_Party/MBS-Dali/dali_lib_3_3/inc/libdali.h:141-143`; `src/frame.c:335-363,393-431`). `MBS_DaliLLReactionCallbackHandler` maps them to the stack states `BUSPOWER_ON` / `BUSPOWER_OFF` / `BUSPOWER_FAIL`, anything else to `BUSPOWER_WRONG` (`Modules/MBS_Dali_Stack/src/MBS_dali.c:2569-2604`).
- `DAL_CheckDaliBusPowerError` runs every DALI task pass and sets / clears one `ERR_DALI_BUSPOWER` entry with the detail bits OFF 0x01 / FAIL 0x02 / WRONG 0x04 (`Modules/DAL_dali/src/DAL_dali_user_function.c:911-928`, bits `inc/DAL_dali_user_function.h:100-102`) -> `lw_daliBusPowerOff` / `Fail` / `Wrong`, 301 CRITICAL (`ERM_error_manager_publish.c:681-695`). Debounce: set 3, clear 2 calls (`ERM_error_manager.c:231-307,365-449`), i.e. milliseconds to seconds.
- A long outage is typically a short `Off` that turns into `Fail` (the `Off` bit is then cleared -> `Alarm cleared: lw_daliBusPowerOff`). `Wrong` needs an illegal libdali value and is practically unreachable.
- **There is no separate bus short-circuit or overload alarm.** A shorted / overloaded bus has no bus voltage and goes the same `Off` / `Fail` path (deduction: no code distinguishes them). Bus collisions raise no alarm, only the `lw_dali` event `DALI Collison Timer Occurs: N`. `thermalOverload` exists only per lamp. `lw_powerFailure` (mains) is the only case that suspends the DALI polling (`DAL_dali_user_function.c:1007`).

Per-lamp consequences (the cascade):
- `ERR_DALI_BUSPOWER` gates nothing (only the status LED reads it, `IO_application.c:469`); the gear polling keeps running. libdali refuses every action while bus power is not ON (`acceptActionsOnSystemFailure` = 0 after the `memset` at `MBS_dali.c:2691`; `dalistack.c:616-627`), so every query returns `MBS_DALI_RESULT_NO_ANSWER` (`MBS_dali.c:3018,3204-3206`).
- `LSY_CheckControlGearStatus` asks `QUERY_GEAR_PRESENT` up to 5 times; any invalid answer sets `gearPresent = false`; status bits and brightness are only overwritten on a valid answer (`Modules/LSY_light_system/src/LSY_light_system_functions.c:802-886`, `:84`). The step runs every `daliSchedulerErrorIntervalSeconds` (default 120 s, `LSY_light_system_dali_scheduler.c:368-381`).
- `LSY_CheckGearLampErrors` then raises, **for every gear listed in a group** (`:1022-1037`), `lw_converter_1_NN_lost` (302, `LSY_CheckLostGear`, `:449-463`, debounced: 3 checks) and right after it the forced `lw_controlGearFailure_1_NN` (302, `:1052-1059`; `LSY_IsGearGenericFailureActive` counts `ERR_CONTROL_GEAR_LOST`, `:431`). With the gear absent `lw_converterFailure_*` / `lw_lampFailure_*` are not raised (they need `gearPresent`; `:932-1000`). Estimated delay with defaults: a few minutes after the bus goes down.
- Sensors on the bus go lost the same way: `lw_motion_1_NN_lost` (`DAL_dali_input_mose_function.c:336-340`), `lw_brightness_1_NN_lost` (`DAL_dali_input_lightsensor_function.c:307,453`), DALI temperature gears `lw_temperatureSensor_1_NN_lost` (`LSY:477-481`).
- Measurements: a lamp is sent only with `gearPresent` and no active lost / not-existing alarm (`Modules/C8Y_cumulocity/src/C8Y_light_system.c:95-118,503-509`), so its `daliControlGear` measurement **stops** at the first failed poll. There is no "not responding" value; `Converter Status` / `Lamp Status` are 1 = OK, 0 = failure (`:513-517`).

Recovery: the next gear step finds the gear again (`gearPresent = true`, fresh status / level); after 2 good checks `Alarm cleared: lw_converter_1_NN_lost`, about one check later `Alarm cleared: lw_controlGearFailure_1_NN`; measurements resume once the lost alarm is gone, with fresh values. Sensors clear their lost alarms the same way (`mose:330-333`, `lightsensor:546`). No rescan needed.

Floorplan plugin: a DALI lamp is a sensor widget on `LW Dali Control Gear 1-NN`; it turns red only for an ACTIVE / ACKNOWLEDGED alarm of a type in the widget's `alarmTypes` (`davaco-demo/src/app/floorplan/model/visual-state.ts:86-113`), severity ignored. A bus alarm does not affect a lamp widget unless its type is listed there. The simulator's floorplans list all lamp alarms of the lamp (`autoAlarms`), incl. `_lost` and `controlGearFailure`, so the cascade turns them red.

Simulator (DECISIONS D8): any `lw_daliBusPower*` fault raises the cascade at once (lost + control gear failure per grouped lamp, `_lost` per brightness / motion sensor item, measurements stop) and recovery clears it right after the bus alarm.

## 3. Events

All sent as `400,<type>,<text>` (`C8Y_smart_rest.c:1014-1080`, `SMART_REST2_CREATE_BASIC_EVENT=400`) except 3.3.

### 3.1 `lw_alarm` - alarm cleared
- type `lw_alarm` (`C8Y_str_parser_getter.h:44`), text **`Alarm cleared: <alarmIdentifier>`**, e.g. `Alarm cleared: lw_lampFailure_1_05_shortCircuit` (`ERM_error_manager_publish.c:209,216`; doc `07-Ereignisse.md:11-13`). One event per detail alarm. Sent only when an active alarm really clears (debounced by `errorManagerClearThresholdCounts`, default 2).

### 3.2 `lw_modbus`
- type `lw_modbus`; text `Modbus slave %02u-%03u (%s): %s: %s` + optional ` - register(s) ...` (`MBM_modbus_master.c:749-770`):
  - timeout transition: `Modbus slave 20-240 (Kuehlung): Timeout alarm: Timeout - slave not responding - registers 30001-30004` (label `"Timeout alarm"`, `:1565-1566`; error string from `MBM_GetErrorString` code 8 = `Timeout - slave not responding`, `:2018`).
  - config transition: label `Config alarm`; read fail `Register read failed (<name>)`; reset fail `Reset write failed (<name>)` (doc `07-Ereignisse.md:115-129`).
  - Only at the transition into the error state (`07:117`).

### 3.3 `lw_daliGroupChange` (structured)
- Template `LW-Events-V03.00.00` (`Documentation/SmartREST 2.0 Templates/SmartREST template LW-Events-V03.00.00.json:6-52`): msg `3200`, mandatory `type,text,time`, custom `groupModeChange.id` (INTEGER), `.previousMode`, `.newMode`, `.reason` (STRING).
- Payload (`C8Y_light_system.c:725-760`): `3200,lw_daliGroupChange,Groupmode changed: <reason> - Group: <N> - From <Prev> to <New>,<ISO time>,<N>,<Prev>,<New>,<reason>`
- JSON equivalent: `{"type":"lw_daliGroupChange","text":"Groupmode changed: cloud-operation - Group: 4 - From Auto to On","time":"<ISO>","groupModeChange":{"id":4,"previousMode":"Auto","newMode":"On","reason":"cloud-operation"}}`.
- `id` is the **1-based group number**; mode strings are capitalised `Auto` / `On` / `Off` (`C8Y_light_system.c:314-329`); the plugin lower-cases them (`davaco-demo .../model/dali-event.ts:14-18`). reason: `cloud-operation` or `digital-input-NN` (`07-Ereignisse.md:39-45`). Sent only if the mode really changed (`LSY_light_system_functions.c:607-611`).

### 3.4 Other events (type -> text)
- `lw_firstConnect`, `lw_coreReady` = `Core Ready` (`C8Y_light_system.c:676`).
- `lw_dali`: `DALI Scan Started` / `DALI Scan Finished` / `DALI Scan Deactivated` (`C8Y_light_system.c:835-847`; the doc spells them `Dali Scan ...`), `DALI Collison Timer Occurs: <n>` (`:857`), `Newly Added Control Gears: <n>` / `Newly Added Input Devices: <n>` (`:873,882`), `Control Gear: 1-NN replaced GTIN: ... - SN: ... replaced with GTIN: ... - SN: ...` (`:815`), `Primary Group <a> triggered Secondary Shutdown Group <b>` / `Automatic Shutdown triggered Group <g>` (`:704-711`).
- `lw_rgb`: `GRGBWAF: <g>.<r>.<g>.<b>.<w>.<a>.<f>` (`:903`). `lw_time` (NTP/DST), `lw_update` (`Downloaded Update: 10%` ...), `lw_gps`, `lw_smartRestTemplate` (`Template <name> Not Existing`, `C8Y_str_parser_getter.c:1762`), `lw_fetch` = failure reason text during boot (`C8Y_smart_rest.c:1226`). Full list: `07-Ereignisse.md:5-145`.

## 4. Operations (cloud -> device)

Template `LW-Operations-V09` (`Documentation/SmartREST 2.0 Templates/SmartREST template LW-Operations-V09.json`); the device polls, gets CSV, and answers with status messages `504` EXECUTING / `506` SUCCESSFUL / `505` FAILED (+reason) (`C8Y_smart_rest.h:84-87`). Supported-operations list: `C8Y_smart_rest.h:49`. Parsing: `C8Y_str_parser_getter.c:1985-2600`. JSON examples: `Documentation/Core_v2-Documentation/03-Operationen.md`.

Common: the fragment shown is what Cumulocity stores on the operation (`{ "<fragment>": {...} }`); the plugin adds `by`, the firmware ignores it. Results are **status only** (no result fields). Some ops reject while a DALI scan/vendor scan is running (`FAILED`, `DALI bus scan is ongoing` / `DALI vendor scan ongoing`, `:1964-1973`).

| Operation | Fragment structure expected | What the device does / reports |
|---|---|---|
| `c8y_Restart` | `{"c8y_Restart":{}}` (msg 902) | Saves runtimes, EXECUTING, resets after 5 s; after boot reports SUCCESSFUL (`:2041-2057`; `C8Y_cumulocity.c:664-672,809-815`). |
| `c8y_Firmware` | `{"c8y_Firmware":{"url":"https://...","name":"...","version":"01.01.05"}}` (msg 901, URL must contain `https`) | EXECUTING, downloads, events `lw_update` `Downloaded Update: N%`, failure texts; SUCCESS after update boot (`:1992-2038`; `03-Operationen.md:171-190`, `07:111-145`). Simulator: just SUCCESSFUL. |
| `c8y_MeasurementRequestOperation` | `{"c8y_MeasurementRequestOperation":{"requestName":"LOG"}}` (msg 1200; alt `{"LW_Commands":{"Request_Measurement":1}}` msg 904) | EXECUTING; reads DALI + Modbus, sends all measurements, then SUCCESSFUL; FAILED `Measurement refresh ongoing` if already running (`:2060-2070`; `C8Y_light_system.c:236-300`). |
| `lw_setGroupColor` | `{"lw_setGroupColor":{"group":1,"red":0,"green":80,"blue":0,"white":0,"amber":0,"freecolor":0}}` (msg 914; values 0-254, >=255 clamped) | Sets RGBWAF for group, SUCCESSFUL; FAILED `Group not found` / `The specified lw_setGroupColor.group is incorrect` (`:2072-2160`); event `lw_rgb` `GRGBWAF: ...` (`c8ySendRGBWAFData`). |
| `lw_fetchCore` / `Groups` / `Motion` / `Brightness` / `DigitalInputs` / `AlarmBlacklist` / `Temperatures` / `ModbusSlaves` / `Rs485` (also `lw_fetchButtons`, `lw_fetchAI`, `lw_fetchDO` in template, not supported) | `{"lw_fetchXxx": true}` (msgs 915-926; handler matches the text `true`, `:2194-2335`) | EXECUTING; device GETs the MO and parses the matching inventory fragment: Core->`lw_configCore`, Groups->`lw_configGroups`, Motion->`lw_configMotion`, Brightness->`lw_configBrightness`, DigitalInputs->`lw_configDigitalInputs`, AlarmBlacklist->`lw_configAlarmBlacklist`, Temperatures->`lw_configTemperatures`, ModbusSlaves->`lw_configModbusSlaves` (+ reset registers + `lw_configRs485`), Rs485->`lw_configRs485` (`C8Y_FetchAllConfig`, `:4631-4661`). SUCCESSFUL when all parsed (`:4307`), else FAILED `Fetch Error: lw_configXxx not found or timeout` / `Fetch Error: The specified <field> is incorrect` (`:4667-4716`, doc `07:68-77`). **No inventory fragment is written back.** `isSavingOnStorage: true` is rejected (`:921-936`). Simulator: SUCCESSFUL (optionally validate). |
| `lw_switchGroupMode` | `{"lw_switchGroupMode":{"group":4,"value":"on"}}`; group 1-16 (1-based), value `on`/`off`/`auto` (msg 930) | `LSY_SetGroupSwitch(group-1, mode, "cloud-operation")`, goes **straight to SUCCESSFUL, the firmware never sends EXECUTING** for this op (`:2348-2388`); event `lw_daliGroupChange` only if mode changed (`LSY_..._functions.c:589-614`). The value is matched with a **case-sensitive `strstr` on the lowercase tokens** `on` / `off` / `auto` (so `On` or `AUTO` is rejected as incorrect). The simulator deliberately differs here for confirm-popup ops: it sets EXECUTING first (PRD §9) to beat the plugin's 10 s PENDING cancel. Errors: `The specified lw_switchGroupMode.value is incorrect`, `...group is incorrect`, `Invalid command for lw_switchGroupMode`. |
| `lw_switchConverterState` | `{"lw_switchConverterState":{"converter":4,"value":"true"}}`; converter 1-64, value `true`/`on`/`false`/`off` (msg 931) | Sets `singleSwitch` ON/OFF for one lamp (identify), straight to SUCCESSFUL, no EXECUTING (`:2391-2422`); value matched case-sensitively with `strstr` on lowercase `true`/`on`/`false`/`off`. Per doc the lamp must be off via its group first, a group-ON lamp cannot be switched off by this op (`03-Operationen.md:27`). No event. |
| `lw_daliReset` | `{"lw_daliReset":{"adress":3,"mode":"singleControlGear"}}` (**sic `adress`**, must exist; modes `all`,`allSensors`,`allControlGears`,`singleSensor`,`singleControlGear`; msg 932) | EXECUTING, erase on bus, SUCCESS later; FAILED `Erase ongoing`, `Invalid address`, `The specified lw_daliReset.mode is incorrect` (`:2425-2490`). |
| `lw_modbusWrite` | `{"lw_modbusWrite":{"slaveAddress":10,"registerAddress":40001,"value":1234,"type":"register"}}`; slaveAddress 1-247, registerAddress 0-65535, value 0-65535 (coil: 0/1), type `coil`(FC5) / `register`(FC6) (msg 933) | Needs `lw_configRs485.isActive` (else FAILED `RS485 interface inactive`); queues write (queue 3) -> EXECUTING; callback -> SUCCESSFUL or FAILED `Modbus write failed: <Modbus error text>`; FAILED `Modbus command queue full`, `Slave address out of range (1-247)`, `Coil value must be 0 or 1`, `Register value out of range (0-65535)`, `The specified lw_modbusWrite.type is incorrect` (`:1509-1620`). No readback measurement is sent immediately; the register value shows up with the next poll. |
| `lw_daliVendorScan` | `{"lw_daliVendorScan":{"address":3,"mode":"singleControlGear"}}` (**`address`** here; modes `all`,`singleSensor`,`singleControlGear`; msg 934) | EXECUTING, SUCCESSFUL after the scan; vendor data (GTIN/SN) is pushed to inventory `lw_connected*` and a `Device Replacement` `lw_dali` event may follow (`:2498-2575`; `03-Operationen.md:93-122`). |
| `lw_daliScan` | `{"lw_daliScan":true}` (msg 935) | EXECUTING, then SUCCESSFUL; events `DALI Scan Started`/`Finished` (`:2578-2590`; `03:124-138`). |

Unknown operation -> FAILED `Operation command not found or timeout` (`:2594-2596`). Pending ops are processed one at a time via polling; the plugin cancels a still-PENDING op after 10 s (see floorplan-plugin.md section 6), so the simulator must at least set EXECUTING within 10 s.

## 5. Inventory fragments (cloud-owned, device reads)

### 5.1 `lw_configCore` (`04-Konfigurationen.md:5-51`; template `LW-ConfigCore-V03.00.00` `:2500` fields)
```json
{ "lw_configCore": { "maxControlGears": 55, "measurementIntervalMinutes": 10, "digitalInputsMsgPerMinute": 10,
  "errorManagerSetThresholdCounts": 3, "errorManagerClearThresholdCounts": 2, "cloudWatchdogRestartTimeoutMinutes": 120,
  "isDaliScanActive": false, "isDaliScanForNewDevicesActive": true, "daliSchedulerIntervalSeconds": 60,
  "daliSchedulerDelayMilliseconds": 500, "daliSchedulerErrorIntervalSeconds": 60, "timeZone": "Europe/Berlin", "ntpServer": "" } }
```
Limits: maxControlGears [1,64], measurementIntervalMinutes [1,255], digitalInputsMsgPerMinute [1,20], cloudWatchdogRestartTimeoutMinutes [15,255], scheduler values [0,65535].

**The object must be written in full.** The PRD's old `{ "measurementIntervalMinutes": 10 }` is **not** a valid subset: the parser walks all 13 fields in order and fails `lw_fetchCore` with `Fetch Error: The specified lw_configCore.<field> not found` on the first empty/missing one (`C8Y_str_parser_getter.c:3384-3389` for `maxControlGears`, per-field checks `:3404-3571`). Only `timeZone` and `ntpServer` are optional (`:3376-3377` defaults, end-of-record check `:3649-3650`). The simulator writes all 13 fields with the realistic defaults above (`timeZone` `Europe/Berlin`, `ntpServer` `""`, `measurementIntervalMinutes` 10). The old name `measurementInterval` was renamed in 02.00.02 (`CHANGELOG.md:32`); the plugin reads `measurementIntervalMinutes` (floorplan-plugin.md section 4). On the unmerged branch `origin/feature_analog_input` the object additionally carries `analogInputsMsgPerMinute` (`04-Konfigurationen.md:17` there).

### 5.2 `lw_configRs485` (`04-Konfigurationen.md:360-387`)
`{"lw_configRs485":{"parity":"none|even|odd","baudrate":9600-115200,"stopBit":1|2,"dataBits":7|8,"isActive":true}}`. All five required, else fetch FAILED. PRD §6.1 value is valid.

### 5.3 `lw_configGroups` (`04-Konfigurationen.md:53-127`; template `LW-ConfigGroups-1-16-V02`, msgs 2200-2215 = 16 groups, record `lw_configGroups[0..15]`)
Array, one object per group, **max 16** (`MAX_GROUP_NUM 16`, `LSY_light_system_data.h:43`); an empty record is accepted (`:2630`, so fewer than 16 is fine):
```json
{ "id": 1, "name": "Shop", "minLevel": 0, "maxLevel": 100, "stdLevel": 100, "dayState": false,
  "dynamicControl": false, "luxThreshold": 4500, "slaveOf": null, "automaticSwitch": false, "channel": 1,
  "scheduler": [ {"off":"22:00","on":"06:00"}, ... 7 entries, Sunday first ... ],
  "converters": [1, 2, 3] }
```
`id` [1,16] required; `minLevel`/`maxLevel` [0,100]; `luxThreshold` [0,200000]; `converters` = lamp numbers [1,64] (the NN of `LW Dali Control Gear 1-NN`); `slaveOf` group id or empty; `channel` ignored; `stdLevel` unused; `scheduler` times local to `timeZone`, empty strings = not configured. The lamp->group assignment is how lamps become "valid" for measurement sending (section 1.2).

### 5.4 `lw_configDigitalInputs` (`04-Konfigurationen.md:193-237`; template pattern `id,isActive,type,highLevelAction,lowLevelAction,initCount,increment,unit,groups[*]`)
```json
[ {"id":1,"isActive":true,"title":"Zapfsäule 1","type":"daliAction","highLevelAction":"on","lowLevelAction":"off","groups":[10,2,3]},
  {"id":2,"isActive":true,"title":"Tür offen","type":"measurement"},
  {"id":3,"isActive":true,"title":"Gaszähler","type":"impulseCounter","initCount":1000,"increment":10,"unit":"pulse"} ]
```
`id` [1,10]; `type` one of `daliAction` | `impulseCounter` | `measurement` (mandatory since 01.08.00, `CHANGELOG.md:29`; `none` rejected); `initCount` [0,4294967295]; `increment` [1,65535]; `unit` max 19 chars (longer is truncated). **There is no `title` field** in the firmware template; `unit` is only meaningful for `impulseCounter`. Decision 2026-10-12 (DECISIONS.md D1): the simulator adds a `title` field to each `lw_configDigitalInputs[]` entry; the firmware parser only reads the template fields, so the extra field is ignored (harmless, but UNVERIFIED for dashboards).

### 5.5 `lw_configModbusSlaves` (`04-Konfigurationen.md:389-495`; templates `LW-ConfigModbusSlaves-0-1..18-19-V01`, base `lw_configModbusSlaves[i]` for i = 0..19; reset registers `LW-ConfigModbusResetRegisters-V01`)
Array **position is irrelevant**; the slave goes into slot `id-1` (`C8Y_str_parser_getter.c:959`), empty records are skipped (`:340-362`). Max 20 slaves.
```json
{ "id": 20, "address": 240, "name": "Kuehlung", "requestInterval": 30, "isSavingOnStorage": false, "isActive": true,
  "resetRegisters": [ {"name":"Reset Current Max","interval":1800,"address":1000,"type":"holding","value":0} ],
  "registers": [
    {"name":"Voltage L1","unit":"V","address":30001,"type":"input","datatype":"float32","scalingFactor":1.0,"swap":"word_byte","changeOfValue":0.5,"conversion":"","custom":""} ] }
```
- Slave: `id` 1-20 (required), `address` 1-247, `name` 1-19 chars (required), `requestInterval` 1-99999 s, `isSavingOnStorage` must be `false` (else FAILED `isSavingOnStorage incorrect - no USB support`), `isActive`. Max 40 registers/slave, <=5 reset registers.
- Register: `name` <=19 chars (series name, required), `unit` <=7 chars, `address` 0-65535 (raw, 0-based), `type` `holding|input|discrete|coil`, `datatype` `uint8|uint16|uint32|int8|int16|int32|int64|float32|float24s|float24u|pf32|bool` (`bool` only for coil/discrete; empty = bool), `scalingFactor` -1e6..1e6 (0/empty = off), `swap` `none|byte|word|word_byte` (`word`/`word_byte` invalid for 16-bit types), `changeOfValue` [0,1e6] (CoV send threshold), `conversion`/`custom` <=14 chars (set -> JSON path, avoid).
- **`description` and `writeRegisters[]` are NOT in the firmware templates** (the patterns list only `id,address,name,requestInterval,isSavingOnStorage,isActive,registers[*].{name,unit,address,type,datatype,scalingFactor,swap,changeOfValue,conversion,custom}`), so the firmware ignores them. They are consumed by the floorplan plugin (`writeRegisters` entries `{name,address,type:"coil"|"holding"}`, see floorplan-plugin.md section 4). PRD `description: "lw-demo"` on a slave is harmless to the firmware but UNVERIFIED for any other consumer.
- Register address note: the cloud config uses the raw 0-based address; the plugin example uses 40001 (a 4xxxx reference style) in write ops, which the firmware accepts as a plain number <=65535.

### 5.6 Analog inputs (info only, not on `devel`; still a v1 non-goal)

Analog inputs exist on branch `origin/feature_analog_input` (commit `2c6e6881`, 2026-09-29): measurement line `201,analogInput,,LW Analog Input,Value%02u,%.2f,V` (`C8Y_str_parser_getter.c:4848` there; type `analogInput`, fragment `LW Analog Input`, series `Value01..Value04` from the channel number, volts with 2 decimals, fixed unit `V`, doc `02-Messwerte.md:73-83` there), config fragment `lw_configAnalogInputs` (`C8Y_ParseForConfigAnalogInputs`, `:1528`), operation `lw_fetchAnalogInputs` (in `CUMULOCITY_SUPPORTED_OP`, `C8Y_smart_rest.h:49` there) and `lw_configCore.analogInputsMsgPerMinute`. Naming therefore follows the digital-input pattern (no titles in series names). Out of scope for v1; PRD §19 keeps the open point.

## Posting via REST instead of SmartREST

The simulator talks REST JSON to Cumulocity (not SmartREST over MQTT). Rules so the result looks like the firmware's data and works with the floorplan plugin:

- **Always send `time`** (now, ISO 8601 with offset/`Z`) on measurements, alarms and events. The firmware leaves it empty and the tenant stamps receive time; over REST a missing `time` is rejected. The floorplan plugin keeps the newest entry **by `time`** (`davaco-demo .../floorplan-live-data.service.ts:~479-488`, `putMeasurement`), so a wrong or old `time` makes a value invisible.
- **Values must be JSON numbers** (not strings): the plugin ignores a non-number (`typeof value !== "number"`, same place).
- **One SmartREST `201` line = one measurement object**: `{ "type": <type>, "time": <ISO>, "source": {"id": "<deviceId>"}, "<fragment>": { "<series>": { "value": <number>, "unit": "<unit>" } } }`. The DALI lamp line (4 series with one fragment) becomes one measurement with four series under the same fragment `LW Dali Control Gear 1-NN`.
- **`210`** -> `{ "type": "c8y_SignalStrength", "c8y_SignalStrength": { "rssi": { "value": <int>, "unit": "dBm" } } }`.
- **`211`** -> `{ "type": "c8y_TemperatureMeasurement", "c8y_TemperatureMeasurement": { "T": { "value": <n>, "unit": "C" } } }`. Unit `"C"` is the Cumulocity convention; **UNVERIFIED locally** (the static template is not in the repo). Decision: use `"C"`.
- **Batch**: `POST /measurement/measurements` with `{ "measurements": [ ... ] }` and content type `application/vnd.com.nsn.cumulocity.measurementcollection+json` (single: `application/vnd.com.nsn.cumulocity.measurement+json`).
- **Empty-unit series** (`Converter Status`, `Lamp Status`, motion `StateN`): the firmware sends an empty unit field. Decision: **omit `unit`** in the JSON (recommended over `""`).
- **Operation status**: `PUT /devicecontrol/operations/{id}` with `{ "status": "EXECUTING" | "SUCCESSFUL" | "FAILED", "failureReason": "<text>" }` (`failureReason` only for FAILED). Replaces SmartREST `504` / `506` / `505`.
- **Alarm de-duplication**: Cumulocity de-duplicates ACTIVE alarms by `source` + `type` (the existing alarm's `count` is incremented instead of a new alarm being created), as the SmartREST path does for the firmware. Alarm body: `{ "type", "text", "severity": "CRITICAL|MAJOR|MINOR|WARNING", "status": "ACTIVE", "time", "source": {"id"} }`; clearing = `PUT /alarm/alarms/{id}` `{ "status": "CLEARED" }`.

## 6. Quick facts for the simulator

- Max slaves 20, registers 40, reg name 19 / unit 7 / slave name 19 chars (`MBM_modbus_master.h:30-34`; doc `04:448,468-469`). PRD limits match. Item id (=register name) <= 19 matches.
- `NO_DIN` digital inputs = 10, brightness sensors 10, motion sensors 10, groups 16, lamps 64.
- DALI group mode mapping: `on` = max level (group `maxLevel`), `off` = 0, `auto` = schedule/lux logic.
- Alarm blacklist cannot suppress anything the simulator invents; it only filters real firmware identifiers.
