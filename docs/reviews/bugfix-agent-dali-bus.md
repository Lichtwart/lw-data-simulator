# Review: agent fragment migration (bug 1) and DALI bus power cascade (bug 2)

Scope: `payload/{config,alarm}.rs`, `device/{crud,apply,tests}.rs`, `sim/{session,faults,core,tests}.rs`,
`c8y/fake.rs`, `docs/{PRD,ARCHITECTURE,DECISIONS}.md`, `docs/research/firmware-core-v2.md` §2.1.
`cargo test`: 263 passed, 0 failed.

## Summary

Both fixes are correct in their main paths. I traced these cases through `sync_bus_cascade` and found no
defect:

- the bus fault switched on and off repeatedly, including before Cumulocity has returned alarm ids
  (`clear_pending` and the strictly ordered normal lane cover this);
- several bus power types active at once (`bus_down` is an "any of them" check);
- a lamp or sensor fault raised or cleared while the bus is down;
- a group change during a bus failure (`config_changed` calls `sync`);
- a session stopped and restarted during a failure (`restore_alarms` then `sync`);
- a scenario that sets a `daliBus` fault (it goes through `set_fault`);
- recovery in realistic mode (events only, no CLEARED).

`emit_changed` is set on every cascade change, and `paused` covers lost lamps and sensors, so the
"Session-visible state" rule is met. Floorplan lamp and lux widgets list `_lost` and
`controlGearFailure` through `autoAlarms` (`payload/floorplan.rs:74-76`; no shipped floorplan sets
`autoAlarms: false`), so the lamps do turn red.

The findings below are about edge cases, robustness and test gaps. Nothing is high severity.

## Findings

### 1. Medium: opening a device now needs write access (`crud::get` sends a PUT)

`src-tauri/src/device/crud.rs:208-214` (`get` calls `ensure_required_fragments`, lines 195-206).

- **Failure scenario:** a user who can only read the inventory opens a demo device that an older app
  version created. The PUT returns 403. `get` passes the error up with `?`, so `get_device`
  (`commands/devices.rs:40`) and the command at `devices.rs:232` fail. The device cannot be opened at
  all, although opening worked before this change. The same failure hits any transient PUT error,
  such as a 5xx or a timeout. Every open also writes an inventory PUT entry into the activity log.
- **Not a problem:**
  - Races with a session running on another machine: the PUT carries only the three fragments,
    Cumulocity merges top-level fragments, and the operation list is a union. Nothing is overwritten.
  - Non-demo devices: they are refused by `ensure_demo` before any write.
- **Suggested fix:** make the migration in `get` best effort. Log the error (`log::warn!`), keep going
  with the unmigrated object, and skip the local merge. Keep it a hard requirement only in
  `session::prepare` (`sim/session.rs:459-461`) and in the apply PUT, because both need write access
  anyway. Add a fake test where the PUT returns 403 and `get` still succeeds.

### 2. Low: migration re-adds operations someone removed on purpose

`src-tauri/src/payload/config.rs:166-191`.

- **Failure scenario:** someone deliberately removes an entry from `c8y_SupportedOperations`, for
  example to hide a menu item for a demo. The next open, apply or session start puts it back. This is
  probably intended (the catalog is the source of truth), but nothing documents it.
- **Suggested fix:** add one sentence to PRD §6.1, or none if this behaviour is wanted.

### 3. Low: a lamp's own `_lost` fault does not raise `lw_controlGearFailure_1_NN`, so recovery is inconsistent

`src-tauri/src/sim/faults.rs:229-251` and `:162-174`.

- **Failure scenario:**
  - Firmware §2.1: `LSY_IsGearGenericFailureActive` counts `ERR_CONTROL_GEAR_LOST`. So a lost gear
    always also has the generic control gear failure.
  - The presenter raises `lw_converter_1_03_lost` as a lamp fault. Only the lost alarm is raised.
  - If the bus then fails and recovers, the cascade clears `lw_controlGearFailure_1_03`, but
    `lw_converter_1_03_lost` stays open because the lamp fault still owns it. The firmware would keep
    the control gear failure while the lost alarm is active.
- **Demo impact:** none on the floorplan. The lamp is red through `_lost`, which is in `autoAlarms`.
  Only the Cumulocity alarm list differs from a real Core. Acceptable for the demo.
- **Suggested fix:** either derive `lw_controlGearFailure_1_NN` from any active `_lost` (own fault or
  bus) in one place, using the same owner-set idea as `bus_cascade`, or list it as "not modelled" in D8.

### 4. Low: after a restart during a bus failure, a lamp fault raised during that failure becomes the bus's

`src-tauri/src/sim/faults.rs:296-324`.

- **Failure scenario:** while the bus is down, the presenter raises `lw_controlGearFailure_1_03` or
  `lw_converter_1_03_lost` as a lamp fault. No POST is sent, because the alarm is already open. The
  session stops and restarts. The open alarm sits next to the open bus alarm, so it is restored as
  bus-owned. On bus recovery it is cleared, and the presenter's lamp fault has disappeared from the
  fault menu.
- D8 documents this trade-off. It cannot be decided from Cumulocity state alone, because there is
  only one alarm. Acceptable. Mentioned for completeness.

### 5. Low: the floorplan reacts slowly to raise and recovery on large demos

`src-tauri/src/sim/faults.rs:154-174`; normal lane in `sim/outbound.rs` (strictly in order).

- **Failure scenario:** on the gas-station demo (22 lamps, all grouped, plus `lux-1`), one bus toggle
  queues:
  - on raise: 1 + 44 + 1 alarm POSTs;
  - on recovery: about 46 `lw_alarm` events plus 46 CLEARED PUTs.

  All of these go one after another on the normal lane. At 100-200 ms per request:
  - on raise, lamps turn red one by one over about 5-10 s;
  - on recovery, a lamp turns green only after its lost and gear-failure clears (4 requests each),
    and the whole floorplan takes 10-20 s.

  This works, but it is noticeable in a live demo.
- **Suggested fix (optional):** order the cascade so that all `_lost` alarms come first (they alone
  turn a widget red), then the `controlGearFailure` alarms. Do the same on recovery for the clears.
  This roughly halves the time until the floorplan has fully reacted. Alternatively, accept it and add
  a note to D8.

### 6. Low: realistic mode and the 200-event window used to drop recovered alarms

`src-tauri/src/sim/session.rs:234-238` (`query_events(..., "lw_alarm", 200)`) and `drop_recovered`.

- **Failure scenario:** in realistic mode the alarms stay ACTIVE in Cumulocity. A restart works out
  which alarms have recovered from the newest 200 `lw_alarm` events. One gas-station bus recovery
  writes about 46 of them. After roughly four bus toggles, the "Alarm cleared" events of older
  recoveries fall out of the window. Those alarms, such as an earlier slave timeout, come back as
  active faults at the next session start.
- Realistic mode also leaves about 46 alarms per bus failure for the presenter to clear by hand. That
  is by design, but painful.
- **Suggested fix:** query `lw_alarm` events newer than the oldest open alarm's `time` instead of a
  fixed count, or raise the count. Optionally mention the manual clean-up cost in the realistic-mode
  copy.

### 7. Low: a cascade alarm whose POST failed is never retried, and its recovery event is still sent

`src-tauri/src/sim/faults.rs:155-159`, `sim/core.rs:1270-1278` and `:947-967`.

- **Failure scenario:** a cascade alarm POST fails (5xx or network). `outbound_result` sets
  `active = false`, but the type stays in `bus_cascade`. Later `sync` calls skip it, because
  `insert` returns false. That lamp stays green on the floorplan for the rest of the bus failure. On
  recovery, `clear_alarm` still sends "Alarm cleared: …" for an alarm that never existed.
- **Suggested fix:** in `sync_bus_cascade`, raise every wanted type whose track is not active and not
  in flight, not only the newly inserted ones. Or remove the type from `bus_cascade` in the Err branch
  of `outbound_result`. Skip the "Alarm cleared" event when the track was never active.

### 8. Low: DALI actions still succeed while the bus is down

`sim/core.rs` `set_group_mode` and the operations `lw_switchGroupMode`, `lw_switchConverterState`,
`lw_daliScan`, `lw_daliReset`.

- **Failure scenario:** firmware §2.1 says libdali refuses every action while bus power is not ON.
  During a simulated bus failure, a floorplan group control or a digital-input `daliAction` still
  switches the group and sends `lw_daliGroupChange`, and the operation turns SUCCESSFUL. The lamps
  send nothing, so the effect only shows after recovery. A presenter who demonstrates "switch the
  group while the bus is dead" sees success.
- **Suggested fix:** either list it under "Not modelled" in D8, or fail the DALI operations with a
  firmware-like reason while `bus_down()`. The firmware text needs checking in the research doc first.

### 9. Low: test gaps (the existing tests do fail without the fix)

`src-tauri/src/sim/tests.rs:787-998`; `src-tauri/src/device/tests.rs:90-150`.

- **The existing tests are good:**
  - Without the cascade, `bus_power_failure_loses_every_lamp_and_recovers` fails on the alarm count
    and on the measurement suppression.
  - Without the agent fragment, the device test fails on the creation body and on the fake's 422.
  - The idempotence of the migration and the refusal of real (non-demo) devices are covered.
- **Not covered:**
  1. Two bus power types at once: clear one, the cascade stays; clear the other, it recovers.
  2. `ConfigChanged` with groups during a failure: a lamp joins and is raised, a lamp leaves and is
     cleared.
  3. An ungrouped lamp is excluded. In the fixture every lamp is grouped, so the `lamp_grouped`
     filter is never exercised.
  4. A sensor's own `_lost` fault surviving bus recovery.
  5. Bus on, then off, before the POSTs are answered: clears happen after the ids arrive.
  6. The snapshot with `emit_changed` after recovery: `paused` is false again.
  7. `crud::get` when the migration PUT fails (see finding 1).
- **Suggested fix:** add (1)-(3) as paused-time tests. They are small extensions of the existing rig.

### 10. Low: `ActiveFault.alarm_id` stays `None` when the alarm is already held

`src-tauri/src/sim/faults.rs:233-244`.

- **Failure scenario:** a lamp fault raised during a bus failure sends no POST. Its `alarm_id` is
  therefore never filled in, although `self.alarms[type].id` is known. The UI does not read `alarmId`
  today (only tests do), so nothing breaks yet.
- **Suggested fix:** copy the track's id into the new `ActiveFault` when no POST is sent.

## Resolution

1. Fixed. `crud::get` logs a warning (`log::warn!`) and opens the device unmigrated when the migration PUT fails;
   `ensure_required_fragments` still errors, so session start and apply stay strict. Test
   `opening_a_device_survives_a_failing_migration` (403 on the PUT).
2. Documented in DECISIONS D8 (missing required operations are re-added; catalog is the source of truth). No code change.
3. Fixed for `_lost`: a lamp's own `lw_converter_1_NN_lost` fault now also raises `lw_controlGearFailure_1_NN`.
   Both causes (bus, own fault) are one derived set in `sync_bus_cascade`, which is now also called for lamp faults;
   the alarm is cleared when the last cause is gone. Lamp / sensor refresh is limited to the lamps whose alarms
   changed. `lw_lampFailure_*` is not covered: the firmware research only names `ERR_CONTROL_GEAR_LOST` for
   `LSY_IsGearGenericFailureActive`. Tests: the two existing bus tests were adjusted and extended.
4. Already accepted in D8 (one alarm cannot tell whose it is after a restart). No change.
5. Reordered: `_lost` alarms (lamps, then sensors) are queued before all `controlGearFailure` ones, on raise and on
   recovery. Not made concurrent: the normal lane is strictly ordered on purpose (ids before clears). Test
   `cascade_posts_lost_alarms_before_gear_failures`.
6. Fixed: `open_alarms` reads the `lw_alarm` events since the oldest open alarm (`C8yClient::query_events_since`, all
   pages) instead of the newest 200. The fake honours `dateFrom`. Test `recovered_alarms_are_found_despite_many_later_events`
   (250 later events).
7. Fixed: a failed alarm POST also releases the type from `bus_cascade`, so the next `sync_bus_cascade` (bus fault
   change, group change, session start) raises it again; recovery skips the "Alarm cleared" event for an alarm that is
   not active. No timer-driven retry. Test `failed_cascade_post_is_retried_and_not_recovered`.
8. Not modelled, documented in D8: the firmware answers `lw_switchGroupMode` SUCCESSFUL even without bus power (it
   changes its own group state), so refusing would deviate from it; the research gives no FAILED reason for scan / reset.
9. Added tests: two bus power types at once, group join / leave and an ungrouped lamp during a failure, sensor fault
   of its own surviving recovery, bus toggled before the POSTs are answered, `paused` false after recovery (snapshot),
   migration PUT failure (finding 1).
10. Fixed: `set_fault` copies the id of an already open alarm into the new `ActiveFault`. Test `fault_on_a_held_alarm_takes_its_id`.
