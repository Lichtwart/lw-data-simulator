import { describe, expect, it } from "vitest";

import {
  isToggleShortcut,
  readCollapsed,
  SIDEBAR_STORAGE_KEY,
  shortcutLabel,
  sidebarLayout,
  writeCollapsed,
} from "./sidebar";

const memory = () => {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
  };
};

const throwing = {
  getItem: (): string | null => {
    throw new Error("blocked");
  },
  setItem: () => {
    throw new Error("blocked");
  },
};

describe("sidebar state", () => {
  it("remembers the collapsed choice", () => {
    const s = memory();
    expect(readCollapsed(s)).toBe(false);
    writeCollapsed(s, true);
    expect(s.getItem(SIDEBAR_STORAGE_KEY)).toBe("1");
    expect(readCollapsed(s)).toBe(true);
    writeCollapsed(s, false);
    expect(readCollapsed(s)).toBe(false);
  });

  it("works without storage", () => {
    expect(readCollapsed(undefined)).toBe(false);
    expect(readCollapsed(throwing)).toBe(false);
    expect(() => writeCollapsed(throwing, true)).not.toThrow();
  });
});

describe("sidebarLayout", () => {
  const base = { narrow: false, collapsed: false, overlayOpen: false, peek: false };

  it("docks on wide windows unless collapsed", () => {
    expect(sidebarLayout(base)).toBe("docked");
    expect(sidebarLayout({ ...base, collapsed: true })).toBe("hidden");
    expect(sidebarLayout({ ...base, collapsed: true, peek: true })).toBe("overlay");
  });

  it("never squeezes narrow windows: hidden, or overlay when opened", () => {
    expect(sidebarLayout({ ...base, narrow: true })).toBe("hidden");
    expect(sidebarLayout({ ...base, narrow: true, overlayOpen: true })).toBe("overlay");
    expect(sidebarLayout({ ...base, narrow: true, collapsed: true, peek: true })).toBe("hidden");
  });
});

describe("isToggleShortcut", () => {
  const key = { key: "\\", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
  it("accepts Cmd+\\ and Ctrl+\\ only", () => {
    expect(isToggleShortcut({ ...key, metaKey: true })).toBe(true);
    expect(isToggleShortcut({ ...key, ctrlKey: true })).toBe(true);
    expect(isToggleShortcut({ ...key, key: "#", code: "Backslash", ctrlKey: true })).toBe(true);
    expect(isToggleShortcut(key)).toBe(false);
    expect(isToggleShortcut({ ...key, metaKey: true, shiftKey: true })).toBe(false);
    expect(isToggleShortcut({ ...key, key: "b", metaKey: true })).toBe(false);
  });
});

describe("shortcutLabel", () => {
  it("uses the platform's modifier", () => {
    expect(shortcutLabel("MacIntel")).toBe("⌘\\");
    expect(shortcutLabel("Win32")).toBe("Ctrl+\\");
  });
});
