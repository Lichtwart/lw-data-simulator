import { describe, expect, it, vi } from "vitest";

import type { ActiveFault, FaultDefinition } from "@/lib/api";

import {
  alarmStatusBadge,
  decodeStatuses,
  encodeStatuses,
  faultKey,
  faultStatus,
  groupFaults,
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

  it("selects lamp faults and groups the rest", () => {
    expect(lampFaults(catalog, 3)).toHaveLength(1);
    expect(lampFaults(catalog, 9)).toHaveLength(0);
    const g = groupFaults(catalog);
    expect(g.daliBus).toHaveLength(1);
    expect(g.customAlarms).toHaveLength(1);
    expect(g.slaves.get("kuehlung")).toHaveLength(2);
  });

  it("assigns per-item faults to their item", () => {
    expect(itemFaultOwner(catalog[0]?.target as ActiveFault["target"])).toBe("dali:3");
    expect(itemFaultOwner({ kind: "sensor", sensorKind: "motion", index: 2, alarm: "lost" })).toBe(
      "motion:2",
    );
    expect(itemFaultOwner({ kind: "daliBus", alarmType: "x" })).toBeNull();
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
