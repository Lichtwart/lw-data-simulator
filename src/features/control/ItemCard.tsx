import { ActivityIcon, RotateCwIcon, TriangleAlertIcon, ZapIcon } from "lucide-react";
import { memo } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  api,
  type FaultDefinition,
  type ItemRuntimeState,
  type LibraryItem_Serialize,
} from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatValue } from "@/lib/format";

import { AlarmStatusBadge } from "./FaultSections";
import { decodeStatuses } from "./faults";
import { ItemControl } from "./ItemControl";
import type { Lang } from "./itemFilter";

interface Props {
  meta: LibraryItem_Serialize;
  state: ItemRuntimeState;
  lang: Lang;
  /** Faults of this lamp / sensor (DALI lamps, brightness and motion sensors). */
  lampFaults: readonly FaultDefinition[];
  /** Statuses of its faults in effect (`encodeStatuses`; a string keeps the memo cheap). */
  lampFaultStatuses: string;
  /** Name of the item's Modbus slave. */
  slaveName?: string | null;
  /** Open the generator editor for this item. */
  onEditGenerator?: (itemId: string) => void;
}

/** Await a command; toast on failure. Resolves `false` when it failed. */
async function run(action: Promise<unknown>): Promise<boolean> {
  try {
    await action;
    return true;
  } catch (e) {
    toast.error(errorMessage(e));
    return false;
  }
}

/**
 * One simulated item: value, control, generator state, per-lamp / per-sensor fault menu (each
 * entry fires its alarm once; it stays in effect until cleared in Cumulocity, DECISIONS D9).
 */
export const ItemCard = memo(function ItemCard({
  meta,
  state,
  lang,
  lampFaults,
  lampFaultStatuses,
  slaveName,
  onEditGenerator,
}: Props) {
  const { t } = useTranslation();
  const title = meta.title[lang];
  const unit =
    meta.kind === "customAlarm" ? null : (meta.unit ?? state.naming?.series[0]?.unit ?? null);
  const control = meta.kind === "customAlarm" ? null : (meta.control ?? null);
  const statuses = decodeStatuses(lampFaultStatuses);
  const generator = state.generator;

  return (
    <article
      aria-label={title}
      className="flex w-full flex-col gap-3 rounded-lg border bg-card p-4 shadow-xs"
    >
      <header className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 flex-col">
          <h3 className="truncate text-sm font-bold text-strong" title={title}>
            {title}
          </h3>
          <span className="truncate font-mono text-xs text-muted-foreground">
            {meta.id}
            {slaveName ? ` · ${slaveName}` : ""}
          </span>
        </div>
        <div className="flex shrink-0 items-center">
          {onEditGenerator && (
            <Button
              variant="ghost"
              size="icon"
              className="size-8"
              aria-label={t("control.editGenerator", { name: title })}
              title={t("control.editGenerator", { name: title })}
              onClick={() => onEditGenerator(meta.id)}
            >
              <ActivityIcon />
            </Button>
          )}
          {lampFaults.length > 0 && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant={statuses.size > 0 ? "secondary" : "ghost"}
                  size="icon"
                  className="size-8 shrink-0"
                  aria-label={t("control.faultMenu", { name: title })}
                >
                  <TriangleAlertIcon
                    className={statuses.size > 0 ? "text-destructive" : undefined}
                  />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-72">
                <DropdownMenuLabel>{t("control.fireFault")}</DropdownMenuLabel>
                {lampFaults.map((f) => {
                  const status = statuses.get(f.alarmType);
                  return (
                    <DropdownMenuItem
                      key={f.alarmType}
                      aria-label={t("control.fireLabel", { name: f.text })}
                      onSelect={() => void run(api.fireFault(f.target))}
                    >
                      <ZapIcon />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate">{f.text}</span>
                        <span className="font-mono text-xs text-muted-foreground">
                          {f.alarmType} · {f.severity}
                        </span>
                      </span>
                      {status && <AlarmStatusBadge status={status} />}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      </header>

      {control ? (
        <ItemControl
          control={control}
          value={state.value}
          unit={unit}
          label={title}
          lang={lang}
          onSet={(value) => run(api.setItemValue({ itemId: meta.id, value }))}
        />
      ) : (
        <span className="font-mono text-sm text-strong">
          {formatValue(state.value, unit, lang)}
        </span>
      )}

      {(generator || state.paused || state.manualOverride) && (
        <footer className="flex flex-wrap items-center gap-2">
          {generator && !state.manualOverride && (
            <Badge variant="info">
              {t("control.generatorRunning", { kind: t(`control.generators.${generator.kind}`) })}
            </Badge>
          )}
          {state.manualOverride && <Badge variant="warning">{t("control.manual")}</Badge>}
          {state.paused && <Badge variant="secondary">{t("control.paused")}</Badge>}
          {generator && state.manualOverride && (
            <Button
              size="sm"
              variant="outline"
              className="h-7"
              onClick={() => void run(api.resumeGenerator(meta.id))}
            >
              <RotateCwIcon />
              {t("control.resumeGenerator")}
            </Button>
          )}
        </footer>
      )}
    </article>
  );
});
