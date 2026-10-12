import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { type ApplyDemoPreview, api } from "@/lib/api";
import { deviceLockOf } from "@/lib/errors";
import { formatSince } from "@/lib/format";
import { loadLibrary } from "@/lib/library";
import { useAsync } from "@/lib/useAsync";

import { useDevice } from "../devices/DeviceContext";
import { SectionCard } from "./SectionCard";

/** Apply a demo pack: choose, preview (`preview_apply_demo`), confirm floorplan replacement, apply. */
export function ApplyDemoCard() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language === "en" ? "en" : "de";
  const { device, reload, reloadConfig } = useDevice();
  const library = useAsync(() => loadLibrary(), []);
  const [demoId, setDemoId] = useState(device.demo?.demoId ?? "");
  const [applyFloorplans, setApplyFloorplans] = useState(true);
  const [preview, setPreview] = useState<ApplyDemoPreview | null>(null);
  const [replace, setReplace] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [lock, setLock] = useState<{ user: string; since: string } | null>(null);

  // Only the newest preview request may set the result (a slow older one must not enable "Apply").
  const previewToken = useRef(0);

  const demos = library.data?.demos ?? [];
  const conflicts = applyFloorplans ? (preview?.floorplanConflicts ?? []) : [];
  const needsReplace = conflicts.length > 0;

  async function runPreview() {
    const token = ++previewToken.current;
    setBusy(true);
    setError(null);
    setReplace(false);
    try {
      const result = await api.previewApplyDemo(device.id, {
        demoId,
        applyFloorplans,
        replaceExistingFloorplans: false,
      });
      if (token === previewToken.current) setPreview(result);
    } catch (e) {
      if (token !== previewToken.current) return;
      setPreview(null);
      setError(e ?? new Error("unknown"));
    } finally {
      if (token === previewToken.current) setBusy(false);
    }
  }

  async function apply(force = false) {
    setBusy(true);
    setError(null);
    try {
      const result = await api.applyDemo(device.id, {
        demoId,
        applyFloorplans,
        replaceExistingFloorplans: replace,
        force,
      });
      toast.success(
        t("apply.done", {
          created: result.floorplansCreated.length,
          replaced: result.floorplansReplaced.length,
        }),
      );
      for (const w of result.warnings) toast.warning(w);
      setPreview(null);
      setLock(null);
      reload();
      reloadConfig();
    } catch (e) {
      const held = deviceLockOf(e);
      if (held) {
        setLock(held);
      } else {
        setLock(null);
        setError(e ?? new Error("unknown"));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <SectionCard title={t("apply.title")} description={t("apply.description")}>
      <div className="grid gap-4 md:grid-cols-2">
        <Field label={t("apply.demo")} htmlFor="apply-demo">
          <Select
            value={demoId}
            onValueChange={(v) => {
              previewToken.current++;
              setBusy(false);
              setDemoId(v);
              setPreview(null);
            }}
          >
            <SelectTrigger id="apply-demo">
              <SelectValue placeholder={t("apply.choose")} />
            </SelectTrigger>
            <SelectContent>
              {demos.map((d) => (
                <SelectItem key={d.id} value={d.id}>
                  {d.name[lang]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <div className="flex items-end">
          <Label className="font-normal">
            <Checkbox
              checked={applyFloorplans}
              onCheckedChange={(v) => {
                previewToken.current++;
                setBusy(false);
                setApplyFloorplans(v === true);
                setPreview(null);
              }}
            />
            {t("apply.floorplans")}
          </Label>
        </div>
      </div>

      {error !== null && <ErrorNotice error={error} />}

      {preview && (
        <div className="flex flex-col gap-3 rounded-md border bg-muted/50 p-4 text-sm">
          <p className="font-semibold text-strong">
            {t("apply.previewItems", { count: preview.itemCount })}
          </p>
          {preview.slaves.length > 0 && (
            <div>
              <h4 className="mb-1 text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
                {t("apply.slaves")}
              </h4>
              <ul className="font-mono text-xs">
                {preview.slaves.map((s) => (
                  <li key={s.key}>
                    {t("apply.slaveLine", { key: s.key, id: s.id, address: s.address })}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {needsReplace && (
            <div className="flex flex-col gap-2 rounded-md border border-warning bg-warning-bg p-3">
              <p className="font-semibold text-strong">{t("apply.conflicts")}</p>
              <ul className="list-disc pl-5">
                {conflicts.map((f) => (
                  <li key={f.id}>{f.name}</li>
                ))}
              </ul>
              <Label className="font-normal">
                <Checkbox checked={replace} onCheckedChange={(v) => setReplace(v === true)} />
                {t("apply.replace")}
              </Label>
            </div>
          )}
          {preview.warnings.length > 0 && (
            <ul className="list-disc pl-5 text-muted-foreground">
              {preview.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
          <p className="text-xs text-muted-foreground">{t("apply.stopsSession")}</p>
        </div>
      )}

      <div className="flex gap-2">
        <Button variant="outline" disabled={busy || !demoId} onClick={() => void runPreview()}>
          {t("apply.preview")}
        </Button>
        <Button
          disabled={busy || !preview || (needsReplace && !replace)}
          onClick={() => void apply()}
        >
          {t("apply.apply")}
        </Button>
      </div>
      <ConfirmDialog
        open={lock !== null}
        onOpenChange={(open) => !open && setLock(null)}
        title={t("session.takeOverTitle")}
        description={t("apply.takeOverText", {
          user: lock?.user ?? "",
          since: lock ? formatSince(lock.since, new Date(), i18n.language) : "",
        })}
        confirmLabel={t("apply.takeOver")}
        busy={busy}
        onConfirm={() => void apply(true)}
      />
    </SectionCard>
  );
}
