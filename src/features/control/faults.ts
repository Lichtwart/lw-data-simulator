import type {
  ActiveFault,
  FaultDefinition,
  FaultTarget,
  LibraryItem_Serialize,
  OpenAlarmStatus,
} from "@/lib/api";

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

export interface FaultSections {
  daliBus: FaultDefinition[];
  /** Slave key → its timeout + custom alarm faults. */
  slaves: Map<string, FaultDefinition[]>;
  customAlarms: FaultDefinition[];
}

/** Catalog split into the non-lamp sections shown on the control tab. */
export function groupFaults(catalog: readonly FaultDefinition[]): FaultSections {
  const out: FaultSections = { daliBus: [], slaves: new Map(), customAlarms: [] };
  for (const f of catalog) {
    const t = f.target;
    if (t.kind === "daliBus") out.daliBus.push(f);
    else if (t.kind === "customAlarm") out.customAlarms.push(f);
    else if (t.kind === "slaveTimeout" || t.kind === "slaveAlarm") {
      out.slaves.set(t.slaveKey, [...(out.slaves.get(t.slaveKey) ?? []), f]);
    }
  }
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
