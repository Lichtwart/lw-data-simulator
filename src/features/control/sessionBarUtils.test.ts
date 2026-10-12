import { describe, expect, it } from "vitest";

import { clampInterval } from "./sessionBarUtils";

describe("clampInterval", () => {
  it("keeps whole seconds within 5..899", () => {
    expect(clampInterval(30)).toBe(30);
    expect(clampInterval(30.6)).toBe(31);
    expect(clampInterval(0)).toBe(5);
    expect(clampInterval(4)).toBe(5);
    expect(clampInterval(900)).toBe(899);
    expect(clampInterval(10_000)).toBe(899);
    expect(clampInterval(Number.NaN)).toBeNull();
  });
});
