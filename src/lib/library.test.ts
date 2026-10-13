import { describe, expect, it } from "vitest";

import type { LibraryItem_Serialize } from "./api";
import { mergeScenarios, selectItems } from "./library";

describe("mergeScenarios", () => {
  const s = (id: string, name = id) => ({ id, name });
  it("replaces in place, hides, and appends new ones", () => {
    const base = [s("normal"), s("night"), s("fault")];
    const out = mergeScenarios(base, [s("new"), s("night", "Nacht 2")], ["fault"]);
    expect(out.map((x) => x.name)).toEqual(["normal", "Nacht 2", "new"]);
  });
  it("returns the library scenarios when nothing is edited", () => {
    const base = [s("a"), s("b")];
    expect(mergeScenarios(base, [], [])).toEqual(base);
  });
});

describe("selectItems", () => {
  const item = (id: string, tags: string[]): LibraryItem_Serialize => ({
    kind: "customAlarm",
    id,
    tags,
    title: { de: id, en: id },
    severity: "MAJOR",
    text: id,
  });
  it("is tags ∪ includes − excludes", () => {
    const items = [item("a", ["retail"]), item("b", ["other"]), item("c", ["retail"])];
    const sel = selectItems(items, { tags: ["retail"], includes: ["b"], excludes: ["c"] });
    expect(sel.map((i) => i.id)).toEqual(["a", "b"]);
  });
});
