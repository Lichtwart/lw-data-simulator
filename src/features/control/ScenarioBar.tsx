import { RotateCcwIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Button } from "@/components/ui/button";
import { api, type Scenario_Serialize } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { usePending } from "@/lib/usePending";

import type { Lang } from "./itemFilter";

const RESET = "__reset";

/**
 * Scenarios of the device (name + description, PRD §8) and "Reset to defaults": every value
 * back to its default, generators restored, every open alarm cleared (counters keep their value).
 */
export function ScenarioBar({
  scenarios,
  lang,
}: {
  scenarios: readonly Scenario_Serialize[];
  lang: Lang;
}) {
  const { t } = useTranslation();
  const calls = usePending();
  const [confirmReset, setConfirmReset] = useState(false);

  const reset = () =>
    calls.run(RESET, () =>
      api
        .resetToDefaults()
        .then(() => {
          setConfirmReset(false);
          toast.success(t("control.resetDone"));
        })
        .catch((e) => toast.error(errorMessage(e))),
    );

  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
          {t("control.scenarios")}
        </h3>
        <Button
          size="sm"
          variant="outline"
          disabled={calls.isPending(RESET)}
          onClick={() => setConfirmReset(true)}
        >
          <RotateCcwIcon />
          {t("control.resetDefaults")}
        </Button>
      </div>
      {scenarios.length > 0 && (
        <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(14rem,100%),1fr))] gap-2">
          {scenarios.map((s) => (
            <li key={s.id} className="flex">
              <Button
                variant="outline"
                className="h-auto w-full flex-col items-start gap-0.5 py-2 text-left whitespace-normal"
                disabled={calls.isPending(s.id)}
                title={s.description?.[lang]}
                onClick={() =>
                  void calls.run(s.id, () =>
                    api
                      .applyScenario(s.id)
                      .then(() =>
                        toast.success(t("control.scenarioApplied", { name: s.name[lang] })),
                      )
                      .catch((e) => toast.error(errorMessage(e))),
                  )
                }
              >
                <span className="flex flex-wrap items-baseline gap-1.5">
                  <span className="font-semibold text-strong">{s.name[lang]}</span>
                  {s.start && (
                    <span className="text-xs font-normal text-muted-foreground">
                      {t("control.scenarioStart")}
                    </span>
                  )}
                </span>
                {s.description && (
                  <span className="line-clamp-2 text-xs font-normal text-muted-foreground">
                    {s.description[lang]}
                  </span>
                )}
              </Button>
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={confirmReset}
        onOpenChange={setConfirmReset}
        title={t("control.resetDefaultsTitle")}
        description={t("control.resetDefaultsText")}
        confirmLabel={t("control.resetDefaults")}
        destructive
        busy={calls.isPending(RESET)}
        onConfirm={() => void reset()}
      />
    </section>
  );
}
