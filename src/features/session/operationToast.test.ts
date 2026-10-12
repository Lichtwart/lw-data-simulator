import { describe, expect, it } from "vitest";

import { operationToast } from "./operationToast";

describe("operationToast", () => {
  it("does not toast a deliberate stop", () => {
    expect(operationToast({ resolution: "stopped", reason: "Simulator stopped" })).toBeNull();
  });
  it("still reports real errors with the reason", () => {
    expect(operationToast({ resolution: "error", reason: "boom" })).toEqual({
      level: "error",
      key: "operations.failed",
      description: "boom",
    });
  });
  it("is quiet for answered operations", () => {
    expect(operationToast({ resolution: "confirmed", reason: null })).toBeNull();
    expect(operationToast({ resolution: "aborted", reason: "x" })).toBeNull();
  });
});
