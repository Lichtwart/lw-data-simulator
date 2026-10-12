import { describe, expect, it } from "vitest";

import type { ActivityEntry } from "@/lib/api";

import { appendEntry, filterEntries, isFailed, mergeEntries } from "./log";

const entry = (id: number, over: Partial<ActivityEntry> = {}): ActivityEntry => ({
  id,
  time: "2026-10-12T12:00:00Z",
  method: "GET",
  baseUrl: "https://x",
  user: "u",
  path: "/inventory/managedObjects",
  status: 200,
  durationMs: 10,
  requestBody: null,
  responseBody: null,
  error: null,
  ...over,
});

describe("activity log", () => {
  it("appends, ignores duplicates and keeps the newest entries", () => {
    let list: ActivityEntry[] = [];
    for (let i = 1; i <= 5; i++) list = appendEntry(list, entry(i), 3);
    expect(list.map((e) => e.id)).toEqual([3, 4, 5]);
    expect(appendEntry(list, entry(5), 3)).toBe(list);
  });

  it("detects failed requests", () => {
    expect(isFailed(entry(1))).toBe(false);
    expect(isFailed(entry(1, { status: 401 }))).toBe(true);
    expect(isFailed(entry(1, { status: null }))).toBe(true);
    expect(isFailed(entry(1, { error: "timeout" }))).toBe(true);
  });

  it("filters by words over method, path and status", () => {
    const list = [
      entry(1),
      entry(2, { method: "POST", path: "/measurement/measurements", status: 201 }),
    ];
    expect(filterEntries(list, "post measurement")).toHaveLength(1);
    expect(filterEntries(list, "201")).toHaveLength(1);
    expect(filterEntries(list, "")).toHaveLength(2);
    expect(filterEntries(list, "alarm")).toHaveLength(0);
  });

  it("merges the loaded log with live entries by id, oldest first", () => {
    const merged = mergeEntries([entry(1), entry(2)], [entry(2), entry(3)]);
    expect(merged.map((e) => e.id)).toEqual([1, 2, 3]);
    expect(mergeEntries([entry(1), entry(2), entry(3)], [], 2).map((e) => e.id)).toEqual([2, 3]);
  });
});
