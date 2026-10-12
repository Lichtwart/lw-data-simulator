import { type FormEvent, lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { api, type DemoDevice_Serialize, type DemoPack_Serialize, type GroupRef } from "@/lib/api";
import { formatCoordinate, parsePosition } from "@/lib/format";
import { loadLibrary } from "@/lib/library";
import { useAsync } from "@/lib/useAsync";

// Leaflet is heavy: load it only when the dialog opens.
const MapPicker = lazy(() => import("./MapPicker").then((m) => ({ default: m.MapPicker })));

const NONE = "__none__";

/** Fields the dialog edits; `device` set = edit mode (name, description, position only). */
export interface DeviceFormValues {
  id: string;
  name: string;
  description: string | null;
  position: { lat: number; lng: number } | null;
}

/** Create (PRD §6.2: name, description, position, parent group, demo pack) or edit a demo device. */
export function DeviceFormDialog({
  open,
  onOpenChange,
  device,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  device?: DeviceFormValues | null;
  onSaved: (device: DemoDevice_Serialize) => void;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.language === "en" ? "en" : "de";
  const editing = Boolean(device);

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [lat, setLat] = useState("");
  const [lng, setLng] = useState("");
  const [groupId, setGroupId] = useState(NONE);
  const [demoId, setDemoId] = useState(NONE);
  const [applyFloorplans, setApplyFloorplans] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const groups = useAsync<GroupRef[]>(
    () => (open && !editing ? api.listDeviceGroups() : Promise.resolve([])),
    [open, editing],
  );
  const library = useAsync(() => loadLibrary(), []);
  const demos: DemoPack_Serialize[] = library.data?.demos ?? [];

  // Reset the form only when the dialog opens, not when the parent passes a new `device` object
  // (e.g. on every session tick) - that would wipe what the user is typing.
  const deviceRef = useRef(device);
  deviceRef.current = device;
  useEffect(() => {
    if (!open) return;
    const device = deviceRef.current;
    setName(device?.name ?? "");
    setDescription(device?.description ?? "");
    setLat(device?.position ? formatCoordinate(device.position.lat) : "");
    setLng(device?.position ? formatCoordinate(device.position.lng) : "");
    setGroupId(NONE);
    setDemoId(NONE);
    setApplyFloorplans(true);
    setError(null);
  }, [open]);

  const parsed = useMemo(() => parsePosition(lat, lng), [lat, lng]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim() || parsed.error) return;
    setBusy(true);
    setError(null);
    try {
      const desc = description.trim() === "" ? null : description.trim();
      let saved: DemoDevice_Serialize;
      if (device) {
        saved = await api.updateDemoDevice(device.id, {
          name: name.trim(),
          description: desc,
          position: parsed.position,
        });
      } else {
        const created = await api.createDemoDevice({
          name: name.trim(),
          description: desc,
          position: parsed.position,
          parentGroupId: groupId === NONE ? null : groupId,
          applyDemo:
            demoId === NONE ? null : { demoId, applyFloorplans, replaceExistingFloorplans: false },
        });
        saved = created.device;
        for (const w of created.warnings) toast.warning(w);
      }
      toast.success(editing ? t("devices.saved") : t("devices.created"));
      onSaved(saved);
      onOpenChange(false);
    } catch (err) {
      setError(err ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  const coordError =
    parsed.error === "latitude"
      ? t("devices.form.latitudeInvalid")
      : parsed.error === "longitude"
        ? t("devices.form.longitudeInvalid")
        : null;

  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {editing ? t("devices.form.editTitle") : t("devices.form.createTitle")}
          </DialogTitle>
          <DialogDescription>{t("devices.form.hint")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          <Field label={t("devices.form.name")} htmlFor="dev-name">
            <Input id="dev-name" value={name} onChange={(e) => setName(e.target.value)} required />
          </Field>
          <Field label={t("devices.form.description")} htmlFor="dev-desc">
            <Textarea
              id="dev-desc"
              rows={2}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={t("devices.form.latitude")}
              htmlFor="dev-lat"
              error={parsed.error === "latitude" ? coordError : null}
            >
              <Input
                id="dev-lat"
                inputMode="decimal"
                className="font-mono"
                value={lat}
                onChange={(e) => setLat(e.target.value)}
                aria-invalid={parsed.error === "latitude"}
              />
            </Field>
            <Field
              label={t("devices.form.longitude")}
              htmlFor="dev-lng"
              error={parsed.error === "longitude" ? coordError : null}
            >
              <Input
                id="dev-lng"
                inputMode="decimal"
                className="font-mono"
                value={lng}
                onChange={(e) => setLng(e.target.value)}
                aria-invalid={parsed.error === "longitude"}
              />
            </Field>
          </div>
          <div className="flex flex-col gap-1.5">
            <Suspense fallback={<div className="h-56 w-full rounded-md border bg-muted" />}>
              <MapPicker
                label={t("devices.form.map")}
                value={parsed.position}
                onChange={(p) => {
                  setLat(formatCoordinate(p.lat));
                  setLng(formatCoordinate(p.lng));
                }}
              />
            </Suspense>
            <p className="text-xs text-muted-foreground">{t("devices.form.mapHint")}</p>
          </div>
          {!editing && (
            <>
              {groups.error !== null && <ErrorNotice error={groups.error} />}
              <Field label={t("devices.form.parentGroup")} htmlFor="dev-group">
                <Select value={groupId} onValueChange={setGroupId}>
                  <SelectTrigger id="dev-group">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE}>{t("devices.ungrouped")}</SelectItem>
                    {(groups.data ?? []).map((g) => (
                      <SelectItem key={g.id} value={g.id}>
                        {g.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field label={t("devices.form.demo")} htmlFor="dev-demo">
                <Select value={demoId} onValueChange={setDemoId}>
                  <SelectTrigger id="dev-demo">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE}>{t("devices.form.noDemo")}</SelectItem>
                    {demos.map((d) => (
                      <SelectItem key={d.id} value={d.id}>
                        {d.name[lang]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              {demoId !== NONE && (
                <Label className="font-normal">
                  <Checkbox
                    checked={applyFloorplans}
                    onCheckedChange={(v) => setApplyFloorplans(v === true)}
                  />
                  {t("apply.floorplans")}
                </Label>
              )}
            </>
          )}
          {error !== null && <ErrorNotice error={error} />}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={busy || !name.trim() || parsed.error !== null}>
              {editing ? t("common.save") : t("devices.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
