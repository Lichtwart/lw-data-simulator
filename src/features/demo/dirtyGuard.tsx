import { createContext, type ReactNode, useCallback, useContext, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useBlocker } from "react-router";

import { ConfirmDialog } from "@/components/ConfirmDialog";

const ReportContext = createContext<(id: string, dirty: boolean) => void>(() => {});

/**
 * Collects the "unsaved changes" state of the Setup cards and guards against losing them:
 * leaving the tab / screen asks first, closing the window gets the browser's confirmation.
 */
export function SetupDirtyGuard({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [dirtyIds, setDirtyIds] = useState<ReadonlySet<string>>(new Set());
  const report = useCallback((id: string, dirty: boolean) => {
    setDirtyIds((cur) => {
      if (cur.has(id) === dirty) return cur;
      const next = new Set(cur);
      if (dirty) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);
  const anyDirty = dirtyIds.size > 0;

  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      anyDirty && currentLocation.pathname !== nextLocation.pathname,
  );

  useEffect(() => {
    if (!anyDirty) return;
    const onBeforeUnload = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [anyDirty]);

  return (
    <ReportContext.Provider value={report}>
      {children}
      <ConfirmDialog
        open={blocker.state === "blocked"}
        onOpenChange={(o) => !o && blocker.state === "blocked" && blocker.reset()}
        title={t("setup.leaveTitle")}
        description={t("setup.leaveText")}
        confirmLabel={t("setup.leave")}
        destructive
        onConfirm={() => blocker.state === "blocked" && blocker.proceed()}
      />
    </ReportContext.Provider>
  );
}

/** Report whether the card `id` has unsaved edits. */
export function useReportDirty(id: string, dirty: boolean) {
  const report = useContext(ReportContext);
  useEffect(() => {
    report(id, dirty);
    return () => report(id, false);
  }, [id, dirty, report]);
}
