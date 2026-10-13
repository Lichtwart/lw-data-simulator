import { describe, expect, it, vi } from "vitest";

import type { ActiveFault, FaultDefinition, LibraryItem_Serialize } from "@/lib/api";

import {
  activeFaultPredicate,
  alarmStatusBadge,
  buildEntries,
  decodeStatuses,
  encodeStatuses,
  faultKey,
  faultStatus,
  itemFaultOwner,
  lampFaults,
  runClearAll,
} from "./faults";

const def = (target: FaultDefinition["target"], alarmType: string): FaultDefinition => ({
  target,
  alarmType,
  text: alarmType,
  severity: "MAJOR",
});

const catalog: FaultDefinition[] = [
  def({ kind: "daliLamp", lamp: 3, alarmType: "lw_lampFailure_1_03_openCircuit" }, "a"),
  def({ kind: "daliLamp", lamp: 4, alarmType: "lw_lampFailure_1_04_openCircuit" }, "b"),
  def({ kind: "daliBus", alarmType: "lw_daliBusPowerFailure" }, "c"),
  def({ kind: "slaveTimeout", slaveKey: "kuehlung" }, "d"),
  def({ kind: "slaveAlarm", slaveKey: "kuehlung", alarmName: "doorOpen" }, "e"),
  def({ kind: "customAlarm", itemId: "smoke" }, "f"),
  def({ kind: "sensor", sensorKind: "brightness", index: 2, alarm: "lost" }, "g"),
  def({ kind: "sensor", sensorKind: "motion", index: 2, alarm: "lost" }, "h"),
];

describe("faults", () => {
  it("gives distinct keys per target", () => {
    const keys = new Set(catalog.map((c) => faultKey(c.target)));
    expect(keys.size).toBe(catalog.length);
  });

  it("gives the alarm status of a fault in effect, else null", () => {
    const active: ActiveFault[] = [
      {
        target: catalog[3]?.target as ActiveFault["target"],
        alarmType: "d",
        alarmId: null,
        raisedAt: "",
        status: "ACKNOWLEDGED",
      },
    ];
    expect(faultStatus(active, { kind: "slaveTimeout", slaveKey: "kuehlung" })).toBe(
      "ACKNOWLEDGED",
    );
    expect(faultStatus(active, { kind: "slaveTimeout", slaveKey: "klima" })).toBeNull();
  });

  it("words the status badge", () => {
    expect(alarmStatusBadge("ACTIVE")).toEqual({
      key: "control.alarmStatus.active",
      variant: "danger",
    });
    expect(alarmStatusBadge("ACKNOWLEDGED")).toEqual({
      key: "control.alarmStatus.acknowledged",
      variant: "warning",
    });
  });

  it("encodes the statuses of a lamp's faults for the card", () => {
    const lamp3 = lampFaults(catalog, 3);
    const active: ActiveFault[] = [
      {
        target: lamp3[0]?.target as ActiveFault["target"],
        alarmType: "a",
        alarmId: "1",
        raisedAt: "",
        status: "ACTIVE",
      },
    ];
    const encoded = encodeStatuses(lamp3, active);
    expect(encoded).toBe("a=ACTIVE");
    expect(decodeStatuses(encoded)).toEqual(new Map([["a", "ACTIVE"]]));
    expect(encodeStatuses(lampFaults(catalog, 4), active)).toBe("");
    expect(decodeStatuses("").size).toBe(0);
  });

  it("selects lamp faults", () => {
    expect(lampFaults(catalog, 3)).toHaveLength(1);
    expect(lampFaults(catalog, 9)).toHaveLength(0);
  });

  it("tells which catalog entries are in effect", () => {
    const isActive = activeFaultPredicate([
      {
        target: { kind: "slaveTimeout", slaveKey: "kuehlung" },
        alarmType: "d",
        alarmId: null,
        raisedAt: "",
        status: "ACTIVE",
      },
    ]);
    expect(catalog.filter(isActive).map((f) => f.alarmType)).toEqual(["d"]);
  });

  it("assigns per-item faults to their item", () => {
    expect(itemFaultOwner(catalog[0]?.target as ActiveFault["target"])).toBe("dali:3");
    expect(itemFaultOwner({ kind: "sensor", sensorKind: "motion", index: 2, alarm: "lost" })).toBe(
      "motion:2",
    );
    expect(itemFaultOwner({ kind: "daliBus", alarmType: "x" })).toBeNull();
  });
});

describe("buildEntries", () => {
  const tags = (t: string) => ["retail", t];
  const items: LibraryItem_Serialize[] = [
    {
      kind: "system",
      id: "rssi",
      system: "rssi",
      title: { de: "Signal", en: "Signal" },
      tags: tags("category:system"),
    },
    {
      kind: "dali",
      id: "light-3",
      index: 3,
      title: { de: "Leuchte 3", en: "Lamp 3" },
      tags: tags("category:lighting"),
    },
    {
      kind: "dali",
      id: "light-4",
      index: 4,
      title: { de: "Leuchte 4", en: "Lamp 4" },
      tags: tags("zone:outside"),
    },
    {
      kind: "daliBrightness",
      id: "lux-2",
      index: 2,
      title: { de: "Helligkeit", en: "Brightness" },
      tags: tags("category:lighting"),
    },
    {
      kind: "modbus",
      id: "frz_temp",
      slave: "kuehlung",
      writable: false,
      title: { de: "Tiefkühler", en: "Freezer" },
      tags: tags("category:cooling"),
    },
    {
      kind: "modbus",
      id: "frz_power",
      slave: "kuehlung",
      writable: false,
      title: { de: "Leistung", en: "Power" },
      tags: tags("category:energy"),
    },
    {
      kind: "customAlarm",
      id: "smoke",
      severity: "CRITICAL",
      text: "Rauch",
      title: { de: "Rauchmelder", en: "Smoke detector" },
      tags: tags("category:safety"),
    },
  ];
  const rows = items.map((meta) => ({ meta }));
  const labels = {
    lang: "de" as const,
    daliBus: "DALI-Bus",
    slave: (name: string) => `Slave ${name}`,
    slaveTimeout: "Timeout (Slave antwortet nicht)",
  };
  const entries = buildEntries(
    rows,
    catalog,
    [{ key: "kuehlung", name: "Kuehlung", requestInterval: 10 }],
    labels,
  );

  it("puts every alarm into the list next to what it belongs to", () => {
    expect(entries.map((e) => e.key)).toEqual([
      "item:rssi",
      "daliBus",
      "item:light-3",
      "item:light-4",
      "item:lux-2",
      "slave:kuehlung",
      "item:frz_temp",
      "item:frz_power",
      "item:smoke",
    ]);
    const types = (key: string) =>
      entries.find((e) => e.key === key)?.faults.map((f) => f.alarmType);
    expect(types("item:light-3")).toEqual(["a"]);
    expect(types("item:lux-2")).toEqual(["g"]);
    expect(types("daliBus")).toEqual(["c"]);
    expect(types("slave:kuehlung")).toEqual(["d", "e"]);
    expect(types("item:smoke")).toEqual(["f"]);
    expect(types("item:rssi")).toEqual([]);
  });

  it("gives alarm cards the tags of their items and a searchable text", () => {
    const slave = entries.find((e) => e.key === "slave:kuehlung");
    expect(slave?.type).toBe("alarms");
    expect([...(slave?.tags ?? [])].sort()).toEqual([
      "category:cooling",
      "category:energy",
      "retail",
    ]);
    expect(slave?.haystack).toContain("kuehlung");
    // The timeout is searchable by its UI label.
    expect(slave?.faultHaystacks[0]).toContain("antwortet nicht");
    const bus = entries.find((e) => e.key === "daliBus");
    expect([...(bus?.tags ?? [])].sort()).toEqual(["category:lighting", "retail", "zone:outside"]);
    const smoke = entries.find((e) => e.key === "item:smoke");
    expect(smoke).toMatchObject({ type: "alarms", title: "Rauchmelder", subtitle: "smoke" });
    expect(smoke?.tags).toContain("category:safety");
  });

  it("keeps alarms whose items are not on the list", () => {
    const keys = buildEntries([], catalog, [], labels).map((e) => e.key);
    expect(keys).toEqual(["daliBus", "slave:kuehlung"]);
  });
});

describe("runClearAll", () => {
  it("closes the dialog and reports the queued clear", async () => {
    const ui = { closeDialog: vi.fn(), success: vi.fn(), failure: vi.fn() };
    await runClearAll(() => Promise.resolve(), ui);
    expect(ui.success).toHaveBeenCalledOnce();
    expect(ui.failure).not.toHaveBeenCalled();
    expect(ui.closeDialog).toHaveBeenCalledOnce();
  });

  it("reports the error, not a success, and still closes the dialog", async () => {
    const ui = { closeDialog: vi.fn(), success: vi.fn(), failure: vi.fn() };
    const error = { code: "http" };
    await runClearAll(() => Promise.reject(error), ui);
    expect(ui.success).not.toHaveBeenCalled();
    expect(ui.failure).toHaveBeenCalledWith(error);
    expect(ui.closeDialog).toHaveBeenCalledOnce();
  });
});
