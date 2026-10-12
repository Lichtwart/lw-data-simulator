# Phase 2 frontend review

Status: open · 2026-10-12 · scope `src/**` (Rust only as contract via `src/lib/bindings.ts`).

Checks: `npm run typecheck` ok, `npm run lint` ok (104 files), `npm test` ok (12 files / 51 tests),
`npm run build` ok but one JS chunk of **865 kB** (gzip 262 kB) with Vite's ">500 kB" warning.
Visual check: `vite` dev server + Chrome screenshot of `#/login`. It renders correctly: Manrope UI,
FT Polar Light heading, one orange CTA, 1.5 px icons. The tenant select is empty outside Tauri, as
expected. Console: one 404 (favicon) and a "form field should have an id or name" issue.

Known and not repeated below: generator editing is not wired (contract request 4), and custom
items with a select control can't be edited (request 5). Finding 4 is a data-loss bug in that area.

Severity counts: **3 high, 12 medium, 25 low** (40 findings).

## High

1. **`src/app/DeviceWorkspace.tsx:138-143` + `src/features/devices/DeviceFormDialog.tsx:74-84`.** The edit dialog wipes what you type while a session runs. `Workspace` passes a new `device={{…}}` object on every render. The dialog's reset effect depends on `[open, device]`. `Workspace` calls `useSession()`, so it re-renders on every `item-values-changed` tick, and each tick resets name, description and position to the saved values. **Fix:** memoise the values (`useMemo` on `device.id/name/description/position`), or reset only when `open` goes from false to true (keep the previous `open` in a ref). — high

2. **`src/features/demo/CustomEditor.tsx:85-89` + `src/features/demo/ItemEditorDialog.tsx:65-69`.** The item editor draft is reset on every parent render. `rowsToSlaves()` returns a new array each render, so the `useMemo` for `allSlaves` never hits. The dialog's effect depends on `[open, initial, slaves]` and resets the draft every time `CustomEditor` renders. With a session running that is every value tick (see 9), so typing into the dialog is lost. **Fix:** memoise `parsedSlaves` on `[slaveRows, libraryKeys]`, and only initialise the draft when the dialog opens. — high

3. **`src/features/session/SessionProvider.tsx:32-42`, `adopt` at `:53`, used by `useStartSession.tsx:28` / `SessionBar.tsx:51` / `DeviceListScreen.tsx:77`.** Command results and events can overwrite each other. The first `getSession()` and every `adopt(commandResult)` replace the whole snapshot, even if newer events (`session-changed`, `operation-prompt`, `item-values-changed`) arrived while the call was in flight. Example: `start_session` applies the start scenario and the backend fails stale PENDING operations, then the older returned snapshot lands last. A visible operation prompt can disappear from the UI and then time out after 60 s without the presenter seeing it. There is also no cancel flag: a `getSession()` that resolves after logout brings the session back. **Fix:** give `SessionSnapshot` a monotonic `revision` (backend) and drop older snapshots in the reducer. Or don't `adopt` command results at all and rely on `session-changed`, keeping `adopt(null)` only. Add a `cancelled` flag to the `getSession` effect. — high

## Medium

4. **`src/features/demo/CustomEditor.tsx:309-316` + `ItemEditorDialog.tsx:67,72-77`.** Editing an unsupported custom item corrupts the list. For a custom item with a select control (or a native kind), `itemToDraft` returns `null` and the dialog falls back to an empty draft, still in edit mode (`editing = true`, so `checkId: false`, id field disabled and empty). Saving `upsert`s a new item with id `""` next to the original. **Fix:** disable the pencil button when `itemToDraft(it) === null` (with a tooltip), and have `validateDraft` always reject an empty id. — medium

5. **`src/features/demo/ItemEditorDialog.tsx:89-91` + `CustomEditor.tsx:106-112`.** "Generate series" silently overwrites existing items. Generated ids are never checked against `takenIds`, and `upsert` replaces items by id, so a series can overwrite existing custom items. Library ids are not checked either, which breaks rule D6. **Fix:** after `generateItemSeries`, reject the series if any id is in `takenIds` and show `setup.items.idTaken` with the colliding ids. — medium

6. **`src/features/demo/SetupSettingsCard.tsx:31-39`, `CustomEditor.tsx:76-79`.** Unsaved edits on the Setup tab are lost when another card saves. Both cards re-seed their local form state whenever `device.demo` changes. Any other card that saves and calls `reload()` (apply, upgrade, settings, custom) throws away unsaved edits in the others. **Fix:** re-seed only when the form is clean (keep a `dirty` flag), or key the forms on a server revision, and show "unsaved changes" with discard/save. — medium

7. **`src/features/demo/ItemEditorDialog.tsx:373`, `CustomEditor.tsx:321`.** The item editor's "Save" only stages the change. "Save" and the trash icon change local state only; nothing is written until the separate card-level "Save". Nothing shows that there are pending changes, and leaving the tab drops them. **Fix:** name the dialog button "Apply" / "Übernehmen", or persist straight away via `updateDemoSetup`. At least show a dirty badge and a `beforeunload`/route-leave guard. — medium

8. **`src/features/activity/ActivityLogPanel.tsx:26-29` (same pattern in `src/features/raw-fire/RawFirePanel.tsx:54-57`).** The first load overwrites live events. `activity-logged` events that arrive while `activityLog()` is loading are appended, and then `setEntries(initial.data)` throws them away. With a running session that loses entries every time the tab opens. **Fix:** merge by id: `setEntries(cur => mergeById(initial.data, cur))`. Do the same in the raw panel with `upsertHistory`. — medium

9. **`src/features/session/SessionProvider.tsx:57-60`, consumers `AppShell.tsx:20`, `DeviceWorkspace.tsx:41`, `DeviceListScreen.tsx:44`.** The whole screen re-renders on every value tick. Every item-value tick creates a new context value, so `AppShell`, the whole `Workspace` subtree (Setup, Raw, the activity table with 500 rows) and the device list re-render. This is the root cause of findings 1 and 2, and it costs CPU during demos. **Fix:** split the context into a slow part (`deviceId`, `deviceName`, `status`, `realtimeConnected`, `autoAccept`, `pendingOperations`) and a fast part (`items`, `activeFaults`), or use `useSyncExternalStore` with selectors. Only `ControlPanel` should subscribe to values. — medium

10. **`src/features/devices/DeviceFormDialog.tsx:32` → `MapPicker.tsx:1-2`.** The map is not code-split. Leaflet and its CSS are imported statically into the main chunk (865 kB). **Fix:** `const MapPicker = lazy(() => import("./MapPicker"))` inside `<Suspense>` with a fixed-height placeholder. Also consider lazy routes for `DeviceWorkspace` / `SettingsScreen`, and `build.rolldownOptions.output.codeSplitting` for the vendor chunks. — medium

11. **`src/features/updates/useUpdateCheck.ts:21-39`, used in `AppShell.tsx:19` and `SettingsScreen.tsx:20` (PRD §15).** Update-check errors show as "up to date", and two hook copies keep separate state. Any `check()` error (the placeholder pubkey, offline, a 404) becomes "no update", so Settings says "Die App ist aktuell." even though nothing was checked. Each hook instance has its own state: an update found in Settings doesn't show the sidebar badge, and opening Settings runs a second check. A replaced `Update` resource is never `close()`d. **Fix:** move the update state into a single provider or module store, add an `error` state with a "Prüfung fehlgeschlagen" message, and close the old `Update` before replacing it. — medium

12. **`src/features/session/GlobalEvents.tsx:11-16`.** The `session-error` toast is not translated. Its description is the raw backend `${code}: ${message}` (English), against "no hard-coded UI strings". **Fix:** if `code` is an `AppError` code, show `t("errors.<code>")`; keep the raw message only in a details line or the activity log. Add `session.errorCodes.*` keys for session-specific codes. — medium

13. **`src/features/control/ItemControl.tsx:84-86`.** Slider preview problems. (a) A generator tick during a drag changes `value`, which clears `draft`, so the thumb jumps back under the cursor. (b) If `setItemValue` fails, the value doesn't change and the draft stays, so the slider shows a value that was never set. **Fix:** track "dragging" (pointer down → commit) and ignore incoming values while dragging. Pass an `onError` from `ItemCard.run` that clears the draft. — medium

14. **`src/features/demo/DaliGroupsEditor.tsx:68-70,116`, `src/features/demo/DigitalInputsEditor.tsx:110-114,147`.** Rows can get duplicate React keys. Rows are keyed by `base.id` / `base.index`, but `add()` picks the next free number from the *edited* `id` / `index` strings. Edit row 1 from 1 to 2, then add: the new row gets base id 1 → no collision, but editing 2→3 then adding gives base 2, the same as the original row 2. React then mixes up the rows' inputs. **Fix:** give each row a stable client key (`crypto.randomUUID()` or a counter) when it is created. — medium

15. **`src/features/session/OperationPrompts.tsx:96-110` (PRD §9).** Operation prompts are not announced to assistive tech or to a presenter who is looking away. A prompt that expires in 60 s appears silently in a corner. There is no live region, focus never moves, and the region uses `section` without `role="alert"`. **Fix:** announce new prompts through an `aria-live="assertive"` region (summary + seconds), give each card `role="alertdialog"` with `aria-describedby`, and consider a short sound or window-attention request (`getCurrentWindow().requestUserAttention`) when the app is in the background. — medium

## Low

16. **`src/features/demo/ApplyDemoCard.tsx:41-59`.** A late preview can apply to the wrong demo. A preview response for an earlier demo or checkbox setting can land after the selection changed, which enables "Apply" with the wrong conflict list. **Fix:** keep a request token in a ref and ignore stale responses. — low

17. **`src/features/control/ItemControl.tsx:129`.** Fast counter clicks are lost. `+1/+10/+100` send `value + n` from the last rendered value, so quick clicks before the event arrives send the same target. **Fix:** keep an optimistic local total until the backend confirms (or add a backend increment). — low

18. **`src/features/control/SessionBar.tsx:89-90`.** Enter commits the interval, then blur commits it again, so `setSendInterval` is called twice. **Fix:** on Enter, blur the field (`e.currentTarget.blur()`) and commit only in `onBlur`. — low

19. **`src/i18n/de.json` / `en.json` (`activity.count`, `control.activeFaults`, `control.unknownItems`, `apply.previewItems`, `export.done`).** No plural forms, so the UI shows "1 Anfragen" / "1 requests". **Fix:** use i18next `_one` / `_other` keys. — low

20. **`src/features/session/OperationPrompts.tsx:29` + `useNow.ts:7`.** Expired prompts stay at "0 s" until `operation-resolved` arrives, and a lost event leaves the card stuck. The 1 s interval isn't aligned to second boundaries, so the countdown can skip a number. **Fix:** hide or grey out prompts more than a few seconds past `expiresAt`; schedule ticks with `setTimeout(…, 1000 - (Date.now() % 1000))`. — low

21. **`src/i18n/index.ts:37-43`, `index.html:2`.** `<html lang>` is hard-coded to `de` and only updated in `setLanguage`. English users keep `lang="de"`, which affects screen readers and hyphenation. **Fix:** set `document.documentElement.lang = i18n.language` after `init` and on `languageChanged`. — low

22. **`src/i18n/de.json:215-216`.** "Sitzung" in `errors.noSession` / `errors.sessionActive` vs "Session" everywhere else. `errors.sessionActive` also shows a raw device id. **Fix:** use one term throughout ("Session"), and add a device name to the backend error details or drop the id. — low

23. **`src/features/control/ItemCard.tsx:131`.** Generator kinds (`randomWalk`, `sine`, …) are shown untranslated. **Fix:** add `control.generators.<kind>` keys. Severity names can stay technical. — low

24. **`src/features/demo/ApplyDemoCard.tsx:138`.** Hard-coded "id" / "addr" in the slave preview. **Fix:** `t("apply.slaveLine", { key, id, address })`. — low

25. **`src/app/AppShell.tsx:72` + `src/features/auth/AuthProvider.tsx:49-52`.** If `logout()` throws, the rejection goes nowhere (`void`) and the user stays logged in without feedback. **Fix:** clear the local auth in `finally`, and toast `errorMessage(e)`. — low

26. **`src/features/auth/AuthProvider.tsx:55`.** The context value is a new object on every render. **Fix:** `useMemo`. — low

27. **`src/styles/fonts.css:6-32`.** Font bloat. All `@fontsource` subsets (Cyrillic, Greek, Vietnamese) in 5 Manrope + 2 mono weights ship as about 90 font files. FT Polar Regular is bundled although it is never used (the design system only allows Light). **Fix:** import `@fontsource/manrope/latin-{400,500,600,700}.css` (+ `latin-ext`), only the weights in use, and drop the FT Polar 400 `@font-face`. — low

28. **`src/features/settings/SettingsScreen.tsx:33-63`.** The language and theme button groups have no accessible group name (`Label` without a control) and no `aria-pressed`. **Fix:** `role="group" aria-labelledby` + `aria-pressed`, or use `ToggleGroup`. — low

29. **`src/lib/theme.ts:15-20`.** The "System" theme doesn't follow later OS changes. **Fix:** add a `matchMedia("(prefers-color-scheme: dark)")` change listener while the preference is `system`. — low

30. **`src/features/devices/DeviceFormDialog.tsx:70-71`.** The groups request (`[open]`) runs when the list screen mounts and again every time the dialog closes, and in edit mode, where it isn't used. `groups.error` is never shown. **Fix:** load only when `open && !editing`, and show the error. — low

31. **`src/features/demo/CustomEditor.tsx:44-58`.** Deleting a custom slave leaves custom items pointing at a slave key that no longer exists, and validation doesn't notice. **Fix:** in `rowsToSlaves` / save, reject (or list) items whose `slave` is not in `allSlaves`. — low

32. **`src/features/control/FaultSections.tsx:81`.** `summary` uses `list-none`, which hides the open/closed marker. **Fix:** add a chevron that rotates with `group-open:`. — low

33. **`src/features/control/ControlPanel.tsx:126-139`, `FaultSections.tsx:34-39`, `GroupModes.tsx:18-24`.** Scenario buttons, fault switches and group toggles have no pending state, so double clicks send duplicate commands. **Fix:** disable them per control while the call is in flight. — low

34. **`src/features/session/GlobalEvents.tsx:18-27` (PRD §8/§9).** With auto-accept on, `autoAccepted` operations change values with no visible feedback. The scenario `start` flag is not shown. **Fix:** an info toast for `autoAccepted` (and optionally `confirmed`); mark the start scenario. — low

35. **`src/features/control/SessionBar.tsx:70-71`.** "zuletzt gesendet" only refreshes with `session-changed`, so it is stale (contract request 2). Until the backend emits it, consider hiding the field. — low

36. **`src/features/devices/MapPicker.tsx:13`.** OSM recommends `https://tile.openstreetmap.org/{z}/{x}/{y}.png` (no `{s}` subdomains), and its tile policy can block requests without a usable Referer, which is a risk from `tauri://localhost`. If the URL changes, update the CSP in `tauri.conf.json` (`img-src https://*.tile.openstreetmap.org` won't match the bare host). Check on a release build. — low

37. **`docs/ARCHITECTURE.md` §10 vs `src/app/router.tsx:7`.** The docs list tabs `control | raw | activity`; the code also has `setup`. **Fix:** update the doc. — low

38. **`src/features/demo/ExportCard.tsx:79`.** The demo id is checked with the item-id rule (`isValidItemId`, ≤ 19 chars), but demo ids are not register names. **Fix:** a separate `isValidDemoId` (slug, matching the library schema). — low

39. **`src/features/activity/ActivityLogPanel.tsx:81-104` (PRD §16).** There is no "short body" column, and bodies only show after expanding a row. Each event also re-renders all 500 rows. **Fix:** add a truncated body column, and `memo(ActivityRow)`. — low

40. **`src/features/devices/DeviceListScreen.tsx:134,207`.** `now` is computed once per render, so lock badges never expire while the screen is open. "Open" uses a play icon, which suggests it starts a session. **Fix:** use `useNow(60_000)`, and an arrow/open icon. — low

## Resolution

2026-10-12. Checks after the fixes: `npm run typecheck`, `npm run lint`, `npm test` (13 files / 64 tests) and
`npm run build` pass. The build has no chunk-size warning (largest chunk 219 kB: `react`; Leaflet is its own
on-demand chunk). 39 of 40 findings are fixed; 36 is deferred. Backend requests are in
`docs/reviews/frontend-contract-requests.md` (items 7 to 11).

| # | Status | How |
|---|---|---|
| 1 | fixed | `DeviceFormDialog` resets only when `open` turns true (device read via ref); `Workspace` memoises the form values. |
| 2 | fixed | `ItemEditorDialog` initialises the draft only on open; `CustomEditor` memoises `parsedSlaves`. |
| 3 | fixed (frontend) + backend request 7 | New `SessionStore`: events are the source of truth; a command result is adopted only if no event arrived meanwhile (`mark` / `adopt`); `getSession` has a cancel flag; the reducer already drops snapshots with an older `revision`. Needs `SessionSnapshot.revision` from the engine. |
| 4 | fixed | The pencil is disabled for items the editor cannot handle; `validateDraft` rejects an empty id even in edit mode. Select controls are now editable, so only native kinds are blocked. Editing also keeps the item's `generator` / `effects`. |
| 5 | fixed | A generated series is rejected when any id is in `takenIds` (library or custom); the colliding ids are shown. |
| 6 | fixed | `useSeededForm`: a reload re-seeds a card only if it is clean. Cards show "Unsaved changes" and a Discard button. |
| 7 | fixed | Dialog button is "Apply" / "Übernehmen". Dirty badge per card, route-leave guard (`useBlocker`) and `beforeunload` guard on the Setup tab. |
| 8 | fixed | Activity log and raw history merge the first load with live entries by id (`mergeEntries`, `upsertHistory`). |
| 9 | fixed | The session lives in an external store with `useSyncExternalStore`. `useSession()` returns the slow part only (stable head); `useDeviceSession()` (items, faults) is used only by the running control panel. |
| 10 | fixed | `MapPicker` is `lazy` inside `Suspense`; `DeviceWorkspace` and `SettingsScreen` are lazy routes; vendor chunks split in `vite.config.ts` (react, radix, i18n, leaflet, vendor). |
| 11 | fixed | One shared module store for the update state; new `error` state ("Prüfung fehlgeschlagen"); the replaced `Update` is `close()`d. |
| 12 | fixed | `session-error` shows `errors.<code>` when it exists, else `"{{code}}: {{message}}"` (request 11). |
| 13 | fixed | The slider ignores incoming values while dragging; a failed `setItemValue` clears the preview (`onSet` returns success). |
| 14 | fixed | Dali group and digital input rows have stable client keys. |
| 15 | fixed | Always-mounted `aria-live="assertive"` region announces new prompts, cards are `role="alertdialog"` with labelled / described by, window attention is requested when the app is in the background (needs a capability, request 8). |
| 16 | fixed | Preview requests carry a token; stale responses are ignored. |
| 17 | fixed | The counter keeps an optimistic total until the value arrives. |
| 18 | fixed | Enter blurs the field; only `onBlur` commits. |
| 19 | fixed | `_one` / `_other` keys for the five strings. |
| 20 | fixed | Prompts are disabled at 0 s and hidden 5 s after expiry; `useNow` ticks on interval boundaries. |
| 21 | fixed | `<html lang>` follows `languageChanged` and the initial language. |
| 22 | fixed | "Session" everywhere; `errors.sessionActive` no longer prints a device id. |
| 23 | fixed | `control.generators.<kind>` keys. |
| 24 | fixed | `apply.slaveLine`. |
| 25 | fixed | `logout` clears the local auth in `finally`; the shell toasts the error. |
| 26 | fixed | `useMemo` for the auth context value. |
| 27 | fixed | Latin and Latin Extended subsets only, weights 300 to 700 (mono 400, 500); FT Polar Regular removed (font files and `@font-face`). |
| 28 | fixed | Language and theme use `ToggleGroup` with a group label. |
| 29 | fixed | `watchSystemTheme()` listens to the OS preference while the preference is "system". |
| 30 | fixed | Groups load only when `open && !editing`; the error is shown. |
| 31 | fixed | Save is disabled and the item ids are listed when a custom item points to a missing slave. |
| 32 | fixed | Chevron that rotates with `group-open:`. |
| 33 | fixed | `usePending` disables scenario buttons, fault switches and group toggles per control while a call runs. The per-lamp fault menu on item cards is not covered. |
| 34 | fixed | Info toast for `autoAccepted`; the start scenario is marked "(Start)". |
| 35 | fixed (hidden) + request 10 | The "last sent" text is not shown until the backend emits it. |
| 36 | fixed | CSP allows `https://tile.openstreetmap.org`; `OSM_TILES` uses the bare host. Verify tile loading on a release build (Referer). |
| 37 | fixed | `docs/ARCHITECTURE.md` lists `control | setup | raw | activity`. |
| 38 | fixed | `isValidDemoId` (slug, as in `demo.schema.json`). |
| 39 | fixed | "Body" column (error, response or request, truncated); `ActivityRow` is `memo` with stable callbacks. |
| 40 | fixed | `useNow(60_000)` for lock expiry; the open button uses an arrow icon. |

Also done (not a review finding): generator editing on the Control tab (`GeneratorDialog`: all four kinds, apply to
session or save to `lw_demo.generators`) and select controls in the custom item editor (contract requests 4 and 5).
