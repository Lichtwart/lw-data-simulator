import { describe, expect, it } from "vitest";

import { defaultContentLanguage, uiLanguage } from "./contentLanguage";

describe("contentLanguage", () => {
  it("maps the UI language", () => {
    expect(uiLanguage("en")).toBe("en");
    expect(uiLanguage("en-GB")).toBe("en");
    expect(uiLanguage("de")).toBe("de");
    expect(uiLanguage(undefined)).toBe("de");
  });

  it("defaults to the device language, not the UI language", () => {
    expect(defaultContentLanguage({ demoId: "retail", language: "en" }, "de")).toBe("en");
    expect(defaultContentLanguage({ demoId: "retail", language: null }, "en")).toBe("de");
    expect(defaultContentLanguage({ demoId: "retail" }, "en")).toBe("de");
  });

  it("uses the UI language for a device without a demo", () => {
    expect(defaultContentLanguage(null, "en")).toBe("en");
    expect(defaultContentLanguage({ demoId: null, language: null }, "en")).toBe("en");
  });
});
