import type { GeneratorConfig } from "@/lib/api";
import { parseDecimal } from "@/lib/format";

/** Generator kinds the editor offers; `none` = no generator (the value only changes manually). */
export const GENERATOR_KINDS = ["none", "randomWalk", "sine", "dailyProfile", "counter"] as const;
type GeneratorKind = (typeof GENERATOR_KINDS)[number];

/** Form state of the generator editor (numbers stay text until validated; PRD §8). */
export interface GeneratorDraft {
  kind: GeneratorKind;
  center: string;
  spread: string;
  /** Random walk step, optional. */
  step: string;
  /** Sine period in seconds. */
  period: string;
  amplitude: string;
  /** 24 hourly values, separated by spaces, commas or semicolons. */
  points: string;
  ratePerHour: string;
}

const EMPTY: GeneratorDraft = {
  kind: "none",
  center: "0",
  spread: "1",
  step: "",
  period: "600",
  amplitude: "1",
  points: Array.from({ length: 24 }, () => "0").join(", "),
  ratePerHour: "10",
};

/** Draft for the current generator; `fallbackCenter` seeds new generators (e.g. the control default). */
export function draftFromGenerator(g: GeneratorConfig | null, fallbackCenter = 0): GeneratorDraft {
  const base = { ...EMPTY, center: String(fallbackCenter) };
  if (!g) return base;
  switch (g.kind) {
    case "randomWalk":
      return {
        ...base,
        kind: "randomWalk",
        center: String(g.center),
        spread: String(g.spread),
        step: g.step === null ? "" : String(g.step),
      };
    case "sine":
      return {
        ...base,
        kind: "sine",
        period: String(g.period),
        amplitude: String(g.amplitude),
        center: g.center === null ? "" : String(g.center),
      };
    case "dailyProfile":
      return { ...base, kind: "dailyProfile", points: g.points.join(", ") };
    case "counter":
      return { ...base, kind: "counter", ratePerHour: String(g.ratePerHour) };
  }
}

/** The 24 hourly points of the text, or `null` when it is not exactly 24 numbers. */
export function parsePoints(text: string): number[] | null {
  const parts = text.split(/[\s,;]+/).filter(Boolean);
  if (parts.length !== 24) return null;
  const nums = parts.map(parseDecimal);
  return nums.every((n): n is number => n !== null) ? nums : null;
}

/**
 * Validate a draft with the backend's rules (`generator_problem`): `undefined` = invalid,
 * `null` = "no generator", otherwise the config.
 */
export function draftToGenerator(d: GeneratorDraft): GeneratorConfig | null | undefined {
  switch (d.kind) {
    case "none":
      return null;
    case "randomWalk": {
      const center = parseDecimal(d.center);
      const spread = parseDecimal(d.spread);
      const step = d.step.trim() === "" ? null : parseDecimal(d.step);
      if (center === null || spread === null || spread < 0) return undefined;
      if (d.step.trim() !== "" && (step === null || step <= 0)) return undefined;
      return { kind: "randomWalk", center, spread, step };
    }
    case "sine": {
      const period = parseDecimal(d.period);
      const amplitude = parseDecimal(d.amplitude);
      const center = d.center.trim() === "" ? null : parseDecimal(d.center);
      if (period === null || period <= 0 || amplitude === null) return undefined;
      if (d.center.trim() !== "" && center === null) return undefined;
      return { kind: "sine", period, amplitude, center };
    }
    case "dailyProfile": {
      const points = parsePoints(d.points);
      return points ? { kind: "dailyProfile", points } : undefined;
    }
    case "counter": {
      const ratePerHour = parseDecimal(d.ratePerHour);
      return ratePerHour === null || ratePerHour < 0 ? undefined : { kind: "counter", ratePerHour };
    }
  }
}

/**
 * The persisted `lw_demo.generators` map after saving `generator` for `itemId`. `null` ("no
 * generator") removes the override, so the item falls back to its library generator (if any)
 * at the next start instead of silently keeping the old override.
 */
export function savedGenerators(
  current: Readonly<Record<string, GeneratorConfig>>,
  itemId: string,
  generator: GeneratorConfig | null,
): Record<string, GeneratorConfig> {
  const { [itemId]: _removed, ...rest } = current;
  return generator === null ? rest : { ...rest, [itemId]: generator };
}
