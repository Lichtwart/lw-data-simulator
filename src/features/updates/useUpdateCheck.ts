import { check, type Update } from "@tauri-apps/plugin-updater";
import { useEffect, useSyncExternalStore } from "react";

import { isTauri } from "@/lib/api";

/** Outcome of a check: a newer version exists, none exists, or the check itself failed. */
export type UpdateResult = "found" | "none" | "error";

interface UpdateState {
  update: Update | null;
  checking: boolean;
  /** The last check failed (offline, bad updater key, 404): "no update" is NOT known. */
  error: boolean;
  /** At least one check finished (successfully or not). */
  checked: boolean;
}

// One shared state for the whole app (sidebar badge + Settings), not one per hook instance.
let state: UpdateState = { update: null, checking: false, error: false, checked: false };
let autoChecked = false;
const listeners = new Set<() => void>();

function set(next: Partial<UpdateState>) {
  state = { ...state, ...next };
  for (const l of listeners) l();
}

async function runCheck(): Promise<UpdateResult> {
  if (!isTauri()) return "none";
  if (state.checking) return state.error ? "error" : state.update ? "found" : "none";
  set({ checking: true });
  try {
    const u = await check();
    // Free the native resource of the replaced `Update`.
    const old = state.update;
    if (old && old !== u) void old.close().catch(() => {});
    set({ update: u, error: false, checking: false, checked: true });
    return u ? "found" : "none";
  } catch {
    set({ error: true, checking: false, checked: true });
    return "error";
  }
}

/**
 * Checks GitHub Releases for a newer version once on start (PRD §15) and exposes it
 * for the "New version" badge. `recheck` is the manual check from Settings. The state is
 * shared by all callers. Requires the real updater pubkey in `src-tauri/tauri.conf.json`;
 * until then checks fail and are reported as `error`, not as "up to date".
 */
export function useUpdateCheck(): UpdateState & { recheck: () => Promise<UpdateResult> } {
  const snapshot = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
  useEffect(() => {
    if (import.meta.env.DEV || autoChecked) return;
    autoChecked = true;
    void runCheck();
  }, []);
  return { ...snapshot, recheck: runCheck };
}
