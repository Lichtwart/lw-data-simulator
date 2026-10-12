import { describe, expect, it } from "vitest";

import { draftToItem, EMPTY_DRAFT, type ItemDraft, itemToDraft, validateDraft } from "./itemDraft";

const none = new Set<string>();
const modbus: ItemDraft = {
  ...EMPTY_DRAFT,
  id: "freezer_temp",
  titleDe: "Tiefkühler",
  titleEn: "Freezer",
  slave: "kuehlung",
  unit: "°C",
  min: "-30",
  max: "0",
  step: "0.5",
  defaultValue: "-18",
  tags: "retail, category:cooling",
};

describe("item drafts", () => {
  it("accepts a valid modbus slider item and builds the library item", () => {
    expect(validateDraft(modbus, { takenIds: none })).toBeNull();
    expect(draftToItem(modbus)).toMatchObject({
      kind: "modbus",
      id: "freezer_temp",
      slave: "kuehlung",
      unit: "°C",
      title: { de: "Tiefkühler", en: "Freezer" },
      tags: ["retail", "category:cooling"],
      control: { kind: "slider", min: -30, max: 0, step: 0.5, default: -18 },
    });
  });

  it("reports the first problem", () => {
    expect(validateDraft({ ...modbus, id: "bad id" }, { takenIds: none })).toBe("id");
    expect(validateDraft(modbus, { takenIds: new Set(["freezer_temp"]) })).toBe("idTaken");
    expect(validateDraft({ ...modbus, titleEn: "" }, { takenIds: none })).toBe("title");
    expect(validateDraft({ ...modbus, slave: "" }, { takenIds: none })).toBe("slave");
    expect(validateDraft({ ...modbus, unit: "12345678" }, { takenIds: none })).toBe("unit");
    expect(validateDraft({ ...modbus, min: "5", max: "1" }, { takenIds: none })).toBe("control");
    expect(validateDraft({ ...modbus, defaultValue: "99" }, { takenIds: none })).toBe("control");
    expect(
      validateDraft({ ...modbus, registerType: "input", writable: true }, { takenIds: none }),
    ).toBe("writable");
  });

  it("does not check the id when editing", () => {
    expect(
      validateDraft(modbus, { takenIds: new Set(["freezer_temp"]), checkId: false }),
    ).toBeNull();
  });

  it("validates device alarms (D2 name rule, text required)", () => {
    const alarm: ItemDraft = {
      ...EMPTY_DRAFT,
      kind: "customAlarm",
      id: "smoke",
      titleDe: "Rauch",
      titleEn: "Smoke",
      text: "Smoke detected",
    };
    expect(validateDraft(alarm, { takenIds: none })).toBeNull();
    expect(validateDraft({ ...alarm, id: "12_smoke" }, { takenIds: none })).toBe("id");
    expect(validateDraft({ ...alarm, text: " " }, { takenIds: none })).toBe("text");
    expect(draftToItem(alarm)).toMatchObject({ kind: "customAlarm", severity: "MAJOR" });
  });

  it("round-trips an item through the draft", () => {
    const item = draftToItem(modbus);
    const back = itemToDraft({ ...item, tags: item.tags ?? [], writable: false } as never);
    expect(back).toMatchObject({ id: "freezer_temp", slave: "kuehlung", min: "-30", step: "0.5" });
  });

  it("does not edit native kinds", () => {
    expect(
      itemToDraft({ kind: "dali", id: "light-1", index: 1, title: { de: "a", en: "b" }, tags: [] }),
    ).toBeNull();
  });

  it("never accepts an empty id, even without the id check (edit mode)", () => {
    expect(validateDraft({ ...modbus, id: "" }, { takenIds: none, checkId: false })).toBe("id");
  });

  it("supports select controls", () => {
    const select: ItemDraft = {
      ...modbus,
      controlKind: "select",
      defaultValue: "1",
      options: [
        { value: "0", de: "Aus", en: "Off" },
        { value: "1", de: "Kühlen", en: "Cool" },
      ],
    };
    expect(validateDraft(select, { takenIds: none })).toBeNull();
    const item = draftToItem(select);
    expect(item).toMatchObject({
      control: {
        kind: "select",
        default: 1,
        options: [
          { value: 0, label: { de: "Aus", en: "Off" } },
          { value: 1, label: { de: "Kühlen", en: "Cool" } },
        ],
      },
    });
    const back = itemToDraft({ ...item, tags: [], writable: false } as never);
    expect(back).toMatchObject({ controlKind: "select", defaultValue: "1" });
    expect(back?.options).toHaveLength(2);
    expect(validateDraft({ ...select, defaultValue: "7" }, { takenIds: none })).toBe("control");
    expect(
      validateDraft(
        { ...select, options: [select.options[0] as never, select.options[0] as never] },
        { takenIds: none },
      ),
    ).toBe("control");
  });
});
