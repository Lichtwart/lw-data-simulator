/**
 * Pure logic of the scenario editor (Setup tab): drafts, validation (mirrors the backend's
 * `validate_device_scenarios` step rules), ids, and the `update_demo_setup` patch
 * (`customScenarios` / `hiddenScenarios`). Unit-tested in `scenarioDraft.test.ts`.
 */
import type {
  Control,
  FaultTarget,
  GeneratorConfig,
  LibraryItem_Serialize,
  LocalizedText,
  Scenario_Serialize,
  ScenarioStep,
} from "@/lib/api";
import { parseDecimal } from "@/lib/format";

import { faultKey } from "../control/faults";
import {
  draftFromGenerator,
  draftToGenerator,
  type GeneratorDraft,
} from "../control/generatorDraft";

/** One step while it is edited (numbers stay text until validated). */
export type StepDraft =
  | { kind: "value"; itemId: string; value: string }
  | { kind: "generator"; itemId: string; generator: GeneratorDraft }
  | { kind: "fault"; target: FaultTarget | null; active: boolean };

/** Form state of one scenario. */
export interface ScenarioDraft {
  /** Empty for a new scenario: derived from the name on submit ({@link scenarioIdFor}). */
  id: string;
  nameDe: string;
  nameEn: string;
  descriptionDe: string;
  descriptionEn: string;
  start: boolean;
  steps: StepDraft[];
}

export const EMPTY_SCENARIO: ScenarioDraft = {
  id: "",
  nameDe: "",
  nameEn: "",
  descriptionDe: "",
  descriptionEn: "",
  start: false,
  steps: [],
};

/** Why a step value does not fit the item's control. */
export type ValueProblem = "number" | "range" | "switch" | "option" | "counter";

export type StepProblem = "item" | "value" | "generator" | "fault";

export type ScenarioProblem =
  | { kind: "name" }
  | { kind: "description" }
  | { kind: "noSteps" }
  | { kind: "step"; index: number; problem: StepProblem };

/** The item's control, if it has one. */
export function controlOf(item: LibraryItem_Serialize): Control | null {
  return "control" in item ? (item.control ?? null) : null;
}

/** Items a value / generator step can address (custom alarms have no value: use a fault step). */
export function hasValue(item: LibraryItem_Serialize): boolean {
  return item.kind !== "customAlarm";
}

/** Start / fallback value of an item (control default, else 100 for a lamp, else 0). */
export function defaultValue(item: LibraryItem_Serialize): number {
  const c = controlOf(item);
  if (c) return c.default;
  return item.kind === "dali" ? 100 : 0;
}

/** Mirror of the backend's `value_problem`: does `value` fit the item's control? */
export function valueProblem(item: LibraryItem_Serialize, value: number): ValueProblem | null {
  if (!Number.isFinite(value)) return "number";
  const c = controlOf(item);
  if (!c) return null;
  switch (c.kind) {
    case "slider":
      return value < c.min || value > c.max ? "range" : null;
    case "switch":
      return value === 0 || value === 1 ? null : "switch";
    case "select":
      return c.options.some((o) => o.value === value) ? null : "option";
    case "counter":
      return value < c.default ? "counter" : null;
  }
}

/**
 * Mirror of the backend's `generator_fit_problem` + `generator_band_problem`: `counter` only for
 * counters, continuous generators not for on / off style values, and their band inside a slider.
 */
export function generatorFits(item: LibraryItem_Serialize, g: GeneratorConfig): boolean {
  const c = controlOf(item);
  const counterItem = item.kind === "impulseCounter" || c?.kind === "counter";
  if (g.kind === "counter") return counterItem;
  const discrete =
    item.kind === "digitalInput" ||
    item.kind === "daliMotion" ||
    item.kind === "impulseCounter" ||
    c?.kind === "switch" ||
    c?.kind === "select" ||
    c?.kind === "counter";
  if (discrete) return false;
  if (c?.kind !== "slider") return true;
  let lo: number;
  let hi: number;
  if (g.kind === "randomWalk") {
    lo = g.center - Math.abs(g.spread);
    hi = g.center + Math.abs(g.spread);
  } else if (g.kind === "sine") {
    const center = g.center ?? c.default;
    lo = center - Math.abs(g.amplitude);
    hi = center + Math.abs(g.amplitude);
  } else {
    lo = Math.min(...g.points);
    hi = Math.max(...g.points);
  }
  return lo >= c.min && hi <= c.max;
}

/** Whether a generator can run on the item at all (some kind fits). */
export function canTakeGenerator(item: LibraryItem_Serialize): boolean {
  if (!hasValue(item)) return false;
  const c = controlOf(item);
  if (item.kind === "impulseCounter" || c?.kind === "counter") return true;
  return !(
    item.kind === "digitalInput" ||
    item.kind === "daliMotion" ||
    c?.kind === "switch" ||
    c?.kind === "select"
  );
}

/** A new step of `kind` for `item` (seeded with the item's default / generator). */
export function newStep(
  kind: StepDraft["kind"],
  item: LibraryItem_Serialize | undefined,
): StepDraft {
  if (kind === "fault") return { kind, target: null, active: true };
  if (kind === "value") {
    return { kind, itemId: item?.id ?? "", value: item ? String(defaultValue(item)) : "" };
  }
  const own = item && "generator" in item ? (item.generator ?? null) : null;
  const generator = draftFromGenerator(own, item ? defaultValue(item) : 0);
  if (generator.kind === "none") {
    const c = item ? controlOf(item) : null;
    generator.kind =
      item?.kind === "impulseCounter" || c?.kind === "counter" ? "counter" : "randomWalk";
  }
  return { kind, itemId: item?.id ?? "", generator };
}

function stepToDraft(step: ScenarioStep): StepDraft {
  switch (step.kind) {
    case "value":
      return { kind: "value", itemId: step.itemId, value: String(step.value) };
    case "generator":
      return {
        kind: "generator",
        itemId: step.itemId,
        generator: draftFromGenerator(step.generator),
      };
    case "fault":
      return { kind: "fault", target: step.target, active: step.active };
  }
}

export function scenarioToDraft(s: Scenario_Serialize): ScenarioDraft {
  return {
    id: s.id,
    nameDe: s.name.de,
    nameEn: s.name.en,
    descriptionDe: s.description?.de ?? "",
    descriptionEn: s.description?.en ?? "",
    start: s.start,
    steps: s.set.map(stepToDraft),
  };
}

/** First problem of a step, or `null`. */
export function stepProblem(
  step: StepDraft,
  items: ReadonlyMap<string, LibraryItem_Serialize>,
): StepProblem | null {
  if (step.kind === "fault") return step.target ? null : "fault";
  const item = items.get(step.itemId);
  if (!item || !hasValue(item)) return "item";
  if (step.kind === "value") {
    const v = parseDecimal(step.value);
    return v === null || valueProblem(item, v) ? "value" : null;
  }
  const g = draftToGenerator(step.generator);
  return g && generatorFits(item, g) ? null : "generator";
}

/** First problem of a draft, or `null` when it can be saved. */
export function validateScenarioDraft(
  d: ScenarioDraft,
  items: ReadonlyMap<string, LibraryItem_Serialize>,
): ScenarioProblem | null {
  if (!d.nameDe.trim() || !d.nameEn.trim()) return { kind: "name" };
  if (!d.descriptionDe.trim() !== !d.descriptionEn.trim()) return { kind: "description" };
  if (d.steps.length === 0) return { kind: "noSteps" };
  for (const [index, step] of d.steps.entries()) {
    const problem = stepProblem(step, items);
    if (problem) return { kind: "step", index, problem };
  }
  return null;
}

/** Draft → scenario. Call only after {@link validateScenarioDraft} returned `null`. */
export function draftToScenario(d: ScenarioDraft, id: string): Scenario_Serialize {
  const set: ScenarioStep[] = d.steps.flatMap((s): ScenarioStep[] => {
    if (s.kind === "fault")
      return s.target ? [{ kind: "fault", target: s.target, active: s.active }] : [];
    if (s.kind === "value") {
      const value = parseDecimal(s.value);
      return value === null ? [] : [{ kind: "value", itemId: s.itemId, value }];
    }
    const generator = draftToGenerator(s.generator);
    return generator ? [{ kind: "generator", itemId: s.itemId, generator }] : [];
  });
  const description: LocalizedText | null =
    d.descriptionDe.trim() && d.descriptionEn.trim()
      ? { de: d.descriptionDe.trim(), en: d.descriptionEn.trim() }
      : null;
  return {
    id,
    name: { de: d.nameDe.trim(), en: d.nameEn.trim() },
    ...(description ? { description } : {}),
    start: d.start,
    set,
  };
}

/** A free scenario id (`[a-z0-9-]`) derived from `name`: `Kühlung aus` → `kuehlung-aus`. */
export function scenarioIdFor(name: string, taken: ReadonlySet<string>): string {
  const slug =
    name
      .toLowerCase()
      .replace(/ä/g, "ae")
      .replace(/ö/g, "oe")
      .replace(/ü/g, "ue")
      .replace(/ß/g, "ss")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40)
      .replace(/-+$/, "") || "scenario";
  if (!taken.has(slug)) return slug;
  for (let n = 2; ; n++) {
    const id = `${slug}-${n}`;
    if (!taken.has(id)) return id;
  }
}

/** Insert or replace `s` (by id) in `list`; with `s.start` every other scenario loses `start`. */
export function upsertScenario(
  list: readonly Scenario_Serialize[],
  s: Scenario_Serialize,
): Scenario_Serialize[] {
  const others = s.start ? list.map((x) => (x.start ? { ...x, start: false } : x)) : [...list];
  const at = others.findIndex((x) => x.id === s.id);
  if (at < 0) return [...others, s];
  others[at] = s;
  return others;
}

/** A copy of `s` under a new id and with `suffix` appended to its names; never the start one. */
export function duplicateScenario(
  s: Scenario_Serialize,
  taken: ReadonlySet<string>,
  suffix: LocalizedText,
): Scenario_Serialize {
  const name = { de: `${s.name.de} ${suffix.de}`, en: `${s.name.en} ${suffix.en}` };
  return { ...s, id: scenarioIdFor(name.en, taken), name, start: false };
}

/** Same JSON with object keys sorted (field order and absent vs. `null` do not matter). */
function canonical(v: unknown): string {
  return JSON.stringify(v, (_k, x: unknown) =>
    x && typeof x === "object" && !Array.isArray(x)
      ? Object.fromEntries(
          Object.entries(x)
            .filter(([, y]) => y !== null && y !== undefined)
            .sort(([a], [b]) => a.localeCompare(b)),
        )
      : x,
  );
}

/** Whether two scenarios say the same (ignoring field order and empty optional fields). */
export function sameScenario(a: Scenario_Serialize, b: Scenario_Serialize): boolean {
  return canonical(a) === canonical(b);
}

/** How a scenario of the edited list relates to the library. */
export function scenarioOrigin(
  base: readonly Scenario_Serialize[],
  s: Scenario_Serialize,
): "library" | "edited" | "custom" {
  const lib = base.find((b) => b.id === s.id);
  if (!lib) return "custom";
  return sameScenario(lib, s) ? "library" : "edited";
}

/**
 * The `update_demo_setup` fields for the edited list: every scenario that is not the unchanged
 * library one is custom; every library scenario missing from the list is hidden.
 */
export function scenarioPatch(
  base: readonly Scenario_Serialize[],
  list: readonly Scenario_Serialize[],
): { customScenarios: Scenario_Serialize[]; hiddenScenarios: string[] } {
  return {
    customScenarios: list.filter((s) => scenarioOrigin(base, s) !== "library"),
    hiddenScenarios: base.filter((b) => !list.some((s) => s.id === b.id)).map((b) => b.id),
  };
}

/** Key of a fault target for selects ({@link faultKey}); `""` for none. */
export function targetKey(target: FaultTarget | null): string {
  return target ? faultKey(target) : "";
}
