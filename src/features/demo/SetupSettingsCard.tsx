import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { useSeededForm } from "@/lib/useSeededForm";

import { MAX_SEND_INTERVAL_SEC, MIN_SEND_INTERVAL_SEC } from "../control/sessionBarUtils";
import { useDevice } from "../devices/DeviceContext";
import { useReportDirty } from "./dirtyGuard";
import { parseIntIn, parseTagList } from "./parse";
import { SectionCard } from "./SectionCard";

interface Form {
  lamps: string;
  interval: string;
  realistic: boolean;
  tags: string;
  includes: string;
  excludes: string;
}

/** Basic `lw_demo` setup: lamp count, send interval, realistic mode, tags / includes / excludes. */
export function SetupSettingsCard() {
  const { t } = useTranslation();
  const { device, reload } = useDevice();
  const demo = device.demo;
  const seed = useMemo<Form>(
    () => ({
      lamps: String(demo?.daliLampCount ?? ""),
      interval: String(demo?.sendIntervalSec ?? ""),
      realistic: demo?.realisticMode ?? false,
      tags: (demo?.tags ?? []).join(", "),
      includes: (demo?.includes ?? []).join(", "),
      excludes: (demo?.excludes ?? []).join(", "),
    }),
    [demo],
  );
  const { value: form, setValue: setForm, dirty, discard, markSaved } = useSeededForm(seed);
  useReportDirty("settings", dirty);
  const [busy, setBusy] = useState(false);
  const { lamps, interval, realistic, tags, includes, excludes } = form;
  const setLamps = (v: string) => setForm((f) => ({ ...f, lamps: v }));
  const setInterval_ = (v: string) => setForm((f) => ({ ...f, interval: v }));
  const setRealistic = (v: boolean) => setForm((f) => ({ ...f, realistic: v }));
  const setTags = (v: string) => setForm((f) => ({ ...f, tags: v }));
  const setIncludes = (v: string) => setForm((f) => ({ ...f, includes: v }));
  const setExcludes = (v: string) => setForm((f) => ({ ...f, excludes: v }));

  if (!demo) return null;
  const lampN = parseIntIn(lamps, 1, 64);
  const intervalN = parseIntIn(interval, MIN_SEND_INTERVAL_SEC, MAX_SEND_INTERVAL_SEC);
  const valid = lampN !== null && intervalN !== null;

  async function save() {
    if (!valid) return;
    setBusy(true);
    try {
      await api.updateDemoSetup(device.id, {
        tags: parseTagList(tags),
        includes: parseTagList(includes),
        excludes: parseTagList(excludes),
        daliLampCount: lampN,
        sendIntervalSec: intervalN,
        realisticMode: realistic,
        customSlaves: null,
        customItems: null,
        generators: null,
      });
      markSaved();
      toast.success(t("setup.saved"));
      reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <SectionCard
      title={t("setup.settings.title")}
      description={t("setup.settings.description")}
      dirty={dirty}
      onDiscard={discard}
    >
      <div className="grid gap-4 md:grid-cols-2">
        <Field
          label={t("setup.settings.lamps")}
          htmlFor="s-lamps"
          error={lampN === null ? t("setup.settings.lampsInvalid") : null}
        >
          <Input
            id="s-lamps"
            className="font-mono"
            value={lamps}
            onChange={(e) => setLamps(e.target.value)}
          />
        </Field>
        <Field
          label={t("setup.settings.interval")}
          htmlFor="s-interval"
          error={
            intervalN === null
              ? t("setup.settings.intervalInvalid", {
                  min: MIN_SEND_INTERVAL_SEC,
                  max: MAX_SEND_INTERVAL_SEC,
                })
              : null
          }
        >
          <Input
            id="s-interval"
            className="font-mono"
            value={interval}
            onChange={(e) => setInterval_(e.target.value)}
          />
        </Field>
        <Field
          label={t("setup.settings.tags")}
          htmlFor="s-tags"
          hint={t("setup.settings.listHint")}
        >
          <Input
            id="s-tags"
            className="font-mono"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
          />
        </Field>
        <Field
          label={t("setup.settings.includes")}
          htmlFor="s-inc"
          hint={t("setup.settings.listHint")}
        >
          <Input
            id="s-inc"
            className="font-mono"
            value={includes}
            onChange={(e) => setIncludes(e.target.value)}
          />
        </Field>
        <Field
          label={t("setup.settings.excludes")}
          htmlFor="s-exc"
          hint={t("setup.settings.listHint")}
        >
          <Input
            id="s-exc"
            className="font-mono"
            value={excludes}
            onChange={(e) => setExcludes(e.target.value)}
          />
        </Field>
        <div className="flex items-center gap-2 self-end pb-2">
          <Switch id="s-real" checked={realistic} onCheckedChange={setRealistic} />
          <Label htmlFor="s-real" className="font-normal">
            {t("setup.settings.realistic")}
          </Label>
        </div>
      </div>
      <div>
        <Button disabled={busy || !valid} onClick={() => void save()}>
          {t("common.save")}
        </Button>
      </div>
    </SectionCard>
  );
}
