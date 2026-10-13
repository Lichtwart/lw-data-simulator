import { describe, expect, it } from "vitest";

import type { FaultDefinition, LibraryItem_Serialize } from "@/lib/api";

import {
  buildChipGroups,
  EMPTY_FILTER,
  type FilterableEntry,
  faultHaystack,
  filterEntries,
  inlineFaults,
  itemHaystack,
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

const slaveNames = new Map([
  ["kuehlung", "Kuehlung"],
  ["klima", "Klima Zone 1"],
]);

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

const toEntry = (i: LibraryItem_Serialize): FilterableEntry & { id: string } => ({
  id: i.id,
  type: "item",
  tags: i.tags,
  haystack: itemHaystack(i, i.kind === "modbus" ? slaveNames.get(i.slave) : null),
  faults: [],
  faultHaystacks: [],
});
const entries = items.map(toEntry);
const id = (e: { id: string }) => e.id;
const never = (_f: FaultDefinition) => false;

const fault = (alarmType: string, text: string): FaultDefinition => ({
  target: { kind: "customAlarm", itemId: alarmType },
  alarmType,
  text,
  severity: "MAJOR",
});

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
    const ids = (query: string) =>
      filterEntries(entries, { ...EMPTY_FILTER, query }, never).map(id);
    expect(ids("tiefkühler")).toEqual(["freezer_temp"]);
    expect(ids("freezer")).toEqual(["freezer_temp"]);
    expect(ids("ev_power")).toEqual(["ev_power"]);
    expect(ids("category:hvac")).toEqual(["ac_mode"]);
    expect(ids("zone 1")).toEqual(["ac_mode"]); // slave name "Klima Zone 1"
    expect(ids("klima modus")).toEqual(["ac_mode"]);
    expect(ids("klima freezer")).toEqual([]);
  });

  it("combines search and tag chips", () => {
    const result = filterEntries(
      entries,
      { ...EMPTY_FILTER, query: "temp", tags: new Set(["zone:shop"]) },
      never,
    ).map(id);
    expect(result).toEqual(["freezer_temp"]);
  });
});

describe("alarms in the list", () => {
  const short = fault("lw_lampFailure_1_03_shortCircuit", "Lamp Failure 1-03 Short Circuit");
  const gear = fault("lw_controlGearFailure_1_03", "Control Gear Failure 1-03");
  const lamp: FilterableEntry & { id: string } = {
    id: "light-3",
    type: "item",
    tags: ["retail", "category:lighting"],
    haystack: "verkaufsraum leuchte 3\nlight-3\ncategory:lighting",
    faults: [short, gear],
    faultHaystacks: [faultHaystack(short), faultHaystack(gear)],
  };
  const door = fault("lw_demo_01_door", "Kühlzelle: Tür offen");
  const slave: FilterableEntry & { id: string } = {
    id: "slave:kuehlung",
    type: "alarms",
    tags: ["retail", "category:cooling"],
    haystack: "slave kuehlung\nkuehlung",
    faults: [door],
    faultHaystacks: [faultHaystack(door)],
  };
  const all = [...entries, lamp, slave];

  it("finds alarm texts and types by the text search", () => {
    const ids = (query: string) => filterEntries(all, { ...EMPTY_FILTER, query }, never).map(id);
    expect(ids("short circuit")).toEqual(["light-3"]);
    expect(ids("lw_demo_01")).toEqual(["slave:kuehlung"]);
    expect(ids("tür offen")).toEqual(["slave:kuehlung"]);
    expect(ids("leuchte gear")).toEqual(["light-3"]);
  });

  it("filters alarm cards by their tags like items", () => {
    const cooling = new Set(["category:cooling"]);
    expect(filterEntries(all, { ...EMPTY_FILTER, tags: cooling }, never).map(id)).toEqual([
      "freezer_temp",
      "slave:kuehlung",
    ]);
  });

  it("shows only cards with an alarm in effect", () => {
    const active = (f: FaultDefinition) => f === door;
    expect(filterEntries(all, { ...EMPTY_FILTER, activeOnly: true }, active).map(id)).toEqual([
      "slave:kuehlung",
    ]);
    expect(filterEntries(all, { ...EMPTY_FILTER, activeOnly: true }, never)).toEqual([]);
  });

  it("lists an item's alarms on the card when in effect or found by the search", () => {
    const types = (query: string, isActive = never) =>
      inlineFaults(lamp, query, isActive).map((f) => f.alarmType);
    expect(types("")).toEqual([]);
    // The card itself matches: the alarms stay in the menu.
    expect(types("leuchte 3")).toEqual([]);
    expect(types("short")).toEqual([short.alarmType]);
    expect(types("leuchte short")).toEqual([short.alarmType]);
    expect(types("failure")).toEqual([short.alarmType, gear.alarmType]);
    expect(types("", (f) => f === gear)).toEqual([gear.alarmType]);
    // Alarm cards list every alarm.
    expect(inlineFaults(slave, "", never)).toEqual([door]);
  });
});
