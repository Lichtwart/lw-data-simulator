import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

import { isTauri } from "./api";

/** Anything with a tauri-specta style `listen` (e.g. `events.sessionChanged`). */
interface Listenable<T> {
  listen: (cb: EventCallback<T>) => Promise<UnlistenFn>;
}

/**
 * Subscribe to a backend event for the lifetime of the component.
 * `useAppEvent(events.itemValuesChanged, (e) => apply(e.payload))`
 */
export function useAppEvent<T>(event: Listenable<T>, handler: (payload: T) => void) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    event
      .listen((e) => handlerRef.current(e.payload))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [event]);
}
