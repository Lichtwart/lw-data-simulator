import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { api, type GeneratorConfig } from "@/lib/api";
import { errorMessage } from "@/lib/errors";

import { useDevice } from "../devices/DeviceContext";
import {
  draftFromGenerator,
  draftToGenerator,
  GENERATOR_KINDS,
  type GeneratorDraft,
  savedGenerators,
} from "./generatorDraft";

export interface GeneratorTarget {
  id: string;
  title: string;
  generator: GeneratorConfig | null;
  /** Control default, used to seed a new generator's center. */
  center: number;
}

/**
 * Edit an item's generator (PRD §8). "Apply" changes it for the running session
 * (`set_item_generator`); "Apply and save" also persists it as an override in
 * `lw_demo.generators` (`update_demo_setup`), so it survives the next session.
 */
export function GeneratorDialog({
  item,
  onClose,
}: {
  /** Item to edit; `null` = closed. */
  item: GeneratorTarget | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { device, reload } = useDevice();
  const [d, setD] = useState<GeneratorDraft>(() => draftFromGenerator(null));
  const [busy, setBusy] = useState(false);

  // Seed the form when the dialog opens for an item (not on every live value tick).
  const itemRef = useRef(item);
  itemRef.current = item;
  const itemId = item?.id ?? null;
  // biome-ignore lint/correctness/useExhaustiveDependencies: seed once per opened item, read via ref
  useEffect(() => {
    const it = itemRef.current;
    if (it) setD(draftFromGenerator(it.generator, it.center));
  }, [itemId]);

  const set = (p: Partial<GeneratorDraft>) => setD((x) => ({ ...x, ...p }));
  const generator = draftToGenerator(d);
  const invalid = generator === undefined;

  async function apply(save: boolean) {
    if (!item || generator === undefined) return;
    setBusy(true);
    try {
      // Persist first: the running session takes the saved setup over (an override that was
      // removed falls back to the library generator), and then gets the chosen generator.
      if (save) {
        await api.updateDemoSetup(device.id, {
          tags: null,
          includes: null,
          excludes: null,
          daliLampCount: null,
          sendIntervalSec: null,
          realisticMode: null,
          customSlaves: null,
          customItems: null,
          generators: savedGenerators(device.demo?.generators ?? {}, item.id, generator),
        });
        reload();
      }
      await api.setItemGenerator(item.id, generator);
      toast.success(save ? t("generator.saved") : t("generator.applied"));
      onClose();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={item !== null} onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("generator.title", { name: item?.title })}</DialogTitle>
          <DialogDescription>{t("generator.hint")}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <Field label={t("generator.kind")} htmlFor="gen-kind">
            <Select
              value={d.kind}
              onValueChange={(v) => set({ kind: v as GeneratorDraft["kind"] })}
            >
              <SelectTrigger id="gen-kind">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {GENERATOR_KINDS.map((k) => (
                  <SelectItem key={k} value={k}>
                    {k === "none" ? t("generator.none") : t(`control.generators.${k}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          {d.kind === "randomWalk" && (
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("generator.center")} htmlFor="gen-center">
                <Input
                  id="gen-center"
                  className="font-mono"
                  value={d.center}
                  onChange={(e) => set({ center: e.target.value })}
                />
              </Field>
              <Field label={t("generator.spread")} htmlFor="gen-spread">
                <Input
                  id="gen-spread"
                  className="font-mono"
                  value={d.spread}
                  onChange={(e) => set({ spread: e.target.value })}
                />
              </Field>
              <Field label={t("generator.step")} htmlFor="gen-step">
                <Input
                  id="gen-step"
                  className="font-mono"
                  value={d.step}
                  onChange={(e) => set({ step: e.target.value })}
                />
              </Field>
            </div>
          )}
          {d.kind === "sine" && (
            <div className="grid grid-cols-3 gap-3">
              <Field label={t("generator.period")} htmlFor="gen-period">
                <Input
                  id="gen-period"
                  className="font-mono"
                  value={d.period}
                  onChange={(e) => set({ period: e.target.value })}
                />
              </Field>
              <Field label={t("generator.amplitude")} htmlFor="gen-amp">
                <Input
                  id="gen-amp"
                  className="font-mono"
                  value={d.amplitude}
                  onChange={(e) => set({ amplitude: e.target.value })}
                />
              </Field>
              <Field label={t("generator.centerOptional")} htmlFor="gen-center">
                <Input
                  id="gen-center"
                  className="font-mono"
                  value={d.center}
                  onChange={(e) => set({ center: e.target.value })}
                />
              </Field>
            </div>
          )}
          {d.kind === "dailyProfile" && (
            <Field
              label={t("generator.points")}
              htmlFor="gen-points"
              hint={t("generator.pointsHint")}
            >
              <Textarea
                id="gen-points"
                rows={3}
                className="font-mono"
                value={d.points}
                onChange={(e) => set({ points: e.target.value })}
              />
            </Field>
          )}
          {d.kind === "counter" && (
            <Field label={t("generator.rate")} htmlFor="gen-rate" className="max-w-xs">
              <Input
                id="gen-rate"
                className="font-mono"
                value={d.ratePerHour}
                onChange={(e) => set({ ratePerHour: e.target.value })}
              />
            </Field>
          )}
          {invalid && <p className="text-xs text-destructive">{t("generator.invalid")}</p>}
          {d.kind === "none" && (
            <p className="text-xs text-muted-foreground">{t("generator.noneHint")}</p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="outline" disabled={busy || invalid} onClick={() => void apply(false)}>
            {t("generator.apply")}
          </Button>
          <Button disabled={busy || invalid || generator === null} onClick={() => void apply(true)}>
            {t("generator.applySave")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
