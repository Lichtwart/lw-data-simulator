import { describe, expect, it } from "vitest";

import { ApiError } from "./api";
import { deviceLockOf } from "./errors";

describe("deviceLockOf", () => {
  it("reads user and since from a deviceLocked error", () => {
    const e = new ApiError({
      code: "deviceLocked",
      details: { user: "anna", since: "2026-10-12T12:00:00Z" },
    });
    expect(deviceLockOf(e)).toEqual({ user: "anna", since: "2026-10-12T12:00:00Z" });
  });
  it("is null for everything else", () => {
    expect(deviceLockOf(new ApiError({ code: "noSession" }))).toBeNull();
    expect(deviceLockOf(new Error("x"))).toBeNull();
  });
});
