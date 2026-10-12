import type { FiredRecord } from "@/lib/api";

/** One-line description of a fired record (for the history list). */
export function describeFired(r: FiredRecord): string {
  const q = r.request;
  switch (q.kind) {
    case "measurement":
      return `${q.type} · ${q.fragment}.${q.series} = ${q.value}${q.unit ? ` ${q.unit}` : ""}`;
    case "alarm":
      return `${q.type} · ${q.severity}`;
    case "event":
      return q.type;
    case "json":
      return `${q.target} · JSON`;
  }
}

/** Insert or replace by id; keeps the newest `max` records, newest first. */
export function upsertHistory(
  list: readonly FiredRecord[],
  record: FiredRecord,
  max = 50,
): FiredRecord[] {
  const rest = list.filter((r) => r.id !== record.id);
  return [record, ...rest].sort((a, b) => b.id - a.id).slice(0, max);
}
