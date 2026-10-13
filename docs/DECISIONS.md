# Product decisions

Decisions that change or extend `PRD.md`. Newest first. Affected PRD sections are marked inline with
"(changed 2026-10-12, see DECISIONS.md)". Evidence lives in `docs/research/`.

## 2026-10-13

### D10. Scenarios are editable per device; "Reset to defaults"

User feedback: scenarios should be editable, new ones creatable, with a description, and one button should set
everything back so scenarios are easy to undo. Decision:

- A library scenario gets an optional `description {de, en}` (schema + serde); the shipped demos carry one. The
  Control tab shows it on the scenario button.
- Device edits live in `lw_demo`: **`customScenarios[]`** (full `Scenario`s; one with the id of a library scenario
  replaces it in place, the others follow the library's) and **`hiddenScenarios[]`** (ids of deleted library
  scenarios). `update_demo_setup` carries both (optional fields). The Setup tab's scenario editor plans value,
  generator and fault steps (raise / clear) and saves the whole list as that patch.
- Validation on save (`validate::validate_device_scenarios`): the library's step rules against the device's
  selected items (library + custom), lamp count, slaves and fault catalog (item selected and not a custom alarm,
  value in the control range / option / counter ≥ start, generator fits the item and its band, fault target
  valid, no value hidden behind its own slave timeout), ids `[A-Za-z0-9_-]` and unique, a name in de and en, and
  **one start scenario** among all scenarios of the device. The library's "the start scenario resets everything
  the others change" rule is **not** enforced on a device; the export reports it as a warning (the exported pack
  must pass the library validation once released).
- Scenarios are checked only when they are edited; a later selection change does not block other setup edits (a
  step on an item that is gone fails when the scenario is applied).
- A running session takes edited scenarios at once (`SetupChanged`); no restart needed.
- Applying a **different** demo drops the scenario edits (like generator overrides); re-applying the same demo
  and upgrades keep them. Export writes the merged list.
- **"Reset to defaults"** (`reset_to_defaults`, Control tab, with a confirmation): "Clear all" (D9), then every
  item back to its default value (control default; lamp 100, else 0) and its effective generator
  (`lw_demo.generators` override, else the library's), manual overrides dropped, changed values sent at once.
  **Counters keep their value** (they never go backwards). DALI group modes and lamps without an item are not
  touched.

## 2026-10-12

### D1. Digital inputs use native firmware series names

- Series are `Value<NN>` and `ImpulseCounter<NN>` (NN = 01-10), fragment `LW Digital Input`, type `digitalInput`.
  This is what the firmware emits on every branch (`research/firmware-core-v2.md` section 1.4).
- The human title (e.g. "Zapfsäule 1") is stored in `lw_configDigitalInputs[].title` (new field; the firmware
  parser ignores extra fields) and in the library item/demo.
- The app shows the title as the selectable label. When a floorplan is applied, the widget label uses the title
  from the config, falling back to the input number if there is no title.
- No title cleanup into series names any more (the old umlaut/length rules are dropped). The "DI title cleanup"
  unit test is replaced by a DI config/title mapping test.

### D2. Multiple custom alarms per slave, plus device-level custom alarms

- A slave definition carries `alarms[]` (any number of custom alarms).
- New item kind `customAlarm` for a device-level alarm that is not bound to a register or slave.
- Type rules: slave alarm `lw_demo_<id:02>_<name>` (`id` = assigned slave id, `name` = alarm name = item id);
  device-level alarm `lw_demo_<name>`. A device-level `name` must not start with two digits plus `_`, so the
  two forms cannot collide.
- Alarm types are written literally into floorplan `alarmTypes` (the plugin never suggests custom types).

### D3. The prototype's 10 binary items become the 10 native digital inputs

Not Modbus registers. They use `type: "measurement"`, series `Value01`..`Value10`, titles per D1.

### D4. New client tag and demo `restaurant`

The library ships three demos: `gas-station`, `retail`, `restaurant` (`library/demos/restaurant.json`). The
prototype's davaco items are the `restaurant` demo. Tag rule and counts: `research/prototype-inventory.md` section 3.

### D5. Decisions taken from the research review

- **`lw_configCore`** is always written with the full 13-field object (the firmware fails `lw_fetchCore` on any
  missing field except `timeZone`/`ntpServer`), not just `measurementIntervalMinutes`.
- **Firmware version** stays `"V 03.00.02"` (the real firmware sends the `V ` too; it must stay parseable as
  `[V ]NN.NN.NN` or the floorplan's Modbus controls hide).
- **Temperature unit** for `c8y_TemperatureMeasurement.T` is `"C"` (Cumulocity convention, unverified locally).
- **Always send `time`** (now, ISO 8601) on measurements, alarms and events; the plugin picks the newest by `time`.
  Series with an empty firmware unit omit `unit`.
- **`sendIntervalSec` max is 899** (stale threshold = `measurementIntervalMinutes` + 5 min = 900 s).
- **Operations the plugin already set FAILED are ignored** by the simulator (10 s PENDING cancel, supersede rule).
- **Writable Modbus items** go into both `registers[]` and `writeRegisters[]` of the slave.
- **`lw_configCore.maxControlGears`** is `max(55, daliLampCount)`: 55 is the default, a device with more lamps must
  not exceed its own limit (PRD §6.1).
- **`lw_modbusWrite` readback** echoes exactly the operation's `value` (the plugin compares raw values with `===`).

### D6. Device-local slaves, items, DI titles and group edits live on the device

The demo editor (PRD §7.4 "created in the app's demo editor", §11.3 "generate series" helper) and the export
round trip (§13) need setup that is not in the bundled library. Decision (foundation review, finding 17):

- `lw_demo` gets **`customSlaves[]`** (`SlaveDefinition`) and **`customItems[]`** (`LibraryItem`). They are merged
  with the library's slaves and items when a session or an apply resolves a demo. Custom ids must not collide with
  library ids; custom Modbus items get register addresses like library ones.
- `lw_demo.slaves[]` stores, per slave, the assigned `id` / `address` **and** the assigned register `address` +
  `registerType` for each of its registers. Assignments are never renumbered (upgrades keep them): new slaves take
  the next free id (down from 20) and address (down from 240); new registers take the lowest free address of their
  type (`coil` from 1, `input` from 30001, `holding` from 40001).
- Modbus items carry `registerType` (`coil` | `holding` | `input`; default `holding`, `coil` for a switch control;
  only coil / holding can be writable). `writeRegisters[]` entries are `{name, address, type: coil | holding}`; the
  matching `lw_modbusWrite` is identified by `slaveAddress` + `registerAddress` + `type` (`coil` | `register`).
- `lw_configGroups` and `lw_configDigitalInputs` (incl. the D1 `title`) are edited directly on the device. New
  commands: `get_device_config`, `set_dali_groups`, `set_digital_inputs`; `update_demo_setup` carries
  `customSlaves` / `customItems`; `generate_item_series` is the "generate series" helper (pure).
- A demo's `digitalInputs[]` references the item by `itemId` (single source of truth for the title: the item's
  German title is written to `lw_configDigitalInputs[].title`). DI settings follow the firmware config
  (`daliAction` with `highLevelAction` / `lowLevelAction` / `groups`, `impulseCounter` with `initCount` /
  `increment` / `unit`, `measurement`; all with `isActive`).
- Export (PRD §13) reads `lw_demo` (incl. custom slaves / items), `lw_configGroups` and `lw_configDigitalInputs`,
  so a hand-tuned device round-trips into the library format.
- **Register addresses are a simulator convention** (domain review, finding 29). The assigned numbers
  (`coil` 1…, `input` 30001…, `holding` 40001…) are stored as the *raw* firmware address (the firmware takes a
  0-based PDU address 0-65535), and consecutive `float32` / `*32` registers are **not** spaced by their word count,
  so they overlap on a real bus. This is harmless for the simulator (no bus) and consistent across `registers[]`,
  `writeRegisters[]`, the plugin `ref` and `lw_modbusWrite`; a real Core given this config would read wrong words.
  Writable holding registers are one 16-bit word (FC6) and default to `uint16`.
- **Digital input source of truth at runtime** (finding 2): the device's `lw_configDigitalInputs` entry decides the
  series (`Value<NN>` vs `ImpulseCounter<NN>`) and the counter unit (`payload::naming::item_naming_with_di`), as the
  firmware does. In the library the item and the demo's DI entry must agree (`unit`, `control.default` ==
  `initCount`); validation enforces it.

### D7. Lock and snapshot live in their own fragments, not inside `lw_demo`

PRD §6.3 first put `lock` and `snapshot` inside `lw_demo`. Final integration review, finding 10. Decision:

- The advisory lock is the top-level fragment **`lw_demoLock`** (`{user, since}`) and the saved live values are
  **`lw_demoSnapshot`** (`{savedAt, values}`). `lw_demo` carries neither (`LwDemoState` still holds them in memory;
  `demo_state::read` merges them in, and writes go to the own fragment).
- Reason: Cumulocity replaces a top-level fragment as a whole. A running session refreshes the lock every 3 minutes
  and "Save snapshot" writes values; if both lived in `lw_demo`, every refresh would rewrite (and could revert) the
  setup the presenter edits at the same time. With separate fragments no write of one touches the others.
- A write of the setup (`demo_state::patch_body`) sends `lw_demo` without lock and snapshot, plus the snapshot
  fragment as it was read; the lock is only ever written by `lock_body`, so a setup edit cannot clear or revive it.
- Compatibility: a lock or snapshot written by an older build inside `lw_demo` is still read (`lw_demo.lock` /
  `lw_demo.snapshot`); the next setup write moves the snapshot to its fragment and drops both from `lw_demo`.
- Anyone reading a device or exporting by hand (AGENTS.md "Exporting a device by hand") looks in `lw_demoLock` and
  `lw_demoSnapshot`; the export ignores them (a lock is not part of a demo, a snapshot is only values).

### D8. A DALI bus power fault takes every lamp and sensor on the bus with it

Field report: raising `lw_daliBusPowerFail` turned only the bus alarm on; the floorplan lamps stayed green.
Firmware evidence: `docs/research/firmware-core-v2.md` §2.1. Decision:

- Any of `lw_daliBusPowerOff` / `Fail` / `Wrong` (all mean "no bus voltage"; there is no separate short or
  overload alarm) raises, per lamp in a group, `lw_converter_1_NN_lost` + `lw_controlGearFailure_1_NN` and, per
  brightness / motion sensor item, its `_lost` alarm; those lamps / sensors stop sending. `lw_powerFailure`,
  `lw_converterExceed` and `lw_ntpSyncFailed` have no cascade.
- **Deviation: no delay.** The firmware needs a few minutes (120 s gear-check interval, 3-check debounce) before
  the lamps are lost, and the recovery takes a few minutes too. The simulator raises and clears the cascade
  together with the bus alarm, so the floorplan reacts during a demo. Order is kept: bus alarm first on raise,
  bus "Alarm cleared" first on recovery.
- The cascade alarms are not faults (not in the fault menu); they are owned by the bus fault
  (`SessionCore::bus_cascade`). A lamp / sensor fault of its own with the same type is not raised twice, and on bus
  recovery its alarm stays open until that fault is cleared; clearing it while the bus is down leaves the alarm
  to the bus. Recovery clears like every other fault: `lw_alarm` "Alarm cleared" event, plus CLEARED unless
  realistic mode. (Changed by D9: faults are fired once and end when their alarm is cleared in Cumulocity; the
  cascade recovery is unchanged, a lamp's own fault cleared while the bus is down is cleared in Cumulocity.)
- **Deviation:** other lamp faults (`lw_lampFailure_*`, `lw_converterFailure_*`) stay active during a bus failure;
  the firmware would clear them while the gear is absent. They are the presenter's choice, and the lamp sends
  nothing anyway; on recovery the lamp comes back with them.
- Session start: lamp / sensor lost alarms open next to an open bus power alarm are restored as the bus's, so they
  recover with it instead of becoming lamp faults.
- A lamp's own `lw_converter_1_NN_lost` fault also holds `lw_controlGearFailure_1_NN` (firmware
  `LSY_IsGearGenericFailureActive` counts `ERR_CONTROL_GEAR_LOST`); it is derived in the same place and owned like
  the bus cascade (`bus_cascade`), so it is cleared when the last cause (own fault or bus) is gone. `lw_lampFailure_*`
  does not imply it (not in the firmware research).
- Post order: all `_lost` alarms (lamps, then sensors) before the `controlGearFailure` ones, on raise and on
  recovery (the outbound normal lane is strictly ordered, so the floorplan turns red / green sooner).
- A cascade alarm whose POST failed is released and raised again by the next sync (bus fault change, group
  change, session start); no "Alarm cleared" is sent for an alarm that was never created.
- Realistic mode: at session start the "Alarm cleared" events are read since the oldest open alarm (not the newest
  200), so a busy recovery cannot hide older recoveries.
- A fault raised while its alarm is open already (held by the cascade) takes over that alarm's id.
- `c8y_SupportedOperations`: the app re-adds missing required operations (and the agent fragment) when a device is
  opened (best effort, a failing PUT is only logged), applied or started (mandatory). Removing one on purpose does
  not stick; the app's catalog is the source of truth.
- Not modelled: DALI actions during a bus failure. The firmware answers `lw_switchGroupMode` SUCCESSFUL regardless
  (it sets the group mode in its own state; libdali merely refuses the bus command, research §4), so the simulator
  keeps succeeding and sending `lw_daliGroupChange`. The short `Off` before `Fail`, DALI temperature sensors (`lw_temperatureSensor_1_NN_lost`; the
  simulator has none), group failure handling.

### D9. Alarms are fired once and end when they are cleared in Cumulocity

Product owner request: the presenter shows on the cloud how an alarm is acknowledged and cleared. Before, every
fault was a switch (raise / clear in the app). Decision:

- **Fire once.** Every fault (alarms panel: DALI bus, slave timeout and slave alarms, custom alarms; item cards:
  lamp and sensor faults) is a button that posts its alarm (`fire_fault`). The effect (D8 cascade, slave values
  stop, lamp status, sensor lost) starts at once and **lasts until the alarm is CLEARED in Cumulocity**. There is
  no per-fault clear in the app.
- **Firing an open fault again** re-posts the alarm; Cumulocity de-duplicates it (same id, `count` + 1, the status
  stays). The button stays enabled; a status badge shows *Aktiv* / *Quittiert* (`ActiveFault.status`,
  `OpenAlarmStatus`).
- **The session follows Cumulocity.** The realtime client subscribes `/alarms/<deviceId>` next to
  `/operations/<deviceId>` on the same Bayeux client. `CLEARED` (also a deleted alarm) for the alarm of a fault ends
  the fault: the firmware's `lw_alarm` "Alarm cleared: <type>" event is sent, values resume, nothing is posted
  again. `ACKNOWLEDGED` (or back to `ACTIVE`) only changes the status (`fault-changed`). Alarms are matched by id;
  alarms the simulator does not track are ignored.
- **Re-sync.** On every (re)connect, after subscribing, the realtime task fetches the open alarms
  (ACTIVE + ACKNOWLEDGED) and delivers them with the instant taken before the request (`RealtimeMsg::OpenAlarms`).
  An alarm the session knew before that instant and that is missing was cleared while nobody listened: its effect
  ends. Alarms whose id the session learned later are left alone (no race with a POST in flight). A clear that
  arrives before the alarm's create answer is remembered (last 256 ids) and applied when the id arrives; so is an
  ACKNOWLEDGED. While a create is in flight (also a re-post of a known alarm: `AlarmTrack.inflight` counts them)
  a clear or a re-sync does not end the fault: after a clear Cumulocity creates a **new** alarm for the re-post,
  and the answer decides (same id as the cleared one: the fault ends; new id: it goes on with it).
- **A failed create** (`POST /alarm/alarms` fails and no alarm id is known) ends the fault again without the
  "Alarm cleared" event and reports the error: no alarm exists, so nobody could clear it in Cumulocity.
- **Optional alarm access.** The alarm subscription and the open-alarm list are optional in the realtime phase:
  403 on either is logged and alarm changes are not followed (operations keep working); the list is retried once on
  other errors, and `OpenAlarms` is only delivered when it succeeded. 401 still ends the phase.
- **"Clear all"** (`clear_all_alarms`, alarms panel, with a confirmation): every open alarm of the device is
  CLEARED, also ones the simulator did not raise (item threshold alarms, raw fire, other tools). The open alarms are
  listed, then cleared with one bulk update per status (`PUT /alarm/alarms?source=<id>&status=ACTIVE`, then
  `…&status=ACKNOWLEDGED`, body `{"status":"CLEARED"}`). After a `200` the list is read once more and what is
  still open is cleared one `PUT` at a time; after a `202 Accepted` (Cumulocity continues in the background) or a
  failed bulk update every listed alarm is cleared one `PUT` at a time at once (idempotent with the background
  job; no polling, which would hold up the ordered outbound lane). The background job clears whatever is open
  when it runs, so an alarm fired right after "Clear all" can be caught by it; accepted. With a session on the device it goes through the session (all
  effects end, the bulk clear is queued behind the alarm posts before it; the toast says the clear was queued).
  If the clear fails the session reports the error, reads the open alarms and takes the ones that are still
  open over again as effects (`cleared_by_all`, `OutboundPayload::ReadOpenAlarms`); without one it is a plain REST call
  (the UI offers it in the alarms panel, i.e. with a running session; the Setup tab's device reset still clears
  alarms too). `reset_demo_device` uses the same bulk clear.
- **DALI bus (D8).** Clearing the bus power alarm in Cumulocity ends the bus failure; the simulator then recovers
  the cascade itself (events, CLEARED unless realistic mode) and the lamps / sensors send again. A **cascade alarm
  cleared by the presenter while the bus is still open** stays cleared: the lamp stays lost and the alarm is not
  raised again (also not by a group change) until the bus recovers; then only the "Alarm cleared" event follows.
  A **lamp's own fault cleared while the bus holds the lamp** ends the fault; the lamp stays lost until the bus
  recovers. Firing a lamp fault whose alarm the bus holds open takes that alarm over (as in D8).
- **Item threshold alarms** (`effects: alarm`) still follow their value. One cleared in Cumulocity (or by "Clear
  all") while the value is above the threshold is not raised again until the value has been below it once.
- **Realistic mode** now only concerns alarms the simulator recovers itself (bus cascade, threshold alarms): it
  sends the "Alarm cleared" events and leaves those alarms open. Fault alarms are cleared by the presenter.
- **Scenarios** keep the library format. A `fault` step with `active: true` fires (again, if open); `active:
  false` clears that alarm in Cumulocity (`PUT` CLEARED, also in realistic mode) and ends the effect like a cloud
  clear. The **start scenario applied by hand** is "Clear all" plus its steps. At **session start** it runs its
  steps only (no blanket clear): open alarms are restored first as effects with their status, and the start
  scenario's own `active: false` steps clear the ones it lists (validation still asks the start scenario to clear
  every fault another scenario raises, for exactly this). Open alarms it does not list stay restored. The start
  scenario button clears all alarms **without** the confirmation "Clear all" asks for; intentional (it is the
  reset button of the demo, and its label already says "Start").
- The raw fire panel is unchanged.
