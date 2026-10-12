import { faultKey } from "@/features/control/faults";
import type {
  FaultChanged,
  ItemValuesChanged,
  OperationPrompt,
  OperationResolved,
  RealtimeStatus,
  SessionSnapshot,
} from "@/lib/api";

/** Client-side mirror of the backend session, kept current by events (pure reducer). */
export interface SessionState {
  session: SessionSnapshot | null;
  /** Last realtime error text (the snapshot only has the boolean). */
  realtimeError: string | null;
}

export const initialSessionState: SessionState = { session: null, realtimeError: null };

export type SessionAction =
  | { type: "snapshot"; session: SessionSnapshot | null }
  | { type: "values"; payload: ItemValuesChanged }
  | { type: "fault"; payload: FaultChanged }
  | { type: "prompt"; prompt: OperationPrompt }
  | { type: "resolved"; payload: OperationResolved }
  | { type: "realtime"; status: RealtimeStatus };

/** Monotonic per-session counter of the snapshot (`null` when there is no session). */
function revisionOf(session: SessionSnapshot | null): number | null {
  return session ? session.revision : null;
}

/** The slow-changing part of a snapshot (everything but the per-tick items and faults). */
export type SessionHead = Omit<SessionSnapshot, "items" | "activeFaults">;

const HEAD_KEYS = [
  "deviceId",
  "deviceName",
  "status",
  "startedAt",
  "autoAccept",
  "realisticMode",
  "sendIntervalSec",
  "lastSentAt",
  "realtimeConnected",
  "restartNeeded",
  "groups",
  "pendingOperations",
] as const satisfies readonly (keyof SessionHead)[];

/** `head` for `session`, reusing `prev` when no head field changed (stable reference). */
export function headOf(
  session: SessionSnapshot | null,
  prev: SessionHead | null,
): SessionHead | null {
  if (!session) return null;
  if (prev && HEAD_KEYS.every((k) => prev[k] === session[k])) return prev;
  const { items: _items, activeFaults: _faults, ...head } = session;
  return head;
}

/** An event at or below the snapshot's revision is already contained in the snapshot. */
function isStale(session: SessionSnapshot, revision: number): boolean {
  return revision <= session.revision;
}

export function sessionReducer(state: SessionState, action: SessionAction): SessionState {
  switch (action.type) {
    case "snapshot": {
      const have = revisionOf(state.session);
      const got = revisionOf(action.session);
      if (
        have !== null &&
        got !== null &&
        action.session?.deviceId === state.session?.deviceId &&
        got < have
      ) {
        return state;
      }
      return {
        session: action.session,
        realtimeError: action.session ? state.realtimeError : null,
      };
    }
    case "values": {
      const s = state.session;
      if (!s || s.deviceId !== action.payload.deviceId || isStale(s, action.payload.revision)) {
        return state;
      }
      const changes = new Map(action.payload.values.map((v) => [v.itemId, v]));
      let changed = false;
      const items = s.items.map((item) => {
        const v = changes.get(item.itemId);
        if (!v || (v.value === item.value && v.manualOverride === item.manualOverride)) return item;
        changed = true;
        return { ...item, value: v.value, manualOverride: v.manualOverride };
      });
      return changed
        ? { ...state, session: { ...s, items, revision: action.payload.revision } }
        : state;
    }
    case "fault": {
      const s = state.session;
      if (!s || s.deviceId !== action.payload.deviceId || isStale(s, action.payload.revision)) {
        return state;
      }
      const key = faultKey(action.payload.target);
      const rest = s.activeFaults.filter((f) => faultKey(f.target) !== key);
      const activeFaults = action.payload.fault ? [...rest, action.payload.fault] : rest;
      return { ...state, session: { ...s, activeFaults, revision: action.payload.revision } };
    }
    case "prompt": {
      const s = state.session;
      if (
        !s ||
        s.deviceId !== action.prompt.deviceId ||
        isStale(s, action.prompt.revision) ||
        s.pendingOperations.some((p) => p.operationId === action.prompt.operationId)
      ) {
        return state;
      }
      return {
        ...state,
        session: {
          ...s,
          pendingOperations: [...s.pendingOperations, action.prompt],
          revision: action.prompt.revision,
        },
      };
    }
    case "resolved": {
      const s = state.session;
      const { operationId, deviceId, revision } = action.payload;
      if (!s || s.deviceId !== deviceId || isStale(s, revision)) return state;
      return {
        ...state,
        session: {
          ...s,
          revision,
          pendingOperations: s.pendingOperations.filter((p) => p.operationId !== operationId),
        },
      };
    }
    case "realtime": {
      const s = state.session;
      const { deviceId, revision, connected, error } = action.status;
      // A late event of a session that is already gone must not touch the new one.
      if (!s || s.deviceId !== deviceId || isStale(s, revision)) return state;
      return {
        session: { ...s, realtimeConnected: connected, revision },
        realtimeError: connected ? null : error,
      };
    }
  }
}
