import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";

import { api, events, isTauri, type SessionSnapshot } from "@/lib/api";
import { useAppEvent } from "@/lib/useAppEvent";

import { useAuth } from "../auth/AuthProvider";
import {
  headOf,
  initialSessionState,
  type SessionAction,
  type SessionHead,
  type SessionState,
  sessionReducer,
} from "./sessionState";

/**
 * Client-side session store. Not React state, so that the per-tick `item-values-changed`
 * updates only re-render components that read items (`useDeviceSession`), not the shell.
 *
 * Ordering rule: `session-changed` and the other events are the source of truth. A snapshot
 * returned by a command is only taken over if no event arrived while the command was in
 * flight (`mark()` before the call, `adopt(snapshot, mark)` after).
 */
export interface SessionStore {
  getState: () => SessionState;
  getHead: () => SessionHead | null;
  subscribe: (listener: () => void) => () => void;
  /** Apply an event (always wins). */
  dispatch: (action: SessionAction) => void;
  /** Event counter, to detect events that arrived during a command. */
  mark: () => number;
  /** Take over a command result, unless events arrived since `mark`. */
  adopt: (session: SessionSnapshot | null, mark: number) => void;
}

export function createSessionStore(): SessionStore {
  let state = initialSessionState;
  let head: SessionHead | null = null;
  let seq = 0;
  const listeners = new Set<() => void>();

  function set(next: SessionState) {
    if (next === state) return;
    state = next;
    head = headOf(state.session, head);
    for (const l of listeners) l();
  }

  return {
    getState: () => state,
    getHead: () => head,
    subscribe: (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    dispatch: (action) => {
      seq += 1;
      set(sessionReducer(state, action));
    },
    mark: () => seq,
    adopt: (session, mark) => {
      if (mark !== seq) return;
      set(sessionReducer(state, { type: "snapshot", session }));
    },
  };
}

const StoreContext = createContext<SessionStore | null>(null);

/** Mirrors the backend session via `get_session` + events. Only mounted below the login. */
export function SessionProvider({ children }: { children: ReactNode }) {
  const { auth } = useAuth();
  const [store] = useState(createSessionStore);

  useEffect(() => {
    if (!isTauri()) return;
    if (!auth) {
      store.dispatch({ type: "snapshot", session: null });
      return;
    }
    let cancelled = false;
    const mark = store.mark();
    api.getSession().then(
      (session) => !cancelled && store.adopt(session, mark),
      () => !cancelled && store.adopt(null, mark),
    );
    return () => {
      cancelled = true;
    };
  }, [auth, store]);

  useAppEvent(events.sessionChanged, (session) => store.dispatch({ type: "snapshot", session }));
  useAppEvent(events.itemValuesChanged, (payload) => store.dispatch({ type: "values", payload }));
  useAppEvent(events.faultChanged, (payload) => store.dispatch({ type: "fault", payload }));
  useAppEvent(events.operationPrompt, (prompt) => store.dispatch({ type: "prompt", prompt }));
  useAppEvent(events.operationResolved, (payload) => store.dispatch({ type: "resolved", payload }));
  useAppEvent(events.realtimeStatus, (status) => store.dispatch({ type: "realtime", status }));

  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>;
}

function useStore(): SessionStore {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useSession outside SessionProvider");
  return store;
}

interface SessionContextValue {
  /** The running session without per-tick data (any device), `null` when none. */
  session: SessionHead | null;
  realtimeError: string | null;
  /** Event counter; pass the value taken before a command to `adopt`. */
  mark: () => number;
  /** Take over a snapshot returned by a command (ignored if events arrived meanwhile). */
  adopt: (session: SessionSnapshot | null, mark: number) => void;
}

/**
 * The slow part of the session: does not re-render on item value ticks. Use
 * `useDeviceSession` for items / faults.
 */
export function useSession(): SessionContextValue {
  const store = useStore();
  const session = useSyncExternalStore(store.subscribe, store.getHead);
  const realtimeError = useSyncExternalStore(store.subscribe, () => store.getState().realtimeError);
  return useMemo(
    () => ({ session, realtimeError, mark: store.mark, adopt: store.adopt }),
    [session, realtimeError, store],
  );
}

/** The full snapshot incl. items and faults (re-renders on every value tick), if it runs on `deviceId`. */
export function useDeviceSession(deviceId: string): SessionSnapshot | null {
  const store = useStore();
  return useSyncExternalStore(store.subscribe, () => {
    const s = store.getState().session;
    return s && s.deviceId === deviceId ? s : null;
  });
}

/** The slow part of the session, but only if it runs on `deviceId`. */
export function useDeviceSessionHead(deviceId: string): SessionHead | null {
  const { session } = useSession();
  return session && session.deviceId === deviceId ? session : null;
}
