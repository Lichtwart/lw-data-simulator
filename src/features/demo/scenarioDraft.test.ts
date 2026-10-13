import { describe, expect, it } from "vitest";

import type { LibraryItem_Serialize, Scenario_Serialize } from "@/lib/api";

import {
  draftToScenario,
  duplicateScenario,
  EMPTY_SCENARIO,
  generatorFits,
  newStep,
  type ScenarioDraft,
  scenarioIdFor,
  scenarioOrigin,
  scenarioPatch,
  scenarioToDraft,
  upsertScenario,
  validateScenarioDraft,
  valueProblem,
} from "./scenarioDraft";

const title = { de: "x", en: "x" };
const freezer: LibraryItem_Serialize = {
  kind: "modbus",
  id: "freezer_temp",
  slave: "kuehlung",
  title,
  tags: [],
  registerType: "input",
  datatype: "float32",
  unit: "°C",
  writable: false,
  control: { kind: "slider", min: -30, max: 0, step: 0.5, default: -18 },
  generator: { kind: "randomWalk", center: -18, spread: 1, step: 0.2 },
  effects: [],
};
const fan: LibraryItem_Serialize = {
  kind: "modbus",
  id: "hood_fan",
  slave: "kuehlung",
  title,
  tags: [],
  registerType: "coil",
  datatype: null,
  unit: null,
  writable: true,
  control: { kind: "switch", default: 0 },
  generator: null,
  effects: [],
};
const meter: LibraryItem_Serialize = {
  kind: "impulseCounter",
  id: "gas-meter",
  index: 3,
  title,
  tags: [],
  unit: "m3",
  control: { kind: "counter", default: 1000 },
  generator: null,
  effects: [],
};
const door: LibraryItem_Serialize = {
  kind: "customAlarm",
  id: "doorOpen",
  title,
  tags: [],
  severity: "MAJOR",
  text: "Tür offen",
};
const items = new Map([freezer, fan, meter, door].map((i) => [i.id, i]));

const draft: ScenarioDraft = {
  ...EMPTY_SCENARIO,
  nameDe: "Kühlung aus",
  nameEn: "Cooling off",
  descriptionDe: "Tiefkühler wird warm.",
  descriptionEn: "Freezer warms up.",
  steps: [
    { kind: "value", itemId: "freezer_temp", value: "-5,5" },
    newStep("generator", freezer),
    { kind: "fault", target: { kind: "customAlarm", itemId: "doorOpen" }, active: true },
  ],
};

describe("scenario drafts", () => {
  it("accepts a valid draft and builds the scenario", () => {
    expect(validateScenarioDraft(draft, items)).toBeNull();
    const s = draftToScenario(draft, "cooling-off");
    expect(s).toEqual({
      id: "cooling-off",
      name: { de: "Kühlung aus", en: "Cooling off" },
      description: { de: "Tiefkühler wird warm.", en: "Freezer warms up." },
      start: false,
      set: [
        { kind: "value", itemId: "freezer_temp", value: -5.5 },
        {
          kind: "generator",
          itemId: "freezer_temp",
          generator: { kind: "randomWalk", center: -18, spread: 1, step: 0.2 },
        },
        { kind: "fault", target: { kind: "customAlarm", itemId: "doorOpen" }, active: true },
      ],
    });
    // Round trip.
    expect(draftToScenario(scenarioToDraft(s), s.id)).toEqual(s);
  });

  it("rejects names, half descriptions, empty scenarios and bad steps", () => {
    expect(validateScenarioDraft({ ...draft, nameEn: " " }, items)).toEqual({ kind: "name" });
    expect(validateScenarioDraft({ ...draft, descriptionEn: "" }, items)).toEqual({
      kind: "description",
    });
    expect(
      validateScenarioDraft({ ...draft, descriptionDe: "", descriptionEn: "" }, items),
    ).toBeNull();
    expect(validateScenarioDraft({ ...draft, steps: [] }, items)).toEqual({ kind: "noSteps" });
    const step = (s: ScenarioDraft["steps"][number]) =>
      validateScenarioDraft({ ...draft, steps: [s] }, items);
    expect(step({ kind: "value", itemId: "freezer_temp", value: "5" })).toMatchObject({
      problem: "value",
    });
    expect(step({ kind: "value", itemId: "nope", value: "1" })).toMatchObject({ problem: "item" });
    expect(step({ kind: "value", itemId: "doorOpen", value: "1" })).toMatchObject({
      problem: "item",
    });
    expect(step({ kind: "fault", target: null, active: true })).toMatchObject({
      problem: "fault",
    });
    expect(step({ ...newStep("generator", fan) })).toMatchObject({ problem: "generator" });
  });

  it("checks values against the control", () => {
    expect(valueProblem(freezer, -30)).toBeNull();
    expect(valueProblem(freezer, 0.5)).toBe("range");
    expect(valueProblem(fan, 0.5)).toBe("switch");
    expect(valueProblem(meter, 999)).toBe("counter");
    expect(valueProblem(meter, 1500)).toBeNull();
  });

  it("checks generators against the item", () => {
    expect(generatorFits(freezer, { kind: "randomWalk", center: -18, spread: 1, step: null })).toBe(
      true,
    );
    expect(generatorFits(freezer, { kind: "randomWalk", center: -1, spread: 2, step: null })).toBe(
      false,
    );
    expect(generatorFits(freezer, { kind: "counter", ratePerHour: 1 })).toBe(false);
    expect(generatorFits(meter, { kind: "counter", ratePerHour: 1 })).toBe(true);
    expect(generatorFits(fan, { kind: "sine", period: 60, amplitude: 1, center: null })).toBe(
      false,
    );
    expect(newStep("generator", meter)).toMatchObject({ generator: { kind: "counter" } });
  });

  it("derives free ids from names", () => {
    expect(scenarioIdFor("Kühlung aus!", new Set())).toBe("kuehlung-aus");
    expect(scenarioIdFor("Kühlung aus", new Set(["kuehlung-aus"]))).toBe("kuehlung-aus-2");
    expect(scenarioIdFor("???", new Set())).toBe("scenario");
  });
});

describe("scenario lists", () => {
  const sc = (id: string, start = false): Scenario_Serialize => ({
    id,
    name: { de: id, en: id },
    start,
    set: [{ kind: "value", itemId: "freezer_temp", value: -18 }],
  });
  const normal = sc("normal", true);
  const night = sc("night");
  const base = [normal, night, sc("fault")];

  it("keeps a single start scenario", () => {
    const list = upsertScenario(base, sc("new", true));
    expect(list.filter((s) => s.start).map((s) => s.id)).toEqual(["new"]);
    expect(list.map((s) => s.id)).toEqual(["normal", "night", "fault", "new"]);
  });

  it("duplicates under a new id, never as start", () => {
    const copy = duplicateScenario(normal, new Set(base.map((s) => s.id)), {
      de: "(Kopie)",
      en: "(copy)",
    });
    expect(copy).toMatchObject({ id: "normal-copy", start: false, name: { de: "normal (Kopie)" } });
  });

  it("saves edits and new scenarios as custom, deleted library ones as hidden", () => {
    const edited = { ...night, name: { de: "Nacht", en: "Night" } };
    // Field order and an empty description do not count as an edit.
    const same: Scenario_Serialize = {
      set: normal.set,
      start: true,
      name: normal.name,
      id: "normal",
      description: null,
    };
    const added = sc("new");
    const list = [same, edited, added];
    expect(scenarioOrigin(base, same)).toBe("library");
    expect(scenarioOrigin(base, edited)).toBe("edited");
    expect(scenarioOrigin(base, added)).toBe("custom");
    expect(scenarioPatch(base, list)).toEqual({
      customScenarios: [edited, added],
      hiddenScenarios: ["fault"],
    });
    expect(scenarioPatch(base, base)).toEqual({ customScenarios: [], hiddenScenarios: [] });
  });
});
