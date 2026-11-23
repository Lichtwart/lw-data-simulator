import { describe, expect, it } from "vitest";

import { textIn } from "./text";

describe("textIn", () => {
  it("uses a plain string for both languages", () => {
    expect(textIn("Kuehlung", "de")).toBe("Kuehlung");
    expect(textIn("Kuehlung", "en")).toBe("Kuehlung");
  });

  it("picks the language of a {de, en} text", () => {
    const text = { de: "Kühlung", en: "Cooling" };
    expect(textIn(text, "de")).toBe("Kühlung");
    expect(textIn(text, "en")).toBe("Cooling");
  });
});
