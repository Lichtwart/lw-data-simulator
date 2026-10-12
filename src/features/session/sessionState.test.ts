import { describe, expect, it } from "vitest";

import type { ActiveFault, ItemRuntimeState, OperationPrompt, SessionSnapshot } from "@/lib/api";

import { createSessionStore } from "./SessionProvider";
import { headOf, initialSessionState, sessionReducer } from "./sessionState";

const item = (itemId: string, value: number): ItemRuntimeState => ({
  itemId,
  value,
  manualOverride: false,
  generator: null,
  naming: null,
  paused: false,
});

const snapshot = (over: Partial<SessionSnapshot> = {}): SessionSnapshot => ({
  revision: 0,
  deviceId: "d1",
  deviceName: "Demo",
  status: "running",
  startedAt: "2026-10-12T12:00:00Z",
  autoAccept: false,
  realisticMode: false,
  sendIntervalSec: 30,
  lastSentAt: null,
  realtimeConnected: true,
  restartNeeded: false,
  items: [item("a", 1), item("b", 2)],
  groups: [],
  activeFaults: [],
  pendingOperations: [],
  ...over,
});

const prompt = (id: string, revision = 1): OperationPrompt => ({
  revision,
  deviceId: "d1",
  operationId: id,
  kind: "switchGroupMode",
  summary: "Group 1 → On",
  detail: { kind: "groupMode", group: 1, mode: "on" },
  by: "sl",
  receivedAt: "2026-10-12T12:00:00Z",
  expiresAt: "2026-10-12T12:01:00Z",
});

const start = () => sessionReducer(initialSessionState, { type: "snapshot", session: snapshot() });

describe("sessionReducer", () => {
  it("merges item values and keeps unchanged items identical", () => {
    const s = start();
    const next = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 1,
        deviceId: "d1",
        values: [{ itemId: "a", value: 5, manualOverride: true }],
      },
    });
    expect(next.session?.items[0]).toMatchObject({ itemId: "a", value: 5, manualOverride: true });
    expect(next.session?.items[1]).toBe(s.session?.items[1]);
  });

  it("returns the same state when nothing changed or the device differs", () => {
    const s = start();
    const same = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 2,
        deviceId: "d1",
        values: [{ itemId: "a", value: 1, manualOverride: false }],
      },
    });
    expect(same).toBe(s);
    const other = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 3,
        deviceId: "other",
        values: [{ itemId: "a", value: 9, manualOverride: false }],
      },
    });
    expect(other).toBe(s);
  });

  it("adds, replaces and removes faults", () => {
    const target = { kind: "slaveTimeout", slaveKey: "k" } as const;
    const fault: ActiveFault = {
      target,
      alarmType: "t",
      alarmId: null,
      raisedAt: "x",
      status: "ACTIVE",
    };
    let s = sessionReducer(start(), {
      type: "fault",
      payload: { revision: 4, deviceId: "d1", target, fault },
    });
    expect(s.session?.activeFaults).toHaveLength(1);
    s = sessionReducer(s, {
      type: "fault",
      payload: { revision: 5, deviceId: "d1", target, fault: { ...fault, alarmId: "42" } },
    });
    expect(s.session?.activeFaults).toEqual([{ ...fault, alarmId: "42" }]);
    // Acknowledged in Cumulocity: same fault, new status (D9).
    const acked = { ...fault, alarmId: "42", status: "ACKNOWLEDGED" } as const;
    s = sessionReducer(s, {
      type: "fault",
      payload: { revision: 6, deviceId: "d1", target, fault: acked },
    });
    expect(s.session?.activeFaults).toEqual([acked]);
    s = sessionReducer(s, {
      type: "fault",
      payload: { revision: 7, deviceId: "d1", target, fault: null },
    });
    expect(s.session?.activeFaults).toEqual([]);
  });

  it("tracks operation prompts without duplicates", () => {
    let s = sessionReducer(start(), { type: "prompt", prompt: prompt("o1", 11) });
    s = sessionReducer(s, { type: "prompt", prompt: prompt("o1", 12) });
    s = sessionReducer(s, { type: "prompt", prompt: prompt("o2", 13) });
    expect(s.session?.pendingOperations.map((p) => p.operationId)).toEqual(["o1", "o2"]);
    s = sessionReducer(s, {
      type: "resolved",
      payload: {
        revision: 20,
        deviceId: "d1",
        operationId: "o1",
        operationType: "lw_x",
        status: "SUCCESSFUL",
        resolution: "confirmed",
        reason: null,
      },
    });
    expect(s.session?.pendingOperations.map((p) => p.operationId)).toEqual(["o2"]);
  });

  it("tracks realtime status and clears everything when the session ends", () => {
    let s = sessionReducer(start(), {
      type: "realtime",
      status: { revision: 1, deviceId: "d1", connected: false, error: "boom" },
    });
    expect(s.session?.realtimeConnected).toBe(false);
    expect(s.realtimeError).toBe("boom");
    s = sessionReducer(s, { type: "snapshot", session: null });
    expect(s).toEqual(initialSessionState);
  });

  it("ignores a realtime event of another device or an older revision (F29)", () => {
    const s = start();
    const other = sessionReducer(s, {
      type: "realtime",
      status: { revision: 5, deviceId: "d2", connected: false, error: "late" },
    });
    expect(other).toBe(s);
    const stale = sessionReducer(s, {
      type: "realtime",
      status: { revision: 0, deviceId: "d1", connected: false, error: "late" },
    });
    expect(stale).toBe(s);
    const none = sessionReducer(initialSessionState, {
      type: "realtime",
      status: { revision: 1, deviceId: "d1", connected: false, error: "x" },
    });
    expect(none).toBe(initialSessionState);
  });

  it("a restart flag change makes a new head", () => {
    const s = start();
    const head = headOf(s.session, null);
    const next = sessionReducer(s, {
      type: "snapshot",
      session: snapshot({ revision: 3, restartNeeded: true }),
    });
    expect(headOf(next.session, head)).not.toBe(head);
    expect(headOf(next.session, head)?.restartNeeded).toBe(true);
  });

  it("ignores prompts when no session runs", () => {
    expect(sessionReducer(initialSessionState, { type: "prompt", prompt: prompt("o") })).toBe(
      initialSessionState,
    );
  });

  it("drops snapshots older than the current one", () => {
    const withRev = (revision: number, sendIntervalSec: number) =>
      snapshot({ sendIntervalSec, revision });
    let s = sessionReducer(initialSessionState, { type: "snapshot", session: withRev(5, 30) });
    s = sessionReducer(s, { type: "snapshot", session: withRev(4, 10) });
    expect(s.session?.sendIntervalSec).toBe(30);
    s = sessionReducer(s, { type: "snapshot", session: withRev(6, 20) });
    expect(s.session?.sendIntervalSec).toBe(20);
  });

  it("drops events not newer than the snapshot revision", () => {
    const s = sessionReducer(initialSessionState, {
      type: "snapshot",
      session: snapshot({ revision: 10 }),
    });
    const stale = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 10,
        deviceId: "d1",
        values: [{ itemId: "a", value: 99, manualOverride: false }],
      },
    });
    expect(stale).toBe(s);
    const fresh = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 11,
        deviceId: "d1",
        values: [{ itemId: "a", value: 99, manualOverride: false }],
      },
    });
    expect(fresh.session?.revision).toBe(11);
  });

  it("keeps the head reference while only item values change", () => {
    const s = start();
    const head = headOf(s.session, null);
    const next = sessionReducer(s, {
      type: "values",
      payload: {
        revision: 7,
        deviceId: "d1",
        values: [{ itemId: "a", value: 5, manualOverride: false }],
      },
    });
    expect(headOf(next.session, head)).toBe(head);
    const prompted = sessionReducer(next, { type: "prompt", prompt: prompt("o1", 99) });
    expect(headOf(prompted.session, head)).not.toBe(head);
  });
});

describe("session store", () => {
  it("ignores a command result when an event arrived while it was in flight", () => {
    const store = createSessionStore();
    const mark = store.mark();
    store.dispatch({ type: "snapshot", session: snapshot({ sendIntervalSec: 60 }) });
    store.adopt(snapshot({ sendIntervalSec: 5 }), mark);
    expect(store.getState().session?.sendIntervalSec).toBe(60);
  });

  it("takes over a command result when no event arrived", () => {
    const store = createSessionStore();
    store.adopt(snapshot(), store.mark());
    expect(store.getState().session?.deviceId).toBe("d1");
    expect(store.getHead()?.deviceName).toBe("Demo");
  });

  it("notifies subscribers and keeps a stable head across value ticks", () => {
    const store = createSessionStore();
    store.dispatch({ type: "snapshot", session: snapshot() });
    const head = store.getHead();
    let calls = 0;
    store.subscribe(() => calls++);
    store.dispatch({
      type: "values",
      payload: {
        revision: 8,
        deviceId: "d1",
        values: [{ itemId: "a", value: 9, manualOverride: false }],
      },
    });
    expect(calls).toBe(1);
    expect(store.getHead()).toBe(head);
  });
});
