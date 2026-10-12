import { BellOffIcon, ChevronRightIcon, ZapIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  type ActiveFault,
  api,
  type FaultDefinition,
  type OpenAlarmStatus,
  type SlaveDefinition,
} from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { usePending } from "@/lib/usePending";

import {
  alarmStatusBadge,
  faultStatus,
  runClearAll,
  type FaultSections as Sections,
} from "./faults";

/** Status badge of a fault whose alarm is open in Cumulocity. */
export function AlarmStatusBadge({ status }: { status: OpenAlarmStatus }) {
  const { t } = useTranslation();
  const badge = alarmStatusBadge(status);
  return <Badge variant={badge.variant}>{t(badge.key)}</Badge>;
}

function FaultRow({
  fault,
  label,
  status,
  disabled,
  onFire,
}: {
  fault: FaultDefinition;
  label: string;
  status: OpenAlarmStatus | null;
  disabled: boolean;
  onFire: () => void;
}) {
  const { t } = useTranslation();
  return (
    <li className="flex items-center justify-between gap-3 py-1.5">
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm text-strong">{label}</span>
        <span className="truncate font-mono text-xs text-muted-foreground">
          {fault.alarmType} · {fault.severity}
        </span>
      </div>
      <div className="flex items-center gap-2">
        {status && <AlarmStatusBadge status={status} />}
        <Button
          size="sm"
          variant="outline"
          aria-label={t("control.fireLabel", { name: label })}
          disabled={disabled}
          onClick={onFire}
        >
          <ZapIcon />
          {t("control.fire")}
        </Button>
      </div>
    </li>
  );
}

/**
 * Faults that are not tied to one lamp: DALI bus alarms, per-slave timeout + custom alarms,
 * device-level custom alarms. Each is a button that fires the alarm once; it stays in effect
 * until it is cleared in Cumulocity (or with "Clear all"), DECISIONS D9.
 */
export function FaultSectionsPanel({
  deviceId,
  sections,
  slaves,
  activeFaults,
}: {
  deviceId: string;
  sections: Sections;
  slaves: readonly SlaveDefinition[];
  activeFaults: readonly ActiveFault[];
}) {
  const { t } = useTranslation();
  const { run, isPending } = usePending();
  const [confirmClear, setConfirmClear] = useState(false);
  const slaveName = (key: string) => slaves.find((s) => s.key === key)?.name ?? key;
  const label = (f: FaultDefinition) =>
    f.target.kind === "slaveTimeout" ? t("control.slaveTimeout") : f.text;

  const blocks: { key: string; title: string; faults: FaultDefinition[] }[] = [];
  if (sections.daliBus.length)
    blocks.push({ key: "bus", title: t("control.daliBus"), faults: sections.daliBus });
  for (const [key, faults] of sections.slaves)
    blocks.push({
      key: `slave:${key}`,
      title: t("control.slave", { name: slaveName(key) }),
      faults,
    });
  if (sections.customAlarms.length)
    blocks.push({ key: "custom", title: t("control.customAlarms"), faults: sections.customAlarms });

  const activeCount = activeFaults.length;

  const clearAll = () =>
    run("clearAll", () =>
      runClearAll(() => api.clearAllAlarms(deviceId), {
        closeDialog: () => setConfirmClear(false),
        success: () => toast.success(t("control.allClearing")),
        failure: (e) => toast.error(errorMessage(e)),
      }),
    );

  // "Clear all" does not depend on the fault catalog (it may be empty or have failed to load),
  // so it is always offered; the fault lists are folded away.
  const clearAllRow = (
    <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
      <p className="text-sm text-muted-foreground">{t("control.faultsHint")}</p>
      <Button
        size="sm"
        variant="outline"
        disabled={isPending("clearAll")}
        onClick={() => setConfirmClear(true)}
      >
        <BellOffIcon />
        {t("control.clearAll")}
      </Button>
    </div>
  );

  return (
    <div className="rounded-lg border bg-card shadow-xs">
      <div className="flex items-center gap-2 px-4 pt-3 text-sm font-bold text-strong">
        {t("control.faultsAndAlarms")}
        {activeCount > 0 && (
          <Badge variant="danger">{t("control.activeFaults", { count: activeCount })}</Badge>
        )}
      </div>
      {clearAllRow}
      {blocks.length > 0 && (
        <details className="group border-t">
          <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 text-sm font-bold text-strong [&::-webkit-details-marker]:hidden">
            <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
            {t("control.faultList")}
          </summary>
          <div className="grid gap-x-8 gap-y-4 border-t px-4 py-3 md:grid-cols-2 xl:grid-cols-3">
            {blocks.map((b) => (
              <section key={b.key} className="flex flex-col">
                <h4 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
                  {b.title}
                </h4>
                <ul className="divide-y">
                  {b.faults.map((f) => {
                    const key = `${f.target.kind}:${f.alarmType}`;
                    return (
                      <FaultRow
                        key={key}
                        fault={f}
                        label={label(f)}
                        status={faultStatus(activeFaults, f.target)}
                        disabled={isPending(key)}
                        onFire={() =>
                          void run(key, () =>
                            api.fireFault(f.target).catch((e) => toast.error(errorMessage(e))),
                          )
                        }
                      />
                    );
                  })}
                </ul>
              </section>
            ))}
          </div>
        </details>
      )}
      <ConfirmDialog
        open={confirmClear}
        onOpenChange={setConfirmClear}
        title={t("control.clearAllTitle")}
        description={t("control.clearAllText")}
        confirmLabel={t("control.clearAll")}
        destructive
        busy={isPending("clearAll")}
        onConfirm={() => void clearAll()}
      />
    </div>
  );
}
