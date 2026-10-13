import { useCallback, useEffect, useState } from "react";
import { useLocation } from "react-router";

import {
  isToggleShortcut,
  NARROW_QUERY,
  readCollapsed,
  type SidebarLayout,
  sidebarLayout,
  writeCollapsed,
} from "./sidebar";

function storage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

function useMediaQuery(query: string): boolean {
  const get = () => window.matchMedia?.(query).matches ?? false;
  const [matches, setMatches] = useState(get);
  useEffect(() => {
    const mq = window.matchMedia?.(query);
    if (!mq) return;
    const onChange = () => setMatches(mq.matches);
    onChange();
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [query]);
  return matches;
}

/**
 * State of the collapsible left menu: collapsed (remembered per viewer) on wide windows,
 * an overlay on narrow ones, a peek while the pointer is at the left edge, Cmd/Ctrl+\ toggles.
 */
export function useSidebar(): {
  layout: SidebarLayout;
  narrow: boolean;
  toggle: () => void;
  close: () => void;
  setPeek: (peek: boolean) => void;
} {
  const narrow = useMediaQuery(NARROW_QUERY);
  const [collapsed, setCollapsed] = useState(() => readCollapsed(storage()));
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [peek, setPeek] = useState(false);
  const { pathname } = useLocation();

  const toggle = useCallback(() => {
    setPeek(false);
    if (narrow) {
      setOverlayOpen((o) => !o);
      return;
    }
    setCollapsed((c) => {
      writeCollapsed(storage(), !c);
      return !c;
    });
  }, [narrow]);

  const close = useCallback(() => {
    setOverlayOpen(false);
    setPeek(false);
  }, []);

  // Navigating from the overlay closes it; so does switching between narrow and wide.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs on route / width change
  useEffect(() => close(), [pathname, narrow, close]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isToggleShortcut(e)) {
        e.preventDefault();
        toggle();
      } else if (e.key === "Escape") {
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggle, close]);

  return {
    layout: sidebarLayout({ narrow, collapsed, overlayOpen, peek }),
    narrow,
    toggle,
    close,
    setPeek,
  };
}
