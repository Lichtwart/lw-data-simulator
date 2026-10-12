import { describe, expect, it } from "vitest";

import {
  isValidDemoId,
  isValidDeviceAlarmName,
  isValidItemId,
  isValidSlaveName,
  isValidUnit,
  parseIntIn,
  parseNumberList,
  parseTagList,
  slugify,
} from "./parse";

describe("demo editor parsing", () => {
  it("parses number lists in range, deduplicated and sorted", () => {
    expect(parseNumberList("3, 1 2;2", 1, 64)).toEqual([1, 2, 3]);
    expect(parseNumberList("", 1, 64)).toEqual([]);
    expect(parseNumberList("0", 1, 64)).toBeNull();
    expect(parseNumberList("65", 1, 64)).toBeNull();
    expect(parseNumberList("1, x", 1, 64)).toBeNull();
  });

  it("parses tag lists", () => {
    expect(parseTagList(" retail, category:cooling ,,retail")).toEqual([
      "retail",
      "category:cooling",
    ]);
  });

  it("validates ids per the library rules", () => {
    expect(isValidItemId("freezer_temp")).toBe(true);
    expect(isValidItemId("a".repeat(19))).toBe(true);
    expect(isValidItemId("a".repeat(20))).toBe(false);
    expect(isValidItemId("bad id")).toBe(false);
    expect(isValidItemId("")).toBe(false);
  });

  it("rejects device alarm names that look like slave alarms (D2)", () => {
    expect(isValidDeviceAlarmName("smoke")).toBe(true);
    expect(isValidDeviceAlarmName("19_co2")).toBe(false);
    expect(isValidDeviceAlarmName("1_co2")).toBe(true);
  });

  it("validates slave names and units", () => {
    expect(isValidSlaveName("Kuehlung")).toBe(true);
    expect(isValidSlaveName("x".repeat(20))).toBe(false);
    expect(isValidSlaveName("  ")).toBe(false);
    expect(isValidUnit("kWh")).toBe(true);
    expect(isValidUnit("12345678")).toBe(false);
  });

  it("parses bounded integers", () => {
    expect(parseIntIn("5", 1, 10)).toBe(5);
    expect(parseIntIn("11", 1, 10)).toBeNull();
    expect(parseIntIn("1.5", 1, 10)).toBeNull();
    expect(parseIntIn("", 1, 10)).toBeNull();
  });

  it("slugifies names", () => {
    expect(slugify("Tankstelle Müller & Söhne")).toBe("tankstelle-muller-sohne");
  });

  it("validates demo ids as slugs, not register names", () => {
    expect(isValidDemoId("gas-station")).toBe(true);
    expect(isValidDemoId("a-very-long-demo-id-over-19")).toBe(true);
    expect(isValidDemoId("Gas_Station")).toBe(false);
    expect(isValidDemoId("-x")).toBe(false);
    expect(isValidDemoId("")).toBe(false);
  });
});
