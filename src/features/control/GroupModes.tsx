import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { api, type DaliGroupState, type GroupMode } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { usePending } from "@/lib/usePending";

const MODES: GroupMode[] = ["on", "off", "auto"];

/** DALI group mode switches: on / off / auto (`set_group_mode`, PRD §7.2). */
export function GroupModes({ groups }: { groups: readonly DaliGroupState[] }) {
  const { t } = useTranslation();
  const { run, isPending } = usePending();
  if (groups.length === 0) return null;
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
        {t("control.groups")}
      </h3>
      <ul className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
        {groups.map((g) => (
          <li
            key={g.id}
            className="flex items-center justify-between gap-3 rounded-lg border bg-card px-4 py-2.5 shadow-xs"
          >
            <span className="min-w-0 truncate text-sm font-semibold text-strong">
              <span className="font-mono text-xs text-muted-foreground">{g.id}</span> {g.name}
            </span>
            <ToggleGroup
              type="single"
              value={g.mode}
              disabled={isPending(String(g.id))}
              aria-label={t("control.groupMode", { name: g.name })}
              onValueChange={(mode) => {
                if (!mode) return;
                void run(String(g.id), () =>
                  api
                    .setGroupMode(g.id, mode as GroupMode)
                    .catch((e) => toast.error(errorMessage(e))),
                );
              }}
            >
              {MODES.map((m) => (
                <ToggleGroupItem key={m} value={m}>
                  {t(`control.modes.${m}`)}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </li>
        ))}
      </ul>
    </section>
  );
}
