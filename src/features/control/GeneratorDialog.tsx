import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api, type GeneratorConfig } from "@/lib/api";
import { errorMessage } from "@/lib/errors";

import { useDevice } from "../devices/DeviceContext";
import { GeneratorFields } from "./GeneratorFields";
import {
  draftFromGenerator,
  draftToGenerator,
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
          <GeneratorFields draft={d} onChange={set} idPrefix="gen" />
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
