import { BellOffIcon, ZapIcon } from "lucide-react";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { api, type FaultDefinition, type OpenAlarmStatus } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { usePending } from "@/lib/usePending";
import { cn } from "@/lib/utils";

import { alarmStatusBadge, decodeStatuses, runClearAll } from "./faults";

/** Status badge of a fault whose alarm is open in Cumulocity. */
export function AlarmStatusBadge({ status }: { status: OpenAlarmStatus }) {
  const { t } = useTranslation();
  const badge = alarmStatusBadge(status);
  return <Badge variant={badge.variant}>{t(badge.key)}</Badge>;
}

/** One alarm: text, type + severity, status while in effect, a button that fires it once. */
export function FaultRow({
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
    <li className="flex items-center justify-between gap-2 py-1.5">
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-sm text-strong" title={label}>
          {label}
        </span>
        <span className="truncate font-mono text-xs text-muted-foreground">
          {fault.alarmType} · {fault.severity}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {status && <AlarmStatusBadge status={status} />}
        <Button
          size="sm"
          variant="outline"
          className="h-7"
          aria-label={t("control.fireLabel", { name: label })}
          title={t("control.fireLabel", { name: label })}
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

/** Fires faults; a second click on the same alarm waits for the first call. */
export function FaultList({
  faults,
  statuses,
  className,
}: {
  faults: readonly FaultDefinition[];
  /** `encodeStatuses` of the faults in effect. */
  statuses: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const { run, isPending } = usePending();
  const byType = decodeStatuses(statuses);
  return (
    <ul className={cn("divide-y", className)}>
      {faults.map((f) => {
        const label = f.target.kind === "slaveTimeout" ? t("control.slaveTimeout") : f.text;
        return (
          <FaultRow
            key={f.alarmType}
            fault={f}
            label={label}
            status={byType.get(f.alarmType) ?? null}
            disabled={isPending(f.alarmType)}
            onFire={() =>
              void run(f.alarmType, () =>
                api.fireFault(f.target).catch((e) => toast.error(errorMessage(e))),
              )
            }
          />
        );
      })}
    </ul>
  );
}

/**
 * Alarm card in the control list: DALI bus alarms, a slave's timeout + alarms, or a custom
 * alarm item. Each alarm fires once and stays in effect until it is cleared in Cumulocity (or
 * with "Clear all"), DECISIONS D9.
 */
export const AlarmCard = memo(function AlarmCard({
  title,
  subtitle,
  faults,
  statuses,
}: {
  title: string;
  subtitle: string | null;
  faults: readonly FaultDefinition[];
  statuses: string;
}) {
  const { t } = useTranslation();
  const active = statuses !== "";
  return (
    <article
      aria-label={title}
      className={cn(
        "flex w-full flex-col gap-2 rounded-lg border bg-card p-4 shadow-xs",
        active && "border-destructive",
      )}
    >
      <header className="flex min-w-0 flex-col">
        <span className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
          {t("control.alarms")}
        </span>
        <h3 className="truncate text-sm font-bold text-strong" title={title}>
          {title}
        </h3>
        {subtitle && (
          <span className="truncate font-mono text-xs text-muted-foreground">{subtitle}</span>
        )}
      </header>
      <FaultList faults={faults} statuses={statuses} />
    </article>
  );
});

/**
 * "Clear all": sets every open alarm of the device to CLEARED. It does not depend on the fault
 * catalog (which may be empty or have failed to load), so it is always offered.
 */
export function ClearAllAlarms({ deviceId }: { deviceId: string }) {
  const { t } = useTranslation();
  const { run, isPending } = usePending();
  const [confirm, setConfirm] = useState(false);
  const clearAll = () =>
    run("clearAll", () =>
      runClearAll(() => api.clearAllAlarms(deviceId), {
        closeDialog: () => setConfirm(false),
        success: () => toast.success(t("control.allClearing")),
        failure: (e) => toast.error(errorMessage(e)),
      }),
    );
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        className="h-7"
        disabled={isPending("clearAll")}
        onClick={() => setConfirm(true)}
      >
        <BellOffIcon />
        {t("control.clearAll")}
      </Button>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t("control.clearAllTitle")}
        description={t("control.clearAllText")}
        confirmLabel={t("control.clearAll")}
        destructive
        busy={isPending("clearAll")}
        onConfirm={() => void clearAll()}
      />
    </>
  );
}
