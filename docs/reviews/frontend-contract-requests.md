# Frontend contract requests (Phase 2C)

Status: all items done or confirmed, 2026-10-12. The engine landed and the frontend uses the new contract (see `engine-requests.md`, "Frontend follow-ups").

1. **DONE (engine added `DemoDeviceSummary.upgradeAvailable`; client-side `isOlderVersion` removed).** **`DemoDeviceSummary` has no `upgradeAvailable`.** The device list computes the "Upgrade available" badge
   client-side: `libraryVersion` (device) older than `app_info().libraryVersion` (`isOlderVersion` in
   `src/lib/format.ts`). `DemoDevice.upgradeAvailable` (workspace header) comes from the backend. Request: add
   `upgradeAvailable: bool` to `DemoDeviceSummary` so both use one rule (e.g. also covering changed floorplans).
2. **DONE (`session-changed` after every periodic send; "last sent" shown in `SessionBar`).** **`SessionSnapshot.lastSentAt` is only fresh when a `session-changed` event arrives.** The session bar shows
   "last sent …" from it. Request: emit `session-changed` (or a small `last-sent` event) after each periodic send,
   or the field stays at its value from the last config change.
3. **DONE (`reason` is optional; the UI sends `{ decision: "abort" }`, backend default `Aborted by user`).** **Operation abort reason.** `answer_operation(abort)` needs a reason text; the UI sends the constant
   `"Aborted by user"` (`ABORT_REASON` in `features/session/OperationPrompts.tsx`). It goes to Cumulocity, so it is
   not translated. Request: if the backend prefers its own default, make `reason` optional.
4. **CONFIRMED (replace semantics; a running session picks up generators, interval and realistic mode; item selection applies at the next start).** **Generator editing (done in the UI, 2026-10-12).** Control tab, per item: `GeneratorDialog` ("Apply to session" =
   `set_item_generator`, "Apply and save to demo" = `set_item_generator` + `update_demo_setup { generators }`).
   Assumption to confirm in the engine: `update_demo_setup.generators` **replaces the whole override map** (the UI
   sends `{ ...demo.generators, [itemId]: config }`), and a running session on the device picks it up.
   There is no way to remove a single override (back to the library default); if wanted, accept `null` values in the
   map or add `reset_item_generator`.
5. **DONE (backend validates unique option values and `default`).** **Select controls in custom items (done in the UI, 2026-10-12).** The item editor edits `Control::Select`
   (options value / de / en, default must be an option value). Backend: please validate that select option values are
   unique and that `default` is one of them for custom Modbus items too.
6. **DONE (`deviceId` on `operation-prompt` and `operation-resolved`; the reducer ignores other devices).** **`operation-resolved` has no `deviceId`.** Fine while only one session runs; the toasts name the operation type only.
7. **DONE (`revision` on the snapshot and all four events; `sessionReducer` drops events not newer than the snapshot; `revisionOf` uses the typed field).** **`SessionSnapshot.revision` (review finding 3).** Add `revision: u32` (monotonic per session, incremented on
   *every* state change the UI can observe: every `session-changed`, `operation-prompt`, `operation-resolved`,
   `fault-changed`, `item-values-changed` batch, `realtime-status`). Put the same counter on the snapshot returned by
   `start_session` / `get_session` and on every `session-changed` payload. Optional but preferred: also add
   `revision` to `item-values-changed`, `fault-changed`, `operation-prompt`, `operation-resolved` payloads, so the UI can
   drop events that are older than an adopted snapshot. Frontend today: `session-changed` / events are the source of
   truth, a command result is adopted only if no event arrived while it was in flight (`SessionStore.adopt` in
   `src/features/session/SessionProvider.tsx`), and `sessionReducer` already ignores a snapshot whose `revision` (read
   defensively via `revisionOf`) is lower than the current one. After `bindings.ts` has the field, replace the cast in
   `revisionOf` with the typed field.
8. **DONE - Window attention capability (review finding 15).** `OperationPrompts` calls
   `getCurrentWindow().requestUserAttention(Critical)` when a prompt arrives while the app is not focused. It needs the
   permission `core:window:allow-request-user-attention` in `src-tauri/capabilities/*.json`; until it is added the call
   fails silently (the live region still announces the prompt).
9. **DONE (CSP + OSM_TILES switched; release-build Referer check still open) - Tile server URL and CSP (review finding 36).** `MapPicker` still uses `https://{s}.tile.openstreetmap.org/...`
   because the CSP in `tauri.conf.json` is `img-src ... https://*.tile.openstreetmap.org`. OSM asks clients to use
   `https://tile.openstreetmap.org/{z}/{x}/{y}.png` (no subdomains). Request: add `https://tile.openstreetmap.org` to
   `img-src`, tell the frontend, then change `OSM_TILES` in `src/features/devices/MapPicker.tsx`. Also verify on a
   release build that tiles load from `tauri://localhost` (OSM tile policy wants a usable Referer).
10. **DONE (see 2; "last sent" re-enabled, `control.lastSent` key added).** **Periodic send event (review finding 35).** The session bar's "last sent" text is hidden until the backend emits
    `session-changed` (or a small `last-sent` event) after each periodic send; see request 2. Re-enable the text in
    `SessionBar.tsx` (`control.lastSent_*` key was removed from the i18n files) once it does.
11. **DONE (`session-error.code` is always an `AppError` code; repeats of one code throttled to one per 30 s).** **`session-error.code`.** The UI translates it with `errors.<code>` when the code is an `AppError` code (message
    passed as `{{details}}`); other codes show `"{{code}}: {{message}}"`. Please emit `AppError` codes (or tell the UI
    the list of session-specific codes so `errors.*` keys can be added).
12. **DONE - `authFailed` details.** `AppError::AuthFailed` carries optional `details`; `errorMessage` appends it in parentheses.
13. **DONE - Sensor faults.** New `FaultTarget` case `sensor { sensorKind, index, alarm }`; shown in the fault menu of the brightness / motion item cards (`itemFaultOwner` / `itemFaultKeyOf` in `features/control/faults.ts`).
