import { beforeAll, describe, expect, it } from "vitest";

import i18n from "@/i18n";
import type { OperationPrompt } from "@/lib/api";

import { operationSummary } from "./operationSummary";

const detail = (d: OperationPrompt["detail"]) => ({ detail: d });

describe("operationSummary (F28)", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("de");
  });

  it("words a group mode in German and English", () => {
    const p = detail({ kind: "groupMode", group: 3, mode: "auto" });
    expect(operationSummary(p, i18n.getFixedT("de"), "de")).toBe("Gruppe 3 → Auto");
    expect(operationSummary(p, i18n.getFixedT("en"), "en")).toBe("Group 3 → Auto");
  });

  it("words a lamp switch and a titled Modbus write", () => {
    expect(
      operationSummary(
        detail({ kind: "converterState", converter: 4, on: false }),
        i18n.getFixedT("de"),
        "de",
      ),
    ).toBe("Lampe 4 → Aus");
    const w = detail({
      kind: "modbusWrite",
      slaveAddress: 20,
      registerAddress: 40001,
      value: 7,
      coil: false,
      itemTitle: { de: "Sollwert", en: "Setpoint" },
    });
    expect(operationSummary(w, i18n.getFixedT("en"), "en")).toBe(
      "Setpoint: Modbus 20:40001 (register) ← 7",
    );
    expect(operationSummary(w, i18n.getFixedT("de"), "de")).toBe(
      "Sollwert: Modbus 20:40001 (Register) ← 7",
    );
  });
});
