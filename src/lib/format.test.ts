import { describe, expect, it } from "vitest";

import {
  formatCoordinate,
  formatNumber,
  formatSince,
  formatValue,
  isLockFresh,
  parseDecimal,
  parsePosition,
  remainingPercent,
  secondsRemaining,
  stepDecimals,
} from "./format";

describe("formatting", () => {
  it("formats values with unit and trims zeros", () => {
    expect(formatNumber(18.5, 2, "en")).toBe("18.5");
    expect(formatValue(50, "%", "en")).toBe("50 %");
    expect(formatValue(-18.256, "°C", "en")).toBe("-18.26 °C");
    expect(formatValue(3, null, "en")).toBe("3");
    expect(formatNumber(Number.NaN)).toBe("–");
  });

  it("derives slider decimals from the step", () => {
    expect(stepDecimals(1)).toBe(0);
    expect(stepDecimals(0.5)).toBe(1);
    expect(stepDecimals(0.05)).toBe(2);
    expect(stepDecimals(0)).toBe(2);
  });

  it("shows only the time for today and date + time otherwise", () => {
    const now = new Date(2026, 9, 12, 15, 0);
    expect(formatSince(new Date(2026, 9, 12, 9, 5).toISOString(), now, "de")).toBe("09:05");
    expect(formatSince(new Date(2026, 9, 10, 9, 5).toISOString(), now, "de")).toContain("09:05");
    expect(formatSince(new Date(2026, 9, 10, 9, 5).toISOString(), now, "de")).toContain(",");
    expect(formatSince("not a date")).toBe("–");
  });
});

describe("lock", () => {
  const now = Date.parse("2026-10-12T12:10:00Z");
  it("is fresh for 10 minutes only", () => {
    expect(isLockFresh({ since: "2026-10-12T12:05:00Z" }, now)).toBe(true);
    expect(isLockFresh({ since: "2026-10-12T12:00:00Z" }, now)).toBe(true);
    expect(isLockFresh({ since: "2026-10-12T11:59:59Z" }, now)).toBe(false);
    expect(isLockFresh(null, now)).toBe(false);
    expect(isLockFresh({ since: "garbage" }, now)).toBe(false);
  });
});

describe("coordinates", () => {
  it("parses decimals with comma or dot", () => {
    expect(parseDecimal("48,137")).toBe(48.137);
    expect(parseDecimal(" -11.5 ")).toBe(-11.5);
    expect(parseDecimal("")).toBeNull();
    expect(parseDecimal("1.2.3")).toBeNull();
    expect(parseDecimal("abc")).toBeNull();
  });
  it("validates the position pair", () => {
    expect(parsePosition("", "")).toEqual({ position: null, error: null });
    expect(parsePosition("48,1", "11,5")).toEqual({
      position: { lat: 48.1, lng: 11.5 },
      error: null,
    });
    expect(parsePosition("91", "11").error).toBe("latitude");
    expect(parsePosition("48", "").error).toBe("longitude");
    expect(parsePosition("48", "181").error).toBe("longitude");
  });
  it("rounds to 5 decimals", () => {
    expect(formatCoordinate(48.1370000001)).toBe("48.137");
    expect(formatCoordinate(11.57549999)).toBe("11.5755");
  });
});

describe("operation countdown", () => {
  const received = "2026-10-12T12:00:00Z";
  const expires = "2026-10-12T12:01:00Z";
  it("counts whole seconds down and never below zero", () => {
    expect(secondsRemaining(expires, Date.parse(received))).toBe(60);
    expect(secondsRemaining(expires, Date.parse("2026-10-12T12:00:59.200Z"))).toBe(1);
    expect(secondsRemaining(expires, Date.parse("2026-10-12T12:02:00Z"))).toBe(0);
    expect(secondsRemaining("bad", 0)).toBe(0);
  });
  it("reports the remaining share", () => {
    expect(remainingPercent(received, expires, Date.parse(received))).toBe(100);
    expect(remainingPercent(received, expires, Date.parse("2026-10-12T12:00:30Z"))).toBe(50);
    expect(remainingPercent(received, expires, Date.parse("2026-10-12T12:05:00Z"))).toBe(0);
  });
});
