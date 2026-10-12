# Phase 3 review — simulation engine and device features

Scope: `src-tauri/src/sim/{core,emit,session,outbound,scheduler,operations,faults,scenario,tests}.rs`,
`src-tauri/src/device/**`, `src-tauri/src/model/{session,device,ops}.rs`,
`src-tauri/src/commands/{devices,session}.rs` and the `c8y/**` / `payload/**` APIs they use. Checked
against PRD §6, §8, §9, §12–§14, DECISIONS.md, ARCHITECTURE.md, `research/firmware-core-v2.md`,
`research/floorplan-plugin.md` and `reviews/engine-requests.md`.

Build state: `cargo test` 191 passed, 0 failed; `cargo clippy --all-targets` clean (no warnings).

Summary: **5 high, 17 medium, 17 low.**

What works well: the state loop is pure (`SessionCore` has no I/O), never awaits HTTP, and holds no
lock across an await. Timers run on a fixed grid, so they do not drift. The outbound queues are
bounded. Realtime de-duplication plus `Triage` handle echoes of the app's own `PUT`s correctly.
The firmware texts, the readback echo, the counter rules and the D6 assignment rules are all
followed.

---

## High

1. **`sim/core.rs:299`, `sim/core.rs:805-809` — DALI group mode is not restored at start, and `lw_daliGroupChange` is only sent on a real change, so the floorplan shows the wrong mode after a restart.**
   Every session starts with all groups in `Auto`. The plugin reads the mode from the **newest
   `lw_daliGroupChange` event** and only falls back to the newest SUCCESSFUL op when no event exists
   (`floorplan-plugin.md` §5, `live-state.ts:163-183`). Example: in the last session group 2 was set
   to `On`, so the newest event says `On`. After a restart the simulator thinks the group is `Auto`.
   The presenter clicks "Auto" in the floorplan, `prev == mode`, so no event is sent, the op goes
   SUCCESSFUL, and the plugin keeps showing `On`. The firmware can skip the event on "no change"
   only because it keeps its real mode across restarts (`firmware-core-v2.md` §3.3). The simulator
   does not.
   **Fix:** in `session::prepare`, read the newest `lw_daliGroupChange` per group
   (`GET /event/events?source=<id>&type=lw_daliGroupChange&revert=true&pageSize=…`, newest per
   `groupModeChange.id`). Seed `SessionCore.groups` and the lamp levels from it. As a safety net,
   send the event after **every** `lw_switchGroupMode` with source `cloud-operation`, even when the
   mode did not change (keep "only on change" for DI actions). Add a test: restart, then an op that
   sets the mode it already has, then expect an event.
   Severity: **high**.

2. **`lib.rs:58-97`, `sim/session.rs:333-343`, `sim/operations.rs:294-305` — quitting the app, or a panic in the session task, leaves visible ops EXECUTING forever and the lock held. Nothing cleans this up later.**
   There is no `RunEvent::ExitRequested` / `CloseRequested` handler. `announce_exit` is never
   called (dead code; its doc claims every non-`Stop` exit goes through it). On quit the tokio
   runtime simply drops the task, so `fail_open_prompts` and `release_lock` never run. Once an op
   is EXECUTING the plugin has **no timeout** (`floorplan-plugin.md` §5), so the floorplan control
   spins forever. At the next start only PENDING ops older than 60 s are failed. EXECUTING ones are
   `Triage::Ignore`d, so they are never cleaned up. A panic in the task has the same effect, and
   the UI only hears about it the next time a command happens to prune the handle.
   **Fix:**
   (a) In `run()`, register `.build(...).run(|app, ev| …)`. On `ExitRequested`, call
   `api.prevent_exit()`, run `AppState::stop_session` with a timeout of about 6 s, then
   `app.exit(0)`.
   (b) Spawn a small watcher on the session `JoinHandle`. When the handle ends with
   `Err(JoinError)`, call `announce_exit(app, Some(&AppError::Internal("session crashed")))` and
   clear `AppState::session`.
   (c) In `prepare`, query EXECUTING ops of this device whose fragment is in `VISIBLE` and that are
   older than `PROMPT_TIMEOUT`. They can only be left over from an earlier run. Set them FAILED
   "Simulator was offline".
   Severity: **high**.

3. **`device/demo_state.rs:222`, `:305-316`, `:381-392` (via `commands/devices.rs:143-202`) — `update_demo_setup`, `set_dali_groups` and `set_digital_inputs` never check that the device is a demo device.**
   `patch_setup` uses `client.get_managed_object` instead of `crud::load_demo_mo`, then writes
   `lw_demo`, **`lw_configModbusSlaves`** and possibly **`lw_configCore`**. `set_groups` and
   `set_digital_inputs` write `lw_configGroups` / `lw_configDigitalInputs` without reading the
   device at all. A wrong or stale `deviceId` from the UI (or a later frontend bug) would overwrite
   the firmware config of a real Core v2. The next `lw_fetch*` would then push that config to the
   real hardware. PRD §6.1: "the app never simulates on real devices".
   **Fix:** call `crud::load_demo_mo(client, device_id)` first in all three functions; `get_config`
   should do the same for consistency. Add tests that each of these returns `InvalidInput` for a
   managed object without `c8y_DeviceTypes: ["demo"]`.
   Severity: **high**.

4. **`device/upgrade.rs:162-175` → `device/apply.rs:203-222` — an upgrade deletes floorplans the app did not create, without asking.**
   The upgrade calls `apply_floorplans` with `replace: true` for **added** and changed layouts.
   `apply_floorplans` takes **every** `lw_floorplan` child of the device with the same name as
   `old` and deletes it, binary included, whether or not its id is in `lw_demo.floorplanIds`. So
   if a user built a floorplan "Erdgeschoss" in the plugin and a later library version adds a
   layout "Erdgeschoss", the upgrade silently deletes the user's floorplan. The upgrade preview
   only lists the floorplan as "added". (`apply_demo` has the same deletion, but there the user
   confirms it through `floorplanConflicts` / `replaceExistingFloorplans`.)
   **Fix:** in `FloorplanJob`, split `replace` into "may replace app-created" and "may replace
   foreign". Upgrades only replace ids that are in `state.floorplan_ids`. A same-name foreign
   floorplan is skipped with a warning, and `UpgradePreview` lists it as a conflict (new field
   `floorplanConflicts`).
   Severity: **high**.

5. **`device/upgrade.rs:128-135` — the upgrade writes the new `libraryVersion` before it touches the floorplans, so a failure part-way leaves the device marked up to date.**
   `lw_demo` (with the new `library_version` and `applied_items`) is written first. If
   `floorplan::create` or `delete` then fails (network, 403, a broken layout), the call returns
   `Err`. But the device now reports the new version, `upgrade_available` is false, and a retry
   fails with "device is up to date". The changed floorplans are never upgraded.
   **Fix:** write `lw_configModbusSlaves` plus `lw_demo` **without** bumping `library_version`
   first. Do the floorplans, recording each created / deleted id in `state` and persisting `lw_demo`
   after each one. Set `library_version` only in the final write.
   Severity: **high**.

---

## Medium

6. **`sim/outbound.rs:116-136`, `sim/operations.rs:616-619` — one hanging status `PUT` blocks the priority lane. A late EXECUTING then lands after the plugin's 10 s cancel and leaves the op stuck in EXECUTING.**
   The priority lane sends one request at a time, with the 30 s client timeout. A single slow
   `PUT` delays every following EXECUTING past the plugin's 10 s PENDING cancel. When that happens
   the tenant sees PENDING → FAILED (plugin) → EXECUTING (simulator). `OpDedup` drops the
   simulator's own EXECUTING echo, the session withdraws the prompt on the FAILED event, and the
   op stays EXECUTING in Cumulocity with nobody left to finish it.
   **Fix:**
   (a) Give status `PUT`s a short timeout through `RequestSpec::timeout(Duration::from_secs(5))`.
   (b) Send EXECUTING concurrently across ops; only keep the order within one op (for example a
   small per-op chain, or spawn EXECUTING and queue the final status once EXECUTING has returned).
   (c) On `Triage::Withdraw` caused by a FAILED event, if the app already sent EXECUTING, queue
   FAILED again with the observed `failureReason`. This is idempotent and fixes the dangling
   state.
   Severity: medium.

7. **`sim/operations.rs:310-320` — the "Simulator was offline" rule compares the laptop clock with the server's `creationTime`, for every op.**
   A laptop clock running more than 60 s fast fails **every** visible op as offline. One running
   slow lets genuinely stale ops through. PRD §9 only asks for the rule "on session start".
   **Fix:** apply the age rule only to the PENDING batch fetched on (re)connect. Either have
   `RealtimeMsg` mark "initial PENDING list" ops, or treat ops that arrive before the first live
   connect reply that way. Also measure the skew from the HTTP `Date` header (the client already
   parses responses) and correct `now` by it.
   Severity: medium.

8. **`sim/core.rs:303-304`, `sim/faults.rs:131-163` — faults and alarm effects are neither restored at start nor cleared at stop, so the UI cannot recover alarms that are still ACTIVE in the tenant.**
   `Stop` leaves raised alarms ACTIVE; in realistic mode they stay ACTIVE even after recover. The
   next session starts with `faults = []` and `alarms = {}`. The fault menu shows "not active", and
   `set_fault(target, false)` on a fault that is not in the list does nothing. So the presenter can
   no longer send "Alarm cleared" or clear it; only a device Reset helps. The floorplan keeps
   showing the alarm.
   **Fix:** in `prepare`, `GET /alarm/alarms?source=<id>&status=ACTIVE,ACKNOWLEDGED`. Map each type
   back to a `FaultTarget` through `fault_catalog` (and alarm-effect types to their item), and fill
   `faults` / `alarms` (with ids) plus the lamp / slave side effects (`update_lamp`,
   `timed_out`). Alternatively let `set_fault(…, false)` clear by type when no tracked fault
   exists.
   Severity: medium.

9. **`sim/session.rs:138-217` — the values of writable Modbus registers are not restored from the last SUCCESSFUL `lw_modbusWrite`, so the plugin shows a readback "mismatch" after every restart.**
   The plugin takes the control state from the newest SUCCESSFUL `lw_modbusWrite` (key
   `<type>:<slave>:<register>`, `floorplan-plugin.md` §5) and compares the readback with `===`.
   After a restart the simulator sends the control default (or the snapshot value), which does not
   match.
   **Fix:** in `prepare`, `GET /devicecontrol/operations?deviceId=<id>&status=SUCCESSFUL&fragmentType=lw_modbusWrite&revert=true&pageSize=100`.
   For each writable register take the newest op value and seed the item with it as
   `counter_seeds` does (marking it manual, so a generator does not overwrite it). Restored values
   should win over the snapshot.
   Severity: medium.

10. **`sim/core.rs:1066-1082`, `:895-909` — lock refresh and "Save snapshot" `PUT` the whole `lw_demo` from the session's own copy, which loses other people's updates.**
    `refresh_lock` (every 3 min) and `save_snapshot` write `self.demo` as a whole. Any `lw_demo`
    change the session did not see is reverted within 3 minutes: `update_demo_setup` from another
    laptop, or a second presenter who took over with `force` (their lock is overwritten, and two
    sessions now simulate one device without either noticing). `SetupChanged` covers only this
    app's own `update_demo_setup`.
    **Fix:** move the lock into its own fragment `lw_demoLock {user, since}` (Cumulocity replaces
    a top-level fragment as a whole, so writing the lock no longer touches the setup). Before a
    refresh, check that the device still carries our lock (a cheap `GET` from a spawned task, with
    the result fed back in), and on a foreign lock stop with a `session-error`. For the snapshot,
    re-read `lw_demo` and only replace `snapshot`; or store the snapshot in `lw_demoSnapshot`.
    Severity: medium.

11. **`device/apply.rs:218-229`, `:250-267` — a failed apply leaves orphaned floorplans that Reset / Delete will never remove.**
    `lw_demo` is written before the floorplans, and `floorplanIds` is only persisted after **all**
    floorplans succeeded. If layout 2 of 3 fails, layout 1 is already created and linked but not
    recorded. Reset / Delete only remove recorded ids, so it stays forever (and the next apply
    without `replace` skips it as a "conflict"). In the same window, a replaced floorplan is
    already deleted.
    **Fix:** persist `lw_demo` (with the new `floorplanIds`) after each created / replaced floorplan,
    or catch the error, write the partial state, and return the error with the partial result.
    Same pattern as finding 5.
    Severity: medium.

12. **`device/upgrade.rs:128-131`, `device/apply.rs:50-54` — upgrades ignore changes at the demo-pack level.**
    The selection uses the device's copy of `tags` / `includes` / `excludes` taken at apply time.
    A new library version that adds an include, changes `daliLampCount`, groups, digital inputs or
    `sendIntervalSec` produces no diff and is never applied. Scenarios happen to work only because
    they are read from the library at session start. PRD §14 promises an upgrade that applies "the
    changes".
    **Fix:** store `appliedDemo` (a digest per field: `tags`, `includes`, `excludes`,
    `daliLampCount`, `daliGroups`, `digitalInputs`, `sendIntervalSec`) at apply time. On upgrade,
    do a three-way merge per field: if the device value still equals what was applied (no local
    override), take the new library value; otherwise keep the local one and list it in the preview
    as "kept local override".
    Severity: medium.

13. **`device/upgrade.rs:149-175` — an upgrade overwrites app floorplans that were edited in the plugin, without warning.**
    A "changed" layout replaces the device floorplan even when the presenter moved widgets in
    Cumulocity. This is exactly the kind of local override PRD §14 says to keep "where possible",
    and the preview gives no hint.
    **Fix:** at apply time also store the digest of the created `lw_floorplan` body
    (`appliedFloorplanBodies[id]`). On upgrade, compare it with the current managed object. If the
    floorplan was edited, show it in the preview as "edited on device", and replace it only on an
    explicit opt-in (or create the new one next to it as "<name> (v1.5)").
    Severity: medium.

14. **`library/validate.rs:755-772` (used by `device/demo_state.rs:200`, `device/apply.rs:71`) — the export round trip breaks the device it was exported from.**
    Export writes `customSlaves` / `customItems` into `items/<demoId>.json`. Once that PR is
    released, the same ids exist in the library, and on the original device `validate_custom`
    reports "item id … is already used". After that, `update_demo_setup` on that device fails
    forever, and apply / plan carries a warning. Upgrade never moves the custom entries into the
    library.
    **Fix:** in `apply_patch`, `plan` and `upgrade::apply`, first drop custom slaves / items whose
    id now exists in the library (when the digests are equal: silently; otherwise keep the library
    version and warn). Have the upgrade preview list them as "now in library".
    Severity: medium.

15. **`payload/naming.rs:219-222` (via `device/setup.rs:83-97`) — deselecting a slave throws away its id / address, so the numbers get reused.**
    `assign_slaves` keeps only the assignments whose key is still selected. If a slave is excluded
    and later included again, it gets a new id / address. Meanwhile another new slave can take the
    freed `LW_Modbus_Slave_20_240`, mixing two slaves' history and breaking floorplan references.
    That contradicts D6 ("Assignments are never renumbered").
    **Fix:** keep unselected assignments in `lw_demo.slaves` (for example with `active: false`, or
    a separate `retiredSlaves`), and count their ids / addresses as used. Only
    `lw_configModbusSlaves` should be limited to selected slaves. The same applies to register
    assignments in `assign_registers`.
    Severity: medium.

16. **`sim/core.rs:353-358` vs `:247-252` — `Generator` steps in the start scenario override `lw_demo.generators` at every session start.**
    The gas-station start scenario "normal" sets a generator for `freezer_temp`. A presenter who
    persisted a different generator for `freezer_temp` with `update_demo_setup` gets it replaced on
    every start. Exported packs make this worse: export turns every generator override into a
    start-scenario step (`device/export.rs:468-478`).
    **Fix:** when applying the *start* scenario at session start, skip `Generator` steps for items
    that have an entry in `demo.generators`, or apply `demo.generators` after the start scenario.
    A manually applied scenario keeps overriding as now.
    Severity: medium.

17. **`commands/session.rs:33-46`, `sim/session.rs:160-186` — `start_session` holds the `AppState::session` mutex across many sequential HTTP calls.**
    `prepare` does a GET, a PUT and then one `newest_value` GET **per counter item**, one after the
    other (gas-station: about 10; 30 s timeout each). Meanwhile `get_session`, `stop_session`,
    `logout` and the 401 path (`expire_auth`) wait on the mutex. The lock is held on purpose (so
    two concurrent starts cannot both win), but a slow tenant freezes the UI for a long time.
    **Fix:** run the counter seeds concurrently (`futures::future::join_all`, or one query per
    fragment). Better still, guard concurrent starts with a `starting: AtomicBool` / placeholder
    state instead of holding the mutex across the I/O.
    Severity: medium.

18. **`sim/core.rs:560-573`, `sim/outbound.rs:89-96` — changes are sent immediately with no coalescing. A full queue then drops payloads of any kind, including alarms, events and the lock release.**
    Every `set_item_value` is its own loop iteration and its own `POST`. A slider drag that sends
    20 commands per second creates 20 requests per second; a slow tenant fills the 256 slots, and
    `enqueue` then rejects whatever comes next: alarms, clears, `lw_daliGroupChange`, inventory
    effects. Old periodic sends stay queued too, so the backlog is stale data.
    **Fix:** coalesce immediate measurement sends with a short debounce timer (about 250 ms; add it
    as a scheduler deadline). On a full normal lane, drop queued *periodic* measurements first,
    since a newer full send supersedes them. Alarms / events / inventory should never be dropped:
    give them a reserved share of the capacity or their own lane.
    Severity: medium.

19. **`device/floorplan.rs:104-119` — `floorplan::delete` deletes any managed object id it is given, with its binary.**
    Reset / Delete / upgrade pass ids from `lw_demo.floorplanIds`. `lw_demo` is plain inventory data
    anyone can edit (device cloned in Device Management, fragment copied by hand), and the id is
    never checked against the type or the parent device.
    **Fix:** in `delete`, require `mo.type == "lw_floorplan"`. Pass `device_id` in and check that the
    floorplan is a child addition of it (or carries an app marker such as
    `lw_floorplan.createdBy = "lw-demo-simulator"`, written by `floorplan_body`). Skip the id with a
    warning otherwise.
    Severity: medium.

20. **`c8y/fake.rs` — the fake lacks `childAssets`, `assetParents` and `revert`.**
    `create_demo_device` with `parentGroupId` (including its rollback when linking fails), the
    grouping in the device list, and counter seeding from the *newest* stored value are untested.
    Counter seeding is only tested with a single seeded value, so ordering bugs cannot show up.
    **Fix:** add `POST /inventory/managedObjects/{id}/childAssets`, fill
    `assetParents.references` when `withParents=true`, and sort by `time` with `revert=true`. Add
    tests: create with a parent, list grouped, counter seeded from the newest of three values.
    Severity: medium.

21. **`device/tests.rs` — no tests for the destructive device paths.**
    Missing: Reset (alarms cleared, only app floorplans and binaries deleted, `floorplanIds`
    emptied), Delete, the demo-device guard on every mutating command (finding 3), apply / upgrade
    failing part-way (findings 5 and 11), an upgrade meeting a same-name user floorplan (finding 4),
    and re-applying the same demo twice (idempotency, `replaceExistingFloorplans` true and false).
    **Fix:** add fake-backed tests. The fake needs a "fail the next request to path X" hook for the
    partial-failure cases.
    Severity: medium.

22. **`sim/tests.rs` — no tests for timing, error and lifecycle paths.**
    Missing: real outbound lanes (cross-lane order, a hanging request, a full queue), a 401 in the
    middle of a session (stop plus drain), a session panic or quit (finding 2), the group mode
    after a restart (finding 1), a late EXECUTING after the plugin's cancel (finding 6), clock skew
    (finding 7), and `SetupChanged` while a session runs. All `paused` tests use `test_pair`, which
    merges both lanes into one ordered channel, so lane ordering cannot fail there.
    **Fix:** add a slow fake (artificial latency per path) and tests for the cases above.
    Severity: medium.

---

## Low

23. **`sim/core.rs:1158-1170`, `sim/outbound.rs:48-54` — the final op status can overtake its own effects.**
    SUCCESSFUL goes on the priority lane, while the readback measurement and `lw_daliGroupChange` go
    on the normal lane. With a backlog the plugin briefly shows "mismatch" or the old mode.
    **Fix:** only EXECUTING needs the priority lane. Queue SUCCESSFUL / FAILED on the normal lane
    after the effects, and gate it on EXECUTING's result (track it in `purposes`).
    Severity: low.

24. **`sim/session.rs:147-151` — `prepare` writes the lock before `setup::resolve`.**
    If `resolve` fails (missing slave, more than 20 slaves), the user's own lock stays on the device
    for 10 minutes.
    **Fix:** resolve and validate first, then write the lock as the last step of `prepare`.
    Severity: low.

25. **`device/apply.rs:59` — apply ignores the advisory lock and clears it.**
    `new_state` sets `lock = None` and apply never calls `check_lock`. Applying on a device another
    presenter is using removes their "in use" marker without a hint.
    **Fix:** call `check_lock(existing, me, req.force, now)` in `apply` (add `force` to
    `ApplyDemoRequest`) and keep a foreign lock.
    Severity: low.

26. **`sim/core.rs:797-802` — group switches send phantom lamps.**
    A group whose `converters` include lamps above `daliLampCount` sends measurements for lamps
    that do not exist (`send_lamps.insert(lamp)` → `hidden_lamp_values` with brightness 0).
    **Fix:** skip lamps that are not in `self.lamps`.
    Severity: low.

27. **`sim/operations.rs:616-619` — a withdrawn prompt emits no `operation-resolved`.**
    The UI only learns about it from the next snapshot, and nothing reaches the activity log.
    **Fix:** emit `OperationResolved` with a new `OperationResolution::Withdrawn` and the status
    that was seen.
    Severity: low.

28. **`sim/core.rs:992`, `:750-756` — an alarm `POST` that fails leaves the track active without an id.**
    A later clear sets `clear_pending` forever, and the fault shows `alarmId: null`.
    **Fix:** on `Purpose::Alarm` with `Err`, reset `track.active` (or retry once) and emit
    `fault-changed`.
    Severity: low.

29. **`model/ops.rs:30`, `sim/operations.rs:326` — the doc for `OperationPrompt.by` overstates what is read.**
    It says "`creationRequest.user` / `by`", but only `by` is read.
    **Fix:** fall back to `op.creationRequest.user` (or `op.user`), or correct the doc.
    Severity: low.

30. **`sim/mod.rs:12-13` — stale comment.**
    It says "except one spawned EXECUTING `PUT` per visible op"; the code uses the priority lane.
    **Fix:** update the comment.
    Severity: low.

31. **`sim/scheduler.rs`, `sim/operations.rs:697` — on macOS, `tokio::time::Instant` stops while the laptop sleeps.**
    After waking up, prompt deadlines and the lock refresh are late by the length of the sleep, so
    an op can stay EXECUTING far beyond 60 s of wall time.
    **Fix:** compare wall-clock `expires_at` too (the prompt carries it), or check
    `SystemTime` on each tick and expire by wall time.
    Severity: low.

32. **`commands/session.rs:33-36` — `start_session` takes its client before it waits for the session mutex.**
    A `logout` that runs in between stops "no session", clears auth, and then the start goes ahead
    with the logged-out client.
    **Fix:** re-check `state.client()` (same tenant / user) after taking the mutex.
    Severity: low.

33. **`sim/faults.rs:33` vs `:64` — the fault catalog and the session count lamps differently.**
    The catalog uses `state.dali_lamp_count`; the session uses `max(dali_lamp_count, highest lamp
    item)`. Lamp items above `daliLampCount` get no faults in the menu.
    **Fix:** use the same rule (factor out `lamp_count(demo, items)`).
    Severity: low.

34. **`sim/operations.rs:193-205`, `:549-581` — `lw_modbusWrite` checks are looser than the firmware.**
    The simulator accepts non-integer register values (the plugin always rounds) and does not check
    `lw_configRs485.isActive` (firmware: FAILED "RS485 interface inactive").
    **Fix:** reject `value.fract() != 0.0` and check RS485 from the device config.
    Severity: low.

35. **`sim/session.rs:458-470` — on stop, the lock release `PUT` waits behind the normal-lane backlog.**
    With a slow tenant, 5 s is not enough and the lock stays for 10 minutes. The final measurement
    payloads are wasted too.
    **Fix:** clear queued periodic measurements on stop, and send the lock release (and the final
    statuses) first.
    Severity: low.

36. **`sim/outbound.rs:116-136` — a lane task that dies is never noticed.**
    If a lane task panics or ends, every later `enqueue` fails ("channel closed") and is only
    reported as a throttled non-fatal error, while the session keeps running and sends nothing.
    **Fix:** treat a closed lane as fatal: end the loop and emit `session-error {fatal: true}`.
    Severity: low.

37. **`commands/devices.rs:52-58` — `create_demo_device` with `applyDemo` leaves a half-configured device when the apply fails.**
    The device exists, but the call returns only the error, so the UI does not know the new id.
    **Fix:** return the created device together with the apply error (or warnings), or roll back
    with a delete.
    Severity: low.

38. **`sim/scheduler.rs:19` — the 1 s minimum send interval makes about 300 requests per minute.**
    That is about 5 POSTs per periodic send for gas-station (see the request-rate section). Within
    the PRD, but heavy for a shared tenant.
    **Fix:** raise the UI / schema minimum to 5 s, or warn below 10 s.
    Severity: low.

39. **`commands/devices.rs:185-202` — changes made by `set_dali_groups` / `set_digital_inputs` do not reach a running session.**
    The session keeps the old groups and DI config (series, actions, counter grid) until the next
    start, unlike `update_demo_setup`.
    **Fix:** send a `SessionCommand::ConfigChanged { groups, inputs }`, or document that a restart
    is needed and have the UI offer it.
    Severity: low.

---

## Request rate (question 6)

Gas-station: 72 selected items, made up of 37 Modbus (12 slaves), 14 custom alarms (no
measurement), 9 DALI lamp items, 9 digital inputs, 2 system and 1 brightness sensor;
`daliLampCount` 14, 4 groups covering lamps 1, 3 and 8–14; `sendIntervalSec` 30.

- One periodic send makes **5 POSTs**: sensors and system in 1, digital inputs in 1, DALI in 2
  (9 sendable lamps in chunks of 6; lamps 2 and 4–7 are in no group and are not sent, as on the
  firmware), Modbus in 1 collection. That stores **58 measurements**.
- Every 30 s that gives **about 10 requests per minute** plus a lock `PUT` every 3 min. That is
  **about 116 stored measurements per minute**, roughly 7k per hour or 170k per 24 h for one
  running device.
- Generators step every 5 s but only update the UI. Their values reach Cumulocity with the
  periodic send. Alarm effects of generator values fire at once, but rarely.
- Operations add 2 `PUT`s plus 1–2 POSTs each; manual changes add 1–2 POSTs each (but see
  finding 18 about slider drags).

Verdict: reasonable for a demo tenant. The things to watch are the 1 s minimum interval
(finding 38) and immediate sends without coalescing (finding 18).


---

## Resolution

All 39 findings are fixed (engine fixer, round 2). Build state at the end: `cargo fmt` clean,
`cargo clippy --all-targets -D warnings` clean, `cargo test` 245 passed (was 191), `npm run gen:types` done,
`npm run typecheck` passes. `src/**` was not edited; the IPC follow-ups are in `docs/reviews/engine-requests.md`
under "Frontend follow-ups (round 2)". The shipped `library/` still loads with zero warnings, also with the new
validation rules.

Design changes that touch several findings:

- **Lock and snapshot live in their own fragments** `lw_demoLock` / `lw_demoSnapshot` (`device/demo_state.rs`).
  `read()` merges them into `LwDemoState.lock` / `.snapshot` (an older copy inside `lw_demo` is still read), and
  `patch_body()` writes `lw_demo` without them. So a lock refresh or "Save snapshot" never rewrites the setup.
- **Outbound queue** (`sim/outbound.rs`) is now a real two-lane queue with a drop policy instead of two mpsc
  channels. Normal lane: one task, strict order. Priority lane: concurrent sends (JoinSet). Droppable items are
  measurements only (periodic ones first); alarms, events, inventory, lock and status updates are never dropped
  (hard cap 4096).
- **Final operation statuses wait for the answer to EXECUTING** and go on the normal lane behind the operation's
  effects (`SessionCore::gated`).

| # | Fix | Test |
|---|-----|------|
| 1 | `prepare` reads the newest `lw_daliGroupChange` per group (`revert=true`), the session starts in that mode (silently). A cloud `lw_switchGroupMode` always sends the event (`always_event`); DI actions and the local command keep "only on change". | `review_paused::group_mode_is_restored_and_cloud_switch_always_reports`, `review_fake::restart_restores_group_modes_alarms_and_written_registers` |
| 2 | (a) `lib.rs` builds with `.run(...)`: `ExitRequested` is prevented once, the session is stopped with a 6 s limit (`AppState::stop_for_exit`), then `app.exit(0)`. (b) `spawn_with` wraps the state loop in a watcher task: a panic emits fatal `session-error` + `session-changed(null)` at once. (c) `prepare` fails visible operations that are EXECUTING for more than 60 s ("Simulator was offline"). `announce_exit` (dead code) is removed. | `session_panic_is_reported`, `leftover_executing_operations_are_failed_at_start`, `stop_*` tests. The exit hook itself needs a Tauri runtime and has no automated test. |
| 3 | `patch_setup`, `set_groups`, `set_digital_inputs`, `get_config` use `crud::load_demo_mo`. | `every_writer_refuses_a_real_device` (also apply / reset / upgrade / delete; asserts that no write reached the device) |
| 4 | `Replace::{No, OwnedOnly, All}`: an upgrade only replaces ids in `floorplanIds`; a same-name foreign floorplan is skipped with a warning and listed in `UpgradePreview.floorplanConflicts`. No new command argument: instead of an opt-in, an edited floorplan is never replaced (see 13). | `upgrade_never_touches_a_same_name_floorplan_of_the_user` |
| 5 | Upgrade order: config + `lw_demo` with the OLD version, floorplans one by one (persisted after each), and only the last write sets `libraryVersion`. | `failed_upgrade_keeps_the_old_version_and_can_be_retried` |
| 6 | (a) status `PUT`s have a 5 s timeout and one retry on a transport error; (b) EXECUTING goes through the concurrent priority lane; (c) a FAILED seen for an open prompt re-queues FAILED (with the observed reason) behind EXECUTING. | `executing_is_sent_concurrently_and_not_behind_measurements`, `priority_lane_is_not_held_up_by_a_hanging_normal_request`, `late_executing_after_plugin_cancel_is_failed_again` |
| 7 | New `RealtimeMsg::PendingOperation` marks the PENDING list of a (re)connect; only those can be "offline". The age is measured on the server clock (`C8yClient::clock_skew`, from the `Date` header). | `offline_rule_uses_the_server_clock_and_only_the_initial_list`, `clock_skew_comes_from_the_date_header`, `only_the_initial_list_can_be_stale` |
| 8 | `prepare` reads ACTIVE / ACKNOWLEDGED alarms; types are mapped back to a `FaultTarget` through `fault_catalog` (item effect alarms are tracked too); lamp / slave side effects are restored. Alarms of an unknown type are not tracked. | `restart_restores_..._alarms_...`, `start_scenario_clears_restored_faults` |
| 9 | `prepare` reads the newest SUCCESSFUL `lw_modbusWrite` per `<type>:<slave>:<register>`; writable items start with that value, marked manual, winning over the snapshot. | `restart_restores_..._written_registers` |
| 10 | Lock refresh is an outbound `LockRefresh` that reads the device first and reports `DeviceLocked` (fatal for the session) when somebody else holds a fresh lock. Snapshot writes only `lw_demoSnapshot`. | `lock_refresh_leaves_the_setup_alone_and_notices_a_takeover`, `lock_taken_over_ends_the_session`, `lock_and_snapshot_live_outside_lw_demo` |
| 11 | `apply_floorplans` persists `lw_demo` after each created floorplan, and again after each replaced one is deleted (the new one is recorded before the old one is deleted). | `failed_apply_leaves_no_orphaned_floorplans` |
| 12 | `lw_demo.appliedDemo` (digest per field) at apply time; the upgrade merges three-way per field (`demoChanges` / `keptOverrides` in the preview). A device applied before this existed gets no demo-level merge. | `upgrade_merges_demo_level_fields_per_field` |
| 13 | `lw_demo.appliedFloorplanBodies` (id to digest of the created `lw_floorplan`). An edited floorplan is kept (and is no longer in `floorplanIds`), the new layout is created next to it as `<name> (v<version>)`; the preview lists it in `floorplansEdited`. A device without a stored digest cannot be checked and is treated as not edited. | `upgrade_keeps_an_edited_floorplan` |
| 14 | `setup::drop_library_duplicates` in apply / plan / upgrade / `update_demo_setup`: custom entries whose id is in the library are dropped (library wins, warning when they differ); an item that was only selected as a custom item moves to `includes`. In `update_demo_setup` only entries that were already custom count, a newly added colliding id is still an error. | `released_export_does_not_break_the_source_device` |
| 15 | `LwDemoState.retiredSlaves`: unselected slaves / registers keep their ids / addresses; `assign_slaves` / `assign_registers` count them as taken. | `deselected_slaves_keep_their_numbers`, `naming::deselected_slave_numbers_are_not_reused` |
| 16 | The start scenario skips `Generator` steps for items in `lw_demo.generators`; a manually applied scenario still overrides. | `start_scenario_keeps_persisted_generator_overrides` |
| 17 | `start_session` holds the session mutex only to look and to store; a `StartClaim` keeps a second start out. Counter seeds (and the other start-up reads) run concurrently. | `state::tests::only_one_start_at_a_time` |
| 18 | Manual changes are coalesced (first change opens a 300 ms window, the latest value goes out). The queue never drops alarms / events / statuses; periodic measurements go first and a new full send supersedes the queued older one. | `manual_changes_are_coalesced`, `outbound::tests::full_queue_drops_periodic_then_immediate_but_never_critical`, `periodic_supersede_and_clear_on_stop` |
| 19 | `floorplan::delete(client, device_id, id)` / `delete_many`: only an `lw_floorplan` that is a child addition of the device; anything else is skipped with a warning. | `reset_and_delete_touch_only_the_apps_floorplans` |
| 20 | `FakeC8y`: `childAssets`, `withParents`, `revert` (events, measurements, operations), `valueFragmentSeries`, `fragmentType`, `fail_after`, `delay`, `Date` header / clock offset. | `create_with_parent_group_lists_grouped_and_rolls_back`, `counter_is_seeded_from_the_newest_stored_value`, `fake_supports_revert_child_assets_and_parents` |
| 21 | New fake-backed device tests: Reset / Delete, demo guard, failing apply / upgrade, same-name user floorplan, re-apply idempotency. | see the tests named in 3, 4, 5, 11, 19 and `reapplying_is_idempotent` |
| 22 | New timing / lifecycle tests: real lanes (priority overtakes a slow request, order of the normal lane, dead lane, full queue), 401 mid-session, panic, restart group mode, late EXECUTING, clock skew, `SetupChanged`. | `outbound::tests::*`, `review_paused::*`, `review_fake::*` |
| 23 | Final statuses go on the normal lane after the effects and wait for EXECUTING's answer. | `final_status_waits_for_executing_and_follows_the_effects` |
| 24 | `prepare` resolves the setup first and writes the lock last. | `prepare_takes_the_lock_only_after_the_setup_resolves` |
| 25 | `ApplyDemoRequest.force` (optional); `apply` calls `check_lock`. The lock is in its own fragment, so an apply no longer clears it. | `apply_respects_a_foreign_lock` |
| 26 | Lamps that are not in `self.lamps` are skipped in a group switch. | `group_switch_sends_no_phantom_lamps` |
| 27 | `OperationResolution::Withdrawn` with the observed status / reason. | `late_executing_after_plugin_cancel_is_failed_again` |
| 28 | A failed alarm `POST` resets the track (`active`, new `inflight` flag), emits `fault-changed` with `alarmId: null`; `clear_pending` is only set while the create is in flight. | `failed_alarm_post_does_not_leave_a_dangling_fault` |
| 29 | `by` falls back to `creationRequest.user`, then `user`. | `operations::by_falls_back_to_creation_request_user` |
| 30 | `sim/mod.rs` comment corrected. | |
| 31 | Prompts carry `expires_wall` and also expire by the wall clock; the lock refresh also fires when the wall clock says so. | `prompt_expires_by_wall_clock` (the wall-clock lock refresh has no test) |
| 32 | `AppState::is_current_login` is re-checked after the start; a mismatch stops the new session and returns `notAuthenticated`. | `state::tests::login_is_rechecked_after_the_start` |
| 33 | One rule, `faults::lamp_count(daliLampCount, items)`, for the catalog, the resolution and the session. | `faults::tests::lamp_count_follows_the_highest_lamp_item` |
| 34 | Non-integer `lw_modbusWrite` values and an inactive RS485 interface (`lw_configRs485.isActive`) fail like the firmware. | `modbus_write_rejects_fractions_and_inactive_rs485`, `operations::modbus_write_values_must_be_integers` |
| 35 | At stop, queued measurements are discarded; the lock release and final statuses use the priority lane. | `stop_discards_queued_measurements_and_sends_the_release` |
| 36 | A lane task that ends closes its lane (guard); `enqueue` fails with `Closed` and a result with `LANE_DEAD_TAG` reaches the session; both are fatal. | `closed_outbound_lane_is_fatal`, `outbound::tests::a_dead_lane_is_reported_and_closed` |
| 37 | `create_demo_device` with `applyDemo` rolls the new device back (delete) when the apply fails. No automated test (needs a Tauri `State`). | |
| 38 | `MIN_SEND_INTERVAL_SEC` is 5 in the scheduler and the library validation. The JSON schema in `library/schema/` still says 1 (not mine). | `scheduler` tests |
| 39 | `SessionCommand::ConfigChanged`; `set_dali_groups` / `set_digital_inputs` send it to a running session on that device. | `config_changed_reaches_the_running_session` |

Other changes: the fixture library's start scenario now clears the faults of "trouble" (the new validation rule 1);
the shipped library already satisfied it. Validation proposals 1-3 from `engine-requests.md` are implemented (see
there); nothing in the shipped library needed loosening.

Known limits, not covered by any finding:

- A panic in the state loop is reported at once, but the lock and the EXECUTING operations it had open are only
  cleaned up by the next start (`prepare`, finding 2c), not by the watcher.
- After an upgrade that created `<name> (v<version>)` next to an edited floorplan, the original is no longer
  recorded as the app's, so a later upgrade sees it as a foreign same-name floorplan (skip + conflict).
- Restoring alarms only tracks types this setup knows (fault catalog and item effect alarms).
