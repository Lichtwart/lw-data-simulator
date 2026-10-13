/** Collapsible left menu: pure decisions (persisted state, layout, shortcut). */

export const SIDEBAR_STORAGE_KEY = "lw.sidebarCollapsed";

/** Below this width the menu never squeezes the content: it is hidden or overlays it. */
export const NARROW_QUERY = "(max-width: 1023px)";

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;

/** Stored "collapsed" choice of this viewer; `false` when nothing (readable) is stored. */
export function readCollapsed(storage: Storage | undefined): boolean {
  try {
    return storage?.getItem(SIDEBAR_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeCollapsed(storage: Storage | undefined, collapsed: boolean): void {
  try {
    storage?.setItem(SIDEBAR_STORAGE_KEY, collapsed ? "1" : "0");
  } catch {
    // Private window / blocked storage: the choice just isn't remembered.
  }
}

/**
 * How the menu is shown. `docked`: in the flow next to the content. `overlay`: above the
 * content (narrow window opened by the user, or a peek from the left edge). `hidden`: content
 * takes the full width.
 */
export type SidebarLayout = "docked" | "overlay" | "hidden";

export function sidebarLayout(s: {
  narrow: boolean;
  collapsed: boolean;
  overlayOpen: boolean;
  peek: boolean;
}): SidebarLayout {
  if (s.narrow) return s.overlayOpen ? "overlay" : "hidden";
  if (!s.collapsed) return "docked";
  return s.peek ? "overlay" : "hidden";
}

/** Cmd+\ (macOS) / Ctrl+\ toggles the menu. */
export function isToggleShortcut(e: {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): boolean {
  if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return false;
  return e.key === "\\" || e.code === "Backslash";
}

/** Shortcut hint for tooltips (key names are not translated). */
export function shortcutLabel(platform: string): string {
  return /Mac|iPhone|iPad/.test(platform) ? "⌘\\" : "Ctrl+\\";
}
