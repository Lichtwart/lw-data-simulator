import type { DemoDeviceSummary, GroupRef } from "@/lib/api";

export interface DeviceSection {
  /** `null` = devices without a parent group. */
  group: GroupRef | null;
  devices: DemoDeviceSummary[];
}

const byName = (a: { name: string }, b: { name: string }) =>
  a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });

/** Devices grouped by parent group (groups sorted by name, "no group" last, devices by name). */
export function groupDevices(devices: readonly DemoDeviceSummary[]): DeviceSection[] {
  const groups = new Map<string, DeviceSection>();
  let ungrouped: DeviceSection | null = null;
  for (const d of devices) {
    if (!d.parentGroup) {
      ungrouped ??= { group: null, devices: [] };
      ungrouped.devices.push(d);
      continue;
    }
    const section = groups.get(d.parentGroup.id) ?? { group: d.parentGroup, devices: [] };
    section.devices.push(d);
    groups.set(d.parentGroup.id, section);
  }
  const sections = [...groups.values()].sort((a, b) =>
    byName(a.group as GroupRef, b.group as GroupRef),
  );
  if (ungrouped) sections.push(ungrouped);
  for (const s of sections) s.devices.sort(byName);
  return sections;
}
