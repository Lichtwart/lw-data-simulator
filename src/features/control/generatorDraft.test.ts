import { describe, expect, it } from "vitest";

import type { GeneratorConfig } from "@/lib/api";
import {
  draftFromGenerator,
  draftToGenerator,
  parsePoints,
  savedGenerators,
} from "./generatorDraft";

describe("generator drafts", () => {
  it("round-trips every generator kind", () => {
    const gens = [
      { kind: "randomWalk", center: -18, spread: 1.5, step: null },
      { kind: "randomWalk", center: 20, spread: 2, step: 0.5 },
      { kind: "sine", period: 600, amplitude: 3, center: null },
      { kind: "sine", period: 60, amplitude: 1, center: 5 },
      { kind: "dailyProfile", points: Array.from({ length: 24 }, (_, i) => i) },
      { kind: "counter", ratePerHour: 12 },
    ] as const;
    for (const g of gens) expect(draftToGenerator(draftFromGenerator(g))).toEqual(g);
  });

  it("maps no generator to null and accepts decimal commas", () => {
    expect(draftToGenerator(draftFromGenerator(null))).toBeNull();
    const d = { ...draftFromGenerator(null), kind: "randomWalk" as const, center: "-18,5" };
    expect(draftToGenerator(d)).toMatchObject({ center: -18.5 });
  });

  it("applies the backend validation rules", () => {
    const rw = { ...draftFromGenerator(null), kind: "randomWalk" as const };
    expect(draftToGenerator({ ...rw, spread: "-1" })).toBeUndefined();
    expect(draftToGenerator({ ...rw, step: "0" })).toBeUndefined();
    expect(draftToGenerator({ ...rw, center: "x" })).toBeUndefined();
    const sine = { ...draftFromGenerator(null), kind: "sine" as const };
    expect(draftToGenerator({ ...sine, period: "0" })).toBeUndefined();
    const counter = { ...draftFromGenerator(null), kind: "counter" as const };
    expect(draftToGenerator({ ...counter, ratePerHour: "-1" })).toBeUndefined();
  });

  it("needs exactly 24 profile points", () => {
    expect(parsePoints("1 2 3")).toBeNull();
    expect(parsePoints(Array.from({ length: 24 }, () => "1.5").join(" "))).toHaveLength(24);
    expect(parsePoints(`${Array.from({ length: 23 }, () => "1").join(",")},x`)).toBeNull();
  });
});

describe("savedGenerators", () => {
  const sine: GeneratorConfig = { kind: "sine", period: 60, amplitude: 1, center: null };
  const walk: GeneratorConfig = { kind: "randomWalk", center: 1, spread: 1, step: null };

  it("sets the override of the item and keeps the others", () => {
    expect(savedGenerators({ a: walk }, "b", sine)).toEqual({ a: walk, b: sine });
    expect(savedGenerators({ a: walk }, "a", sine)).toEqual({ a: sine });
  });

  it("removes the override when saving 'no generator' (F8)", () => {
    expect(savedGenerators({ a: walk, b: sine }, "b", null)).toEqual({ a: walk });
    expect(savedGenerators({}, "b", null)).toEqual({});
  });
});
