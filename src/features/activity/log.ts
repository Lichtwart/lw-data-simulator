import type { ActivityEntry } from "@/lib/api";

/** The backend keeps the last 500 requests (PRD §16); the view mirrors that. */
const ACTIVITY_LIMIT = 500;

/** Append a new entry (ignoring duplicates by id), keep the newest `limit`. Oldest first. */
export function appendEntry(
  list: readonly ActivityEntry[],
  entry: ActivityEntry,
  limit = ACTIVITY_LIMIT,
): ActivityEntry[] {
  if (list.some((e) => e.id === entry.id)) return list as ActivityEntry[];
  const next = [...list, entry];
  return next.length > limit ? next.slice(next.length - limit) : next;
}

/** Merge the initially loaded log with entries that arrived live meanwhile (by id, oldest first). */
export function mergeEntries(
  loaded: readonly ActivityEntry[],
  live: readonly ActivityEntry[],
  limit = ACTIVITY_LIMIT,
): ActivityEntry[] {
  const byId = new Map<number, ActivityEntry>();
  for (const e of loaded) byId.set(e.id, e);
  for (const e of live) byId.set(e.id, e);
  const all = [...byId.values()].sort((a, b) => a.id - b.id);
  return all.length > limit ? all.slice(all.length - limit) : all;
}

/** Does the request count as failed (transport error or HTTP ≥ 400)? */
export function isFailed(e: Pick<ActivityEntry, "status" | "error">): boolean {
  return e.error !== null || e.status === null || e.status >= 400;
}

/** Case-insensitive filter over method, path and status. */
export function filterEntries(list: readonly ActivityEntry[], query: string): ActivityEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return list as ActivityEntry[];
  return list.filter((e) => {
    const hay = `${e.method} ${e.path} ${e.status ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}
