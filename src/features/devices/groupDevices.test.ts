import { describe, expect, it } from "vitest";

import type { DemoDeviceSummary } from "@/lib/api";

import { groupDevices } from "./groupDevices";

const dev = (id: string, name: string, group?: [string, string]): DemoDeviceSummary => ({
  id,
  name,
  description: null,
  parentGroup: group ? { id: group[0], name: group[1] } : null,
  demoId: null,
  libraryVersion: null,
  lock: null,
  lastUpdated: null,
  upgradeAvailable: false,
});

describe("groupDevices", () => {
  it("groups by parent, sorts groups and devices by name, ungrouped last", () => {
    const sections = groupDevices([
      dev("1", "Tankstelle 10", ["g2", "Süd"]),
      dev("2", "Solo"),
      dev("3", "Tankstelle 2", ["g2", "Süd"]),
      dev("4", "Laden", ["g1", "Nord"]),
    ]);
    expect(sections.map((s) => s.group?.name ?? null)).toEqual(["Nord", "Süd", null]);
    expect(sections[1]?.devices.map((d) => d.name)).toEqual(["Tankstelle 2", "Tankstelle 10"]);
    expect(sections[2]?.devices.map((d) => d.id)).toEqual(["2"]);
  });

  it("returns nothing for an empty list", () => {
    expect(groupDevices([])).toEqual([]);
  });
});
