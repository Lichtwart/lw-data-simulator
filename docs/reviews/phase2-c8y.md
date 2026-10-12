# Phase 2 Cumulocity I/O review — 2026-10-12

Scope: `src-tauri/src/c8y/**`, `src-tauri/src/raw.rs`, `src-tauri/src/commands/{auth,activity,raw}.rs`,
`docs/ARCHITECTURE.md` §15.

Checks: the tree does not compile right now because `src/library/mod.rs:27` declares
`mod schema_tests;` and the file does not exist (another agent is still working on it). In a scratch copy with an
empty `schema_tests.rs`: `cargo test c8y` passes (34 tests), `cargo test raw` passes (4), and
`cargo clippy --all-targets -- -D warnings` is clean. The 11 integration tests passed 8 runs in a row (about 2 s
each, no flakes seen).

Checked and fine (no finding):
- **Does the 30 s client timeout kill the 50 s long-poll? No.** In reqwest 0.13.5, `RequestBuilder::timeout`
  "affects only this request and overrides the timeout configured using `ClientBuilder::timeout()`", and it
  covers the body as well. Connects use `poll_timeout + poll_grace` = 65 s (`realtime.rs:393,397`,
  `client.rs:301-303`). Finding 5 asks for a test that keeps it that way.
- Basic auth with `UseXBasic: true` on every request. The password is only in `C8yClient`, whose custom `Debug`
  hides it. It never reaches the activity log, and the curl export prints `user:****`.
- Media types for inventory, child additions, measurements (single vs collection), alarms, events, operations
  and currentUser are right. The binary upload is multipart with `object` `{name,type}`, `filesize` and `file`,
  and reqwest sets the boundary. `failureReason` is sent only with FAILED.
- Query values are percent-encoded per RFC 3986 (`$`, `(`, `'`, space and `,` are all encoded). Comma lists
  work for alarm `status` and `type`.
- 401 handling: `current_user` maps 401 to `AuthFailed` without an `auth-expired` event, so a failed login does
  not fire it. Every other 401, realtime included, reports `auth-expired`.
- Locks: `ActivityLog` uses a std `Mutex` that is never held across `.await`. `AppState::client()` clones the
  client and drops the guard. `expire_auth` drops the write guard before it awaits `stop_session`.
- `truncate_body` cuts on a UTF-8 character boundary (tested with `ä`). A non-UTF-8 response is logged as
  `[N bytes]`.

## High

1. **c8y/realtime.rs:333, 371, 419-423 — re-handshake has no backoff, so it can spin forever.**
   `PhaseEnd::Rehandshake` loops straight back (`continue` at :333). If the server keeps answering
   `402::Unknown client` or `advice.reconnect = "handshake"`, the worker runs handshake, subscribe,
   `GET` PENDING operations, emits `Connected`, then connects again, as fast as the network allows. That would
   hammer the production tenant, wipe the 500-entry activity log in seconds, and make the `realtime-status`
   event flap. Causes we can expect: a load balancer without sticky sessions (Bayeux keeps client state per
   node, and reqwest keeps no cookies by default), a server restart loop, or a subscribe that is rejected with
   `reconnect: handshake`.
   **Fix:** count re-handshakes in a row. Allow the first one at once; from the second on, sleep `backoff` and
   double it, exactly like `Lost`. Reset the count only after a `/meta/connect` reply with `successful: true`.
   Also consider `reqwest::ClientBuilder::cookie_store(true)` (feature `cookies`) so server affinity cookies such
   as `BAYEUX_BROWSER` or a load-balancer cookie are sent back. Add a fake mode that answers every connect with
   402, and assert that fewer than N handshakes happen in 1 s.

## Medium

2. **c8y/realtime.rs:381 — backoff is reset before the connection is proven.**
   `*backoff = backoff_min` runs right after the PENDING `GET`, before any `/meta/connect` has worked. If
   connect fails every time (HTTP 500, or `successful:false` with a non-402 error), each attempt restarts at 1 s.
   That gives a steady loop of handshake, subscribe, PENDING GET, `Connected`, `Disconnected` every second,
   without ever backing off.
   **Fix:** reset `backoff` (and emit `Connected`) only after the first successful `/meta/connect` reply, or
   once the phase has lived longer than `backoff_max`. Keep sending the PENDING operations after `Connected`.

3. **c8y/sink.rs:48-55 (+ state.rs:159-173) — `auth-expired` is sent even when nothing expired.**
   `expire_auth` correctly ignores a 401 from a client of another tenant, or when already logged out. But
   `TauriSink::auth_expired` emits `AuthExpired` anyway. So a 401 from a stale client (an old session's
   in-flight request, a realtime retry, or a 401 that lands after a switch to another tenant) sends the UI to
   the login screen while the backend is still logged in. Several concurrent 401s also send several events.
   **Fix:** make `expire_auth` return `bool` (true = it cleared the login) and emit only then.

4. **c8y/realtime.rs:11-14 + sim/session.rs — the duplicate handling the realtime contract promises does not
   exist yet.** The module promises that "the session de-duplicates by id / status". Every reconnect sends all
   PENDING operations again. A CREATE can arrive both in the PENDING list and on the channel. Our own PUTs
   (EXECUTING, SUCCESSFUL) also come back as UPDATE messages. Nothing consumes `subscribe_operations` yet, and
   no code de-duplicates. If this is forgotten when the session is wired up, the same op gets answered twice or
   a confirm prompt shows twice (PRD §9).
   **Fix:** either de-duplicate in the worker (a `HashMap<id, last status>` sent per subscription, dropping a
   message whose status matches the last one sent), or add a named helper in `sim::operations` and a test for
   it. Document that UPDATE echoes of our own status changes must be ignored. A FAILED status must win (D5).

5. **c8y/integration_tests.rs:339-346, c8y/client.rs:20,219 — no test covers the long-poll vs client
   timeout, the timeout path, or a 401 during realtime.**
   `REQUEST_TIMEOUT` is a fixed 30 s, so no test can show that a connect held longer than the client timeout
   survives. If someone drops `.timeout(t)` in `send_raw`, every production long-poll dies at 30 s and the
   connection flaps, and the tests still pass. The `Post::Timeout` branch ("two in a row means lost") is never
   run, and neither is a 401 on `/notification/realtime`.
   **Fix:** make the client timeout configurable (for example `C8yClient::with_timeouts` or a field in
   `RealtimeConfig`). Add tests that:
   (a) set the client timeout to 200 ms and hold the connect for 500 ms, and expect no `Disconnected`;
   (b) set `connect_hold` above `poll_timeout + poll_grace` while the fake ignores `advice.timeout`, and expect
   `Disconnected("connect: timed out")` after 2 polls;
   (c) rotate the password, then expect exactly one `auth_expired` call and that the worker stops retrying
   (see 7).

## Low

6. **c8y/realtime.rs:162, 417-427 — server `advice` is ignored.** `advice.interval` (wait before the next
   connect) and `advice.timeout` from handshake and connect replies are not used. `reconnect: "none"` is treated
   like retry. The handshake sends `timeout: 60000` but connects ask for 50 000.
   **Fix:** store the latest advice. Sleep `interval` before connecting. Set the HTTP timeout to
   `max(poll_timeout, advice.timeout) + grace`. On `none`, emit `Disconnected` and stop.

7. **c8y/realtime.rs:316-318, 372 — 401 and 403 are retried forever.** After a 401 the worker emits
   `Disconnected` and retries with backoff, which means more failed logins (Cumulocity can lock the user or IP
   after repeated failures). This only stops if the session drops the subscription. A subscribe rejected with
   `403::Forbidden` (missing role) is also retried every 30 s with no clear error.
   **Fix:** make 401 terminal: end the task after `check_unauthorized`. For 403 on handshake or subscribe, emit
   `Disconnected("permission denied: …")` and stop, or use backoff_max with a clear message.

8. **c8y/realtime.rs:105-110, 183 — no `/meta/disconnect` on drop or re-handshake.** Server-side clients and
   subscriptions stay alive until the server times them out. `bayeux::disconnect` exists but is never called.
   **Fix:** on cancel, spawn a fire-and-forget `/meta/disconnect` with a 2 s timeout before aborting.

9. **c8y/api.rs:132-166 — paging stops at the first short page, and the 50-page cap cuts results without a
   word.** This is fine against Cumulocity, but the brief asked for `withTotalPages`. If a page is ever short
   because of permission filtering, results are silently cut. Hitting `MAX_PAGES` (100 000 objects) also returns
   a partial list with no error.
   **Fix:** send `withTotalPages=true` on the first page and stop when `currentPage >= statistics.totalPages`.
   Fall back to the short-page rule if the field is missing. On hitting `MAX_PAGES`, log `warn!` or return an
   error.

10. **c8y/api.rs:105-126 — `query` is combined with `type`, `fragmentType` and `text`.** Cumulocity's inventory
    treats the query-language parameter separately, and other filters may be ignored when `query` is set
    (needs checking against the tenant). The fake applies all filters (fake.rs:563-583), so tests would not
    notice.
    **Fix:** when `query` is set, fold `type` and `fragmentType` into the `$filter` (`type eq '…'`,
    `has(…)`), or reject the mix in `params()`.

11. **c8y/client.rs:29-46 + PRD §5 — the own-domain check never matches the real tenants.** Production
    `lichtwart.ram.m2m.telekom.com` is the tenant's own domain, but the code only treats `https://<tenantId>.…`
    as own. So `tenantId/user` is always sent. That works on every Cumulocity domain, so nothing breaks, but the
    doc comment and PRD wording do not match what the code does.
    **Fix:** either always send `tenantId/user` (simplest, and correct everywhere) and update the comment, or
    add an optional `ownDomain: bool` per tenant in `tenants.json`.

12. **c8y/client.rs:489-495 — `AuthFailed` loses the server's reason.** "User account is locked", "password
    expired" and "TFA required" all show as a generic failure.
    **Fix:** `AuthFailed { message: Option<String> }` filled from `error_message(bytes)`.

13. **model/auth.rs:34-43 — `LoginRequest` derives `Debug` with the password.** Any future
    `log::debug!("{req:?}")` or tracing of command arguments would print the password.
    **Fix:** write a manual `Debug` that prints `password: "****"`. Optionally zeroize on drop.

14. **commands/auth.rs:64-71 — a keychain failure blocks a valid login.** With `remember`, a keychain error
    (user refuses the macOS prompt, locked keychain) returns `Keychain` and the user is not logged in.
    **Fix:** log in anyway, and return or emit a warning that "remember me" failed (PRD §5 calls it optional).

15. **c8y/api.rs:254-272 — the upload uses the default 30 s timeout.** A multi-MB floorplan image on a slow
    link times out.
    **Fix:** `spec.timeout(Duration::from_secs(120))` for `upload_binary` and `download_binary`.

16. **c8y/activity.rs:64-77 — curl headers can differ from what was sent.**
    - `download_binary` sends `Accept: */*`, but curl prints `managedobject+json`.
    - A body truncated at 2 kB no longer parses, so `Null` is passed and a measurement collection gets the
      single-measurement `Content-Type`.
    **Fix:** store `accept` / `content_type` (not credentials) in `ActivityEntry`, or at least a flag. When the
    body is truncated, add `# body truncated` to the command.

17. **c8y/client.rs:99 — `error_message` does not shorten `error` when there is no `message`.**
    **Fix:** `truncate_body(e, 300)`.

18. **raw.rs:119-128 — the event `extra_json` can overwrite `type`, `text` and `time`.** For example,
    `{"type":""}` gets past the "type required" check.
    **Fix:** insert `type`, `text` and `time` after merging the extras, or reject those keys in extras.
    `source` is already protected.

19. **raw.rs:137-154 + media.rs:100 — `{"measurements": <not an array>}`.** The body gets no `time` or
    `source`, but `media::for_request` still picks the collection type, because it only checks that the key
    exists.
    **Fix:** in `build`, return `InvalidInput("measurements must be an array")`. In media, check
    `is_array()`.

20. **c8y/api.rs:363-371 — `clear_open_alarms` stops at the first failed PUT** and leaves the rest open,
    without saying how many it cleared.
    **Fix:** keep going, collect the errors, and return the count plus the first error. Or use the bulk
    `PUT /alarm/alarms?source=…&status=ACTIVE` with `{status: CLEARED}`.

21. **c8y/credentials.rs:64-67, 110-118 — the saved-logins file is written in place.** A crash halfway can
    corrupt it, which empties the list. `delete` removes the user name before deleting the keychain entry, so
    a keychain failure leaves an orphaned secret.
    **Fix:** write to `saved-logins.json.tmp` and then `rename`. Delete the keychain entry first, then the user
    name.

22. **c8y/integration_tests.rs:440-446 — race in the re-handshake test.** The missed op is pushed after
    `drop_realtime_clients()` has already woken the long-poll. On a multi-thread runtime the worker could
    in theory finish its PENDING `GET` first.
    **Fix:** push the op before calling `drop_realtime_clients()`.

23. **c8y/integration_tests.rs:93-108 — the offline test reuses a port that was just freed.** A parallel test
    can bind the same random port, and then the request does not fail with `Network`.
    **Fix:** use a base URL that can never connect, such as `http://127.0.0.1:1`, or a listener that is bound
    and kept but never accepts.

24. **c8y/fake.rs:399-425 — the fake's query parser accepts unknown terms.** `matches_query` returns `true`
    for any term it cannot parse (`or`, `ne`, `bygroupid(...)`), so a wrong query in code under test still
    "matches".
    **Fix:** panic, or return `false` with a clear message, on unsupported syntax.

25. **c8y/realtime.rs:113-118 — `subscribe_operations` is `async` but awaits nothing,** while
    `subscribe_operations_with` is sync.
    **Fix:** make both sync (or both async) so the API is consistent.

26. **docs/ARCHITECTURE.md:366-417 — §15 leaves out several things:** `RealtimeConfig` and the default timing
    (50 s poll + 15 s grace, overriding the 30 s REST timeout), `RequestSpec::timeout`, the
    de-duplication duty (finding 4), and what happens after `auth-expired`.
    **Fix:** add a short "Realtime" paragraph and a row for `RealtimeConfig`.

## Resolution

Checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean; `cargo test -- c8y raw credentials auth`
51 tests pass (5 runs, about 2 s each).

1. **Fixed.** `Worker::run` counts re-handshakes in a row: the first is immediate, the next ones wait `backoff`
   (doubling). Count and backoff reset only after a successful `/meta/connect` reply (or a phase longer than
   `backoff_max`). The client now has `cookie_store(true)` (reqwest `cookies` feature). Test
   `endless_unknown_client_is_backed_off` (fake knob `always_unknown_client`): 2..12 handshakes in 1 s.
2. **Fixed, with one deliberate difference.** Backoff resets only when the phase is proven (see 1). `Connected` is
   still emitted right after the PENDING `GET`, because the first connect can be held for the whole 50 s long-poll
   and the UI would show "offline" that long; with the backoff the flapping loop is gone.
3. **Fixed.** `AppState::expire_auth` returns `bool`; `TauriSink` emits `auth-expired` only on `true`.
4. **Fixed in the worker.** `OpDedup` (per subscription, per id): deliver only if the status is further along
   (PENDING < EXECUTING < SUCCESSFUL < FAILED, so FAILED wins). Unit test plus
   `realtime_does_not_redeliver_known_operations`. The session still has to ignore the echo of its own status
   changes (new status = delivered); documented in the module docs and ARCHITECTURE §15.
5. **Fixed.** `C8yClient::with_request_timeout`, fake `client_with_timeout`. Tests: `long_poll_outlives_the_client_timeout`
   (a), `two_connect_timeouts_in_a_row_are_a_lost_connection` (b, fake knob `ignore_advice_timeout`),
   `unauthorized_realtime_reports_once_and_stops` (c).
6. **Fixed.** `interval` is slept before each connect (capped 60 s), `timeout` raises the HTTP timeout to
   `max(poll_timeout, advice.timeout) + grace`, `reconnect: none` ends the task with `Disconnected`; the handshake
   now asks for the same `poll_timeout` as connects. Parsing is unit-tested; the sleep path has no integration test.
7. **Fixed.** 401 (and 403 on any realtime step, PENDING `GET` included, and a `403::` Bayeux error) is terminal:
   `Disconnected(...)` then the channel closes. Tests for 401 and 403.
8. **Fixed.** Cancel sends a best-effort `/meta/disconnect` (2 s) from the task (`Drop` no longer aborts it); a
   double connect timeout also disconnects. Asserted in the drop test and in (b). Not sent after a 402 (the server
   already forgot the client).
9. **Fixed.** First page sends `withTotalPages=true`; stops at `statistics.totalPages`, else the short-page rule;
   `warn!` when `MAX_PAGES` cuts the result. Fake returns `totalPages` on request.
10. **Fixed (reject).** `query` together with `type` / `fragmentType` / `text` returns `InvalidInput`; tested.
11. **Fixed.** `basic_auth_user(tenant_id, user)` always sends `tenantId/user`; comment, test and ARCHITECTURE
    updated. **Open for the owner of `docs/PRD.md`:** §5 (line 53) still describes the own-domain rule.
12. **Fixed.** `AppError::AuthFailed(Option<String>)` with the server reason (tested). Note: this changes the
    generated TypeScript type to `{ code: "authFailed"; details: string | null }`; `src/lib/bindings.ts` needs
    regenerating (not touched here, outside my scope). The code string is unchanged, so UI mapping still works.
13. **Fixed.** Manual `Debug` for `LoginRequest` (`password: "****"`), tested. No zeroize.
14. **Partly fixed.** A keychain failure no longer blocks the login; it is logged with `warn!`. Telling the UI
    ("remember me failed") needs a new field or event in the IPC contract, so it is deferred.
15. **Fixed.** `BINARY_TIMEOUT` = 120 s for upload and download.
16. **Fixed without a model change.** `to_curl` uses `Accept: */*` for binary downloads and adds `# body truncated`;
    a cut `{"measurements"…` body still gets the collection `Content-Type`. Tested. (No `accept` field in
    `ActivityEntry`, to keep the IPC type stable.)
17. **Fixed.** `error` alone is truncated to 300.
18. **Fixed.** Event extras are merged first; `type`, `text`, `time`, `source` are set after. Tested.
19. **Fixed.** `build` returns `InvalidInput("measurements must be an array")`; `media::for_request` checks
    `is_array()`. Tested in `build`.
20. **Fixed.** `clear_open_alarms` tries all alarms, logs the count, returns the first error; 401 stops at once.
    Tested.
21. **Fixed.** Temp file + `rename`; `delete` removes the keychain entry first. Tested (temp file gone).
22. **Fixed.** Op is pushed before `drop_realtime_clients()`.
23. **Fixed.** Offline test uses `http://127.0.0.1:1`.
24. **Fixed.** Unsupported query terms and `or` panic with a clear message (two `should_panic` tests).
25. **Fixed.** `subscribe_operations` is sync now.
26. **Fixed.** ARCHITECTURE §3 (client / realtime rows) and §15 (new "Realtime" section with the `RealtimeConfig`
    table, de-dup duty, `auth-expired` behaviour; fake knobs).

Touched outside the listed scope (small, needed): `src/error.rs` (12), `src/state.rs` (3), `src/model/auth.rs` (13).
