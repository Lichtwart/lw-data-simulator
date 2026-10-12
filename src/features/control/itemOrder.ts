import type { LibraryItem_Serialize } from "@/lib/api";

const KIND_ORDER: Record<LibraryItem_Serialize["kind"], number> = {
  system: 0,
  dali: 1,
  daliBrightness: 2,
  daliMotion: 3,
  digitalInput: 4,
  impulseCounter: 5,
  modbus: 6,
  customAlarm: 7,
};

/** Display order: native kinds first (by index), then Modbus by slave and id. */
export function compareItems(a: LibraryItem_Serialize, b: LibraryItem_Serialize): number {
  const k = KIND_ORDER[a.kind] - KIND_ORDER[b.kind];
  if (k !== 0) return k;
  if (a.index !== undefined && b.index !== undefined && a.index !== b.index)
    return a.index - b.index;
  if (a.kind === "modbus" && b.kind === "modbus" && a.slave !== b.slave)
    return a.slave.localeCompare(b.slave);
  return a.id.localeCompare(b.id, undefined, { numeric: true });
}
