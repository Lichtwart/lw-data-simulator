import { useCallback, useEffect, useRef, useState } from "react";

import { isTauri } from "./api";

export interface AsyncState<T> {
  data: T | undefined;
  error: unknown;
  loading: boolean;
  reload: () => void;
  setData: (data: T) => void;
}

/**
 * Run an async loader on mount and whenever `deps` change; the latest call wins.
 * Outside the Tauri webview (plain browser, tests) nothing is loaded.
 */
export function useAsync<T>(loader: () => Promise<T>, deps: readonly unknown[]): AsyncState<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  // biome-ignore lint/correctness/useExhaustiveDependencies: `deps` + `tick` drive reloads on purpose
  useEffect(() => {
    if (!isTauri()) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    loaderRef.current().then(
      (v) => {
        if (cancelled) return;
        setData(v);
        setError(null);
        setLoading(false);
      },
      (e) => {
        if (cancelled) return;
        setError(e ?? new Error("unknown"));
        setLoading(false);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [...deps, tick]);

  const reload = useCallback(() => setTick((n) => n + 1), []);
  return { data, error, loading, reload, setData };
}
