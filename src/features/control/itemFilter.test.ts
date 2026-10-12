import { describe, expect, it } from "vitest";

import type { LibraryItem_Serialize, SlaveDefinition } from "@/lib/api";

import {
  buildChipGroups,
  filterItems,
  matchesQuery,
  matchesTags,
  tagNamespace,
  toggleTag,
} from "./itemFilter";

const item = (
  id: string,
  de: string,
  en: string,
  tags: string[],
  slave = "kuehlung",
): LibraryItem_Serialize => ({
  kind: "modbus",
  id,
  slave,
  title: { de, en },
  tags,
  writable: false,
});

const slaves: SlaveDefinition[] = [
  { key: "kuehlung", name: "Kuehlung", requestInterval: 10 },
  { key: "klima", name: "Klima Zone 1", requestInterval: 10 },
];

const items = [
  item("freezer_temp", "Tiefkühler Temperatur", "Freezer temperature", [
    "retail",
    "gas-station",
    "category:cooling",
    "zone:shop",
  ]),
  item("ac_mode", "Klima Modus", "AC mode", ["retail", "category:hvac", "zone:shop"], "klima"),
  item("ev_power", "Ladeleistung", "Charging power", [
    "gas-station",
    "category:ev-charging",
    "zone:outside",
  ]),
];

describe("tags", () => {
  it("splits the namespace", () => {
    expect(tagNamespace("category:cooling")).toBe("category");
    expect(tagNamespace("retail")).toBe("");
  });

  it("builds chip groups: client tags first, then namespaces alphabetically, with counts", () => {
    const groups = buildChipGroups(items);
    expect(groups.map((g) => g.namespace)).toEqual(["", "category", "zone"]);
    expect(groups[0]?.tags).toEqual([
      { tag: "gas-station", count: 2 },
      { tag: "retail", count: 2 },
    ]);
    expect(groups[2]?.tags.find((t) => t.tag === "zone:shop")?.count).toBe(2);
  });

  it("ORs inside a namespace and ANDs across namespaces", () => {
    const sel = new Set(["category:cooling", "category:hvac", "zone:shop"]);
    expect(matchesTags(["category:hvac", "zone:shop"], sel)).toBe(true);
    expect(matchesTags(["category:hvac", "zone:outside"], sel)).toBe(false);
    expect(matchesTags(["zone:shop"], sel)).toBe(false);
    expect(matchesTags([], new Set())).toBe(true);
  });

  it("toggles without mutating", () => {
    const a = new Set(["x"]);
    const b = toggleTag(a, "y");
    expect([...a]).toEqual(["x"]);
    expect([...b].sort()).toEqual(["x", "y"]);
    expect([...toggleTag(b, "x")]).toEqual(["y"]);
  });
});

describe("search", () => {
  it("requires every word (AND), case-insensitive", () => {
    expect(matchesQuery("tiefkühler temperatur", "tief TEMP")).toBe(true);
    expect(matchesQuery("tiefkühler temperatur", "tief klima")).toBe(false);
    expect(matchesQuery("anything", "   ")).toBe(true);
  });

  it("searches title (de + en), id, tags and slave name", () => {
    const none = new Set<string>();
    const ids = (query: string) =>
      filterItems(items, { query, tags: none }, slaves).map((i) => i.id);
    expect(ids("tiefkühler")).toEqual(["freezer_temp"]);
    expect(ids("freezer")).toEqual(["freezer_temp"]);
    expect(ids("ev_power")).toEqual(["ev_power"]);
    expect(ids("category:hvac")).toEqual(["ac_mode"]);
    expect(ids("zone 1")).toEqual(["ac_mode"]); // slave name "Klima Zone 1"
    expect(ids("klima modus")).toEqual(["ac_mode"]);
    expect(ids("klima freezer")).toEqual([]);
  });

  it("combines search and tag chips", () => {
    const result = filterItems(items, { query: "temp", tags: new Set(["zone:shop"]) }, slaves).map(
      (i) => i.id,
    );
    expect(result).toEqual(["freezer_temp"]);
  });
});
