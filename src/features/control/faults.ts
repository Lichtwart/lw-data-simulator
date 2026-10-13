import type {
  ActiveFault,
  FaultDefinition,
  FaultTarget,
  LibraryItem_Serialize,
  OpenAlarmStatus,
  SlaveDefinition,
} from "@/lib/api";

import { type FilterableEntry, faultHaystack, itemHaystack, type Lang } from "./itemFilter";

/** Stable identity of a fault target (for matching catalog entries to active faults). */
export function faultKey(target: FaultTarget): string {
  switch (target.kind) {
    case "daliLamp":
      return `daliLamp:${target.lamp}:${target.alarmType}`;
    case "daliBus":
      return `daliBus:${target.alarmType}`;
    case "slaveTimeout":
      return `slaveTimeout:${target.slaveKey}`;
    case "slaveAlarm":
      return `slaveAlarm:${target.slaveKey}:${target.alarmName}`;
    case "customAlarm":
      return `customAlarm:${target.itemId}`;
    case "sensor":
      return `sensor:${target.sensorKind}:${target.index}:${target.alarm}`;
  }
}

/**
 * Status of the fault's alarm in Cumulocity while the fault is in effect (fired and not yet
 * cleared there), else `null` (DECISIONS D9).
 */
export function faultStatus(
  faults: readonly ActiveFault[],
  target: FaultTarget,
): OpenAlarmStatus | null {
  const key = faultKey(target);
  return faults.find((f) => faultKey(f.target) === key)?.status ?? null;
}

/** Badge of an open alarm status: i18n key + variant (acknowledged is calmer than active). */
export function alarmStatusBadge(status: OpenAlarmStatus): {
  key: "control.alarmStatus.active" | "control.alarmStatus.acknowledged";
  variant: "danger" | "warning";
} {
  return status === "ACKNOWLEDGED"
    ? { key: "control.alarmStatus.acknowledged", variant: "warning" }
    : { key: "control.alarmStatus.active", variant: "danger" };
}

/**
 * `alarmType=STATUS` of the faults among `defs` that are in effect, joined with `,` (a string
 * keeps `ItemCard`'s memo cheap). Read back with {@link decodeStatuses}.
 */
export function encodeStatuses(
  defs: readonly FaultDefinition[],
  faults: readonly ActiveFault[],
): string {
  return defs
    .flatMap((d) => {
      const status = faultStatus(faults, d.target);
      return status ? [`${d.alarmType}=${status}`] : [];
    })
    .join(",");
}

/** Alarm type → status from {@link encodeStatuses}. */
export function decodeStatuses(encoded: string): Map<string, OpenAlarmStatus> {
  const out = new Map<string, OpenAlarmStatus>();
  for (const part of encoded.split(",")) {
    const [type, status] = part.split("=");
    if (type && (status === "ACTIVE" || status === "ACKNOWLEDGED")) out.set(type, status);
  }
  return out;
}

/** Catalog entries of one DALI lamp. */
export function lampFaults(catalog: readonly FaultDefinition[], lamp: number): FaultDefinition[] {
  return catalog.filter((f) => f.target.kind === "daliLamp" && f.target.lamp === lamp);
}

/** Key of the item a per-item fault belongs to (`dali:3`, `brightness:2`, `motion:1`), else `null`. */
export function itemFaultOwner(target: FaultTarget): string | null {
  if (target.kind === "daliLamp") return `dali:${target.lamp}`;
  if (target.kind === "sensor") return `${target.sensorKind}:${target.index}`;
  return null;
}

/** Same key for a library item (`null` for items without per-item faults). */
export function itemFaultKeyOf(item: LibraryItem_Serialize): string | null {
  if (item.kind === "dali") return `dali:${item.index}`;
  if (item.kind === "daliBrightness") return `brightness:${item.index}`;
  if (item.kind === "daliMotion") return `motion:${item.index}`;
  return null;
}

/** Predicate "this catalog entry's alarm is in effect" for the filter helpers. */
export function activeFaultPredicate(
  faults: readonly ActiveFault[],
): (fault: FaultDefinition) => boolean {
  const keys = new Set(faults.map((f) => faultKey(f.target)));
  return (fault) => keys.has(faultKey(fault.target));
}

/** A card of the control list (see {@link FilterableEntry}). */
export type ControlEntry<R extends { meta: LibraryItem_Serialize }> = FilterableEntry & {
  key: string;
} & (
    | { type: "item"; row: R }
    | {
        type: "alarms";
        title: string;
        /** Technical second line (item id / slave key), not translated. */
        subtitle: string | null;
        /** `slaveTimeout` faults are labelled in the UI (their text is the firmware's). */
        group: "daliBus" | "slave" | "customAlarm";
      }
  );

export interface EntryLabels {
  lang: Lang;
  daliBus: string;
  slave: (name: string) => string;
  slaveTimeout: string;
}

const DALI_KINDS: ReadonlySet<LibraryItem_Serialize["kind"]> = new Set([
  "dali",
  "daliBrightness",
  "daliMotion",
]);

/**
 * The control list in display order: every item with its own alarms (lamp / sensor alarms,
 * a custom alarm item becomes an alarm card), the DALI bus alarms before the first DALI item,
 * each slave's timeout + alarms before its first register. An alarm card takes the tags of the
 * items it belongs to, so filtering by category / zone / client shows it with them.
 */
export function buildEntries<R extends { meta: LibraryItem_Serialize }>(
  rows: readonly R[],
  catalog: readonly FaultDefinition[],
  slaves: readonly SlaveDefinition[],
  labels: EntryLabels,
): ControlEntry<R>[] {
  const slaveNames = new Map(slaves.map((s) => [s.key, s.name]));
  const byOwner = new Map<string, FaultDefinition[]>();
  const custom = new Map<string, FaultDefinition[]>();
  const slaveFaults = new Map<string, FaultDefinition[]>();
  const bus: FaultDefinition[] = [];
  const push = <K>(m: Map<K, FaultDefinition[]>, k: K, f: FaultDefinition) =>
    m.set(k, [...(m.get(k) ?? []), f]);
  for (const f of catalog) {
    const t = f.target;
    const owner = itemFaultOwner(t);
    if (owner) push(byOwner, owner, f);
    else if (t.kind === "daliBus") bus.push(f);
    else if (t.kind === "customAlarm") push(custom, t.itemId, f);
    else if (t.kind === "slaveTimeout" || t.kind === "slaveAlarm") push(slaveFaults, t.slaveKey, f);
  }

  const union = (pred: (m: LibraryItem_Serialize) => boolean) => [
    ...new Set(rows.flatMap((r) => (pred(r.meta) ? r.meta.tags : []))),
  ];
  const label = (f: FaultDefinition) =>
    f.target.kind === "slaveTimeout" ? labels.slaveTimeout : undefined;
  const faultFields = (faults: readonly FaultDefinition[]) => ({
    faults,
    faultHaystacks: faults.map((f) => faultHaystack(f, label(f))),
  });

  const busEntry = (): ControlEntry<R> => ({
    type: "alarms",
    key: "daliBus",
    group: "daliBus",
    title: labels.daliBus,
    subtitle: null,
    tags: union((m) => DALI_KINDS.has(m.kind)),
    haystack: `${labels.daliBus}\ndali`.toLowerCase(),
    ...faultFields(bus),
  });
  const slaveEntry = (key: string, faults: readonly FaultDefinition[]): ControlEntry<R> => {
    const name = slaveNames.get(key) ?? key;
    const title = labels.slave(name);
    return {
      type: "alarms",
      key: `slave:${key}`,
      group: "slave",
      title,
      subtitle: key,
      tags: union((m) => m.kind === "modbus" && m.slave === key),
      haystack: [title, name, key].join("\n").toLowerCase(),
      ...faultFields(faults),
    };
  };

  const out: ControlEntry<R>[] = [];
  let busPlaced = bus.length === 0;
  const slavesPlaced = new Set<string>();
  for (const row of rows) {
    const m = row.meta;
    if (!busPlaced && DALI_KINDS.has(m.kind)) {
      out.push(busEntry());
      busPlaced = true;
    }
    if (m.kind === "modbus" && !slavesPlaced.has(m.slave)) {
      slavesPlaced.add(m.slave);
      const faults = slaveFaults.get(m.slave);
      if (faults) out.push(slaveEntry(m.slave, faults));
    }
    if (m.kind === "customAlarm") {
      out.push({
        type: "alarms",
        key: `item:${m.id}`,
        group: "customAlarm",
        title: m.title[labels.lang],
        subtitle: m.id,
        tags: m.tags,
        haystack: itemHaystack(m),
        ...faultFields(custom.get(m.id) ?? []),
      });
      continue;
    }
    const owner = itemFaultKeyOf(m);
    const slaveName = m.kind === "modbus" ? slaveNames.get(m.slave) : null;
    out.push({
      type: "item",
      key: `item:${m.id}`,
      row,
      tags: m.tags,
      haystack: itemHaystack(m, slaveName),
      ...faultFields((owner !== null ? byOwner.get(owner) : undefined) ?? []),
    });
  }
  // Alarms without an item on the list (no DALI item / no register of that slave selected).
  if (!busPlaced) out.push(busEntry());
  for (const [key, faults] of slaveFaults)
    if (!slavesPlaced.has(key)) out.push(slaveEntry(key, faults));
  return out;
}

/**
 * The "Clear all" flow, free of React: the dialog closes whatever happens, the toast says the
 * clear was queued ("being cleared": a running session ends the effects at once and clears in
 * Cumulocity in the background, which can still fail and is then reported as a session error)
 * or what went wrong.
 */
export async function runClearAll(
  clear: () => Promise<unknown>,
  ui: { closeDialog: () => void; success: () => void; failure: (error: unknown) => void },
): Promise<void> {
  try {
    await clear();
    ui.success();
  } catch (e) {
    ui.failure(e);
  } finally {
    ui.closeDialog();
  }
}
