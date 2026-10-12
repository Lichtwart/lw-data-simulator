# Review: fire-once alarms, cloud clear ends the effect, "Clear all" (DECISIONS D9)

Scope: `c8y/{realtime,api,fake,integration_tests}.rs`, `model/session.rs`,
`sim/{core,faults,session,scenario,outbound,tests}.rs`, `commands/session.rs`,
`src/features/control/{FaultSections,ItemCard,ControlPanel}.tsx`, `faults.ts(+test)`,
`sessionState.test.ts`, i18n, docs (PRD §7/§8, ARCHITECTURE command table, DECISIONS D8/D9), schema.

`npm run check`: passes (typecheck, biome, vitest 85/85, cargo fmt, clippy `-D warnings`, cargo test
285/285, library validation). The two builds were not run.

## Summary

The main paths are correct and well covered. I traced these and found no defect:

- realtime `/alarms/<id>` parsing: CREATE / UPDATE unwrap `data`, DELETE (`data` = id string, number or
  object) becomes `{id, status: CLEARED}`; other devices' channels are ignored;
- subscribe both channels → GET PENDING ops → `fetched_at` → GET open alarms (all pages) → `Connected`,
  ops, `OpenAlarms`, then the notifications that came with the subscribe replies. Notifications that
  arrive between subscribe and the GET are delivered after the list and are idempotent with it;
- clear before the create answer (`recent_cleared` + check in the `Purpose::Alarm` Ok branch);
- re-sync: only tracks whose id was learned before `fetched_at` can be called "gone"; restored alarms get
  `id_at` = core creation, which is always before the realtime GET (handshake + subscribe come first);
- D8 bus cascade: a cloud clear of the bus alarm → `end_fault` → `sync_bus_cascade` recovers the
  cascade (event, PUT CLEARED unless realistic mode) and lamps / sensors send again; a cascade alarm
  cleared in the cloud stays cleared (`suppressed`) until the bus recovers; a lamp's own fault cleared
  while the bus holds the lamp keeps the lamp lost;
- "Clear all" in the session: everything ends locally, one `ClearAlarms` item is queued on the strictly
  ordered normal lane behind every alarm POST queued before it, so in-flight creates are cleared too;
- threshold alarms cleared in the cloud stay down until the value has been below the threshold once;
- the state loop never awaits HTTP (the GETs run in the realtime task, the clears in the outbound worker);
  `clear_all_alarms` only clones the sender under the `session` lock;
- IPC: `OpenAlarmStatus` / `ActiveFault.status` are specta types, no `f64` / `u64`; `fire_fault` and
  `clear_all_alarms` are in `session_commands!`; the ARCHITECTURE table lists them; i18n parity holds;
  no raw colours; the only `signal` element in the running view is still the session bar's.

The findings are races at the edges, error paths, and test gaps. Nothing is a blocker.

## Findings

### 1. Major: firing again while the presenter clears creates an alarm the session no longer tracks

`src-tauri/src/sim/faults.rs:373-395` (`alarm_cleared_in_cloud`), `src-tauri/src/sim/core.rs:1285-1330`
(`Purpose::Alarm` Ok branch), also `faults.rs:458-463` (`reconcile_open_alarms`).

- **Failure scenario:** the slave timeout fault is active on alarm `5`. The presenter clears `5` in the
  Cumulocity UI. Before the realtime notification arrives (the long poll can take a moment), they click
  "Auslösen" again in the app. This is the obvious move when the badge still says *Aktiv*. Cumulocity
  handles the clear first, so the re-post creates a **new** alarm `7`. The session then gets realtime
  `CLEARED 5`. `track.id` is still `5`, because the POST answer has not arrived, so the fault ends: the
  "Alarm cleared" event is sent and the slave values resume. Then the POST answer brings `7`.
  `track.active` is false and `7` is not in `recent_cleared`, so nothing happens. Result: alarm `7` is
  ACTIVE in Cumulocity, but the app shows no fault and the values run normally. When the presenter later
  clears `7`, the session ignores it (`find` needs `t.active`), so the firmware's "Alarm cleared" event
  never comes. A scenario that re-fires (`active: true`) and a reconnect (`reconcile_open_alarms` drops
  `5` while the re-post is in flight) hit the same race.
- **Suggested fix:** while a create is in flight for the track (`track.inflight`), let
  `alarm_cleared_in_cloud` only record the id in `recent_cleared` and leave the decision to the POST
  answer. If the answer has the cleared id, the existing `recent_cleared` check ends the fault. If it has
  a new id, the fault simply goes on with that id. Do the same in `reconcile_open_alarms`: skip
  `inflight` tracks. Turn `inflight: bool` into a counter, so that two quick fires do not clear the flag
  on the first answer.
- **Test:** add to `sim/tests.rs` `cloud_clear`: fire `door()`, ack it (id `a1`), fire again (do not
  ack), `r.cloud("a1","CLEARED")`, then answer the re-post with `{"id":"a9","status":"ACTIVE"}`. Assert
  that the fault is still active with `alarm_id == a9` and no "Alarm cleared" was sent. Then send
  `r.cloud("a9","CLEARED")` and assert that the fault ends. This test fails today.

### 2. Minor: "Clear all" reports success before anything is cleared, and a failure leaves the session out of step with Cumulocity

`src/features/control/FaultSections.tsx:105-114`, `src-tauri/src/commands/session.rs:176-189`,
`src-tauri/src/sim/faults.rs:480-497`, `src-tauri/src/sim/core.rs:1378`.

- **Failure scenario:** with a session running, `clear_all_alarms` returns as soon as the command is
  queued, so the toast "Alle Alarme aufgehoben." appears at once. Then the `ClearAlarms` item fails, for
  example because the listing GET times out or the user has no alarm admin role and both the bulk update
  and the single PUTs get 403. The alarms are still open in Cumulocity. But the session has already ended
  every effect and set every track to inactive. Later cloud clears of those alarms are ignored, and the
  re-sync after a reconnect ignores them as well. All the presenter sees is a separate `session-error`
  toast after the success toast. Their floorplan stays red while the app shows no faults.
- **Suggested fix:**
  - Word the toast as "being cleared" (`control.allClearing`), or skip it when a session runs and rely on
    the badges.
  - On `Purpose::ClearAll` → `Err`, have the realtime task re-sync, or queue a re-read of the open
    alarms, and restore the ones still open the way `restore_alarms` does. The minimum is a
    `session-error` with its own error code, so the UI can say "Clear all failed, alarms are still open".
- **Test:** a paused-time test that answers the `ClearAlarms` item with `Err(Http 500)`. Assert the
  error event today; after the fix, assert the restore.

### 3. Minor: a failed alarm POST leaves a fault that nothing can end

`src-tauri/src/sim/core.rs:1332-1352`.

- **Failure scenario:** firing the bus fault gets a 5xx on the POST. The fault stays in
  `self.faults`, so the lamps stay lost and the panel shows *Aktiv*, with `alarm_id: None` and
  `track.active = false`. No alarm exists in Cumulocity, so the presenter cannot clear it there. Before D9
  they toggled the fault off; now there is no per-fault clear. The only ways out are "Auslösen" again
  (which re-posts) or "Clear all". The badge also claims an ACTIVE alarm that does not exist.
- **Suggested fix:** under D9 the effect lives exactly as long as the alarm. When its create fails, end
  the fault (`end_fault` without the "Alarm cleared" event, since there was no alarm) and report the
  error. Another option is to keep it but show a "not posted" state. Update
  `failed_alarm_post_does_not_leave_a_dangling_fault` to match.

### 4. Minor: the realtime phase, and with it operation handling, now depends on alarm read access

`src-tauri/src/c8y/realtime.rs:642-670` (subscribe loop), `:679-682`, `:769-777` (`fetch_end`).

- **Failure scenario:** a user may create alarms but not read or subscribe to them, or the open-alarm GET
  answers 403. The subscribe or the GET then ends the phase with `PhaseEnd::Fatal`, and realtime stops for
  the whole session. That also stops `lw_switchGroupMode` / `lw_modbusWrite` from the floorplan, which
  worked before this change. A transient 5xx on the alarm GET makes every connect phase `Lost` with
  backoff, which delays operations too. Session start only logs a warning when the same GET fails
  (`sim/session.rs:214-238`), so the two paths disagree.
- **Suggested fix:** treat the alarm subscription and the open-alarm GET as optional. On 403, log it,
  report a non-fatal `session-error` ("alarm changes are not followed"), and keep handling operations. On
  other errors, retry the GET once and emit `OpenAlarms` only when it succeeded. Add a test to
  `integration_tests.rs` that fails `GET /alarm/alarms` with 403 and asserts that operations still
  arrive.

### 5. Minor: "Clear all" holds up the ordered lane, and a fire right after it can be cleared by the background job

`src-tauri/src/c8y/api.rs:419-495`, `src-tauri/src/sim/outbound.rs:493`.

- **Failure scenario:**
  - After a `202`, the worker polls up to 5 × 400 ms and then sends one PUT per remaining alarm, all on
    the normal lane. Measurements and the **final** operation statuses wait behind it, so the floorplan
    spinner of a confirmed `lw_modbusWrite` keeps turning for a few seconds. When the bulk update fails
    and many alarms are open, there are N sequential PUTs.
  - The presenter clicks "Clear all" and then fires the bus fault at once. If Cumulocity's background
    job is still running after the 5 checks (the listed alarms are then cleared one by one), it can still
    clear the new bus alarm. The fault then ends a moment after it was fired.
- **Suggested fix:** do not poll before the first per-alarm pass. After the `202`, send the per-alarm
  PUTs at once; they are idempotent with the background job. Or move `ClearAlarms` into its own lane, or
  send it to the priority lane with a "barrier" (the session holds new alarm POSTs until the clear has
  answered).
- **Doc nit in the same place:** `api.rs:418` says "Alarms raised after the listing are left alone".
  That is true only for the fallback; the bulk update clears whatever is open when Cumulocity runs it.

### 6. Nit: the start scenario clears every device alarm without the confirmation that "Clear all" asks for

`src/features/control/ControlPanel.tsx:150-166`.

- **Scenario:** one click on "Normal (Start)" clears all open alarms of the device, including alarms
  from other tools. The "Alle aufheben" button asks first (`ConfirmDialog`, destructive), the start
  scenario button does not. This is the required behaviour, but the two buttons do the same destructive
  thing in different ways.
- **Suggested fix:** add a hint to the start button (tooltip or small text such as "hebt alle Alarme
  auf"), or reuse the confirmation.

### 7. Nit: an ACKNOWLEDGED notification that arrives before the create answer is lost

`src-tauri/src/sim/faults.rs:400-424`.

- `alarm_status_in_cloud` matches by id only; while `track.id` is `None`, the status change is dropped
  and the badge says *Aktiv* until the next reconnect. This is unlikely, because a human acknowledges
  later. A fix is to remember the newest open status per id (like `recent_cleared`) and apply it in
  the Ok branch.

### 8. Nit: "Clear all" is hidden in some states

`src/features/control/FaultSections.tsx:102, 117`.

- The button is inside the collapsed `<details>`, and the whole panel returns `null` when the catalog has
  no bus, slave or custom faults. That includes a failed catalog load: `catalog.error` makes the catalog
  `[]`. In that state "Clear all" is not offered, although it does not depend on the catalog. A fix is to
  render the Clear-all row whenever a session runs, and to consider moving it to the `<summary>` line.

### 9. Nit: D9 wording of the bulk update

`docs/DECISIONS.md:172`. `status=ACTIVE|ACKNOWLEDGED` reads like one request with an "or" value. The code
sends two PUTs (`status=ACTIVE`, then `status=ACKNOWLEDGED`), which is the safe reading of the
Cumulocity API. Write "one bulk `PUT` per status".

### 10. Test gaps (in addition to the tests suggested above)

- Finding 1's race (re-post vs. cloud clear), also through `OpenAlarms` while a re-post is in flight.
- The `ClearAll` error path (finding 2).
- The Clear-all flow in the UI has no vitest. The decisions inside `clearAll` (close the dialog, which
  toast to show) are inline in the component; AGENTS.md asks for small pure functions or hooks here.
- A threshold alarm cleared by realtime (not by "Clear all") while above the threshold. Today this is
  covered only indirectly through `clear_all_alarms`, which takes a different code path
  (`alarm_cleared_in_cloud`'s non-fault branch is not exercised).
- A realtime DELETE end to end. The parser is unit-tested (`parses_alarm_notifications`), but the fake
  never emits a DELETE, so the session path for a deleted alarm is untested.

## Resolution

Tests named below are in `src-tauri/src/sim/tests.rs` (module `cloud_clear` unless noted),
`src-tauri/src/c8y/integration_tests.rs` or `src/features/control/faults.test.ts`.

1. **Fixed.** `AlarmTrack.inflight` is now a counter. While a create is in flight,
   `alarm_cleared_in_cloud` only records the id (`recent_cleared`) and `reconcile_open_alarms` skips the
   track; the POST answer decides (cleared id: the fault ends; new id: it goes on). Test
   `firing_again_while_the_old_alarm_is_cleared_follows_the_new_alarm` (also covers the `OpenAlarms`
   re-sync during a re-post). Checked: with the two guards temporarily removed it fails
   ("answer pending": the fault ended on the clear), with them it passes.
2. **Fixed.** The toast now says the clear was queued (`control.allClearing`, was "all cleared"). On a
   failed `ClearAll` the session reports the error, queues `OutboundPayload::ReadOpenAlarms` and takes the
   alarms that are still open over again via `restore_alarms` (`cleared_by_all`). Tests
   `failed_clear_all_restores_the_alarms_still_open`, `runClearAll` (vitest). A dedicated error code was not
   added: the HTTP error of the failed clear is reported as before.
3. **Fixed.** A failed alarm POST with no known id (and no other create in flight) ends the fault without
   the "Alarm cleared" event; a failed re-post of a known alarm leaves it alone. Test
   `failed_alarm_post_does_not_leave_a_dangling_fault` (updated).
4. **Fixed (logging only).** 403 on the alarm subscription or the open-alarm list: logged, alarm changes
   are not followed, operations go on. Other list errors: one retry, then no `OpenAlarms`. 401 stays fatal.
   No extra `session-error` is emitted (realtime messages have no warning variant; it is a log line).
   Tests `forbidden_alarm_access_keeps_operations_flowing`, `open_alarm_list_is_retried_once`.
5. **Fixed in part.** After a `202` or a failed bulk update the per-alarm PUTs start at once (no
   polling, `BULK_CLEAR_*` removed); after a `200` one re-check. The comment is corrected. Test:
   request-order assertion in the alarm test in `integration_tests.rs` (no GET between the last bulk PUT and the
   per-alarm PUTs). Skipped: a separate lane / barrier. The ordered lane is needed so the clear runs behind
   queued alarm POSTs; the remaining cost is N sequential PUTs, and the background job racing a fresh fire
   is documented in D9 as accepted.
6. **Skipped, documented.** Behaviour kept on purpose; D9 now says the start scenario clears without a
   confirmation.
7. **Fixed.** `recent_status` remembers statuses of unknown ids and the create answer applies them. Test
   `acknowledge_before_the_create_answer_is_not_lost`.
8. **Fixed.** The panel always renders the title and the Clear-all row, also with an empty or failed
   catalog; only the fault lists sit in the `<details>` (`control.faultList`).
9. **Fixed.** D9 wording ("one bulk `PUT` per status") and ARCHITECTURE line updated.
10. **Test gaps covered:** the race (1), the `ClearAll` error path (2), `runClearAll` (vitest),
    `threshold_alarm_cleared_in_cumulocity_is_not_raised_again_until_below` (realtime clear of a threshold
    alarm), realtime DELETE end to end (new fake helper `FakeC8y::delete_alarm`, used in
    `cloud_clear::fake::presenter_acks_and_clears_in_cumulocity`).
