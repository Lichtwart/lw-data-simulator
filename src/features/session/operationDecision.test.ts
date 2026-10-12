import { describe, expect, it } from "vitest";

import { operationDecision } from "./operationDecision";

describe("operationDecision", () => {
  it("confirms without a reason", () => {
    expect(operationDecision("confirm", "ignored")).toEqual({ decision: "confirm" });
  });
  it("sends the trimmed abort reason, or none when blank (F19)", () => {
    expect(operationDecision("abort", "  Kunde wartet ")).toEqual({
      decision: "abort",
      reason: "Kunde wartet",
    });
    expect(operationDecision("abort", "   ")).toEqual({ decision: "abort", reason: null });
    expect(operationDecision("abort")).toEqual({ decision: "abort", reason: null });
  });
});
