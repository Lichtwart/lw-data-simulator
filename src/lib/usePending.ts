import { useCallback, useRef, useState } from "react";

/**
 * Per-control "call in flight" tracking: `run(key, fn)` ignores a second call for the same key
 * until the first settled (no duplicate commands on double click); `isPending(key)` disables the control.
 */
export function usePending() {
  const active = useRef(new Set<string>());
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());

  const run = useCallback(async (key: string, fn: () => Promise<unknown>) => {
    if (active.current.has(key)) return;
    active.current.add(key);
    setPending(new Set(active.current));
    try {
      await fn();
    } finally {
      active.current.delete(key);
      setPending(new Set(active.current));
    }
  }, []);

  const isPending = useCallback((key: string) => pending.has(key), [pending]);
  return { run, isPending };
}
