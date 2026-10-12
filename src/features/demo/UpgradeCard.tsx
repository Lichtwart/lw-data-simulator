import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { api, type UpgradePreview } from "@/lib/api";
import { deviceLockOf, errorMessage } from "@/lib/errors";
import { formatSince } from "@/lib/format";
import { useAsync } from "@/lib/useAsync";

import { useDevice } from "../devices/DeviceContext";
import { SectionCard } from "./SectionCard";

function IdList({ title, ids }: { title: string; ids: readonly string[] }) {
  if (ids.length === 0) return null;
  return (
    <div>
      <h4 className="mb-1 text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
        {title} ({ids.length})
      </h4>
      <p className="font-mono text-xs break-words">{ids.join(", ")}</p>
    </div>
  );
}

/** Things the upgrade skips or keeps: a short explanation plus the affected names. */
function Note({
  tone,
  title,
  hint,
  ids,
}: {
  tone: "info" | "warning";
  title: string;
  hint: string;
  ids: readonly string[];
}) {
  if (ids.length === 0) return null;
  return (
    <div
      className={
        tone === "warning"
          ? "flex flex-col gap-1 rounded-md border border-warning bg-warning-bg p-3"
          : "flex flex-col gap-1 rounded-md border bg-muted/50 p-3"
      }
    >
      <h4 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
        {title} ({ids.length})
      </h4>
      <p className="text-xs text-muted-foreground">{hint}</p>
      <p className="font-mono text-xs break-words">{ids.join(", ")}</p>
    </div>
  );
}

/** Library upgrade: preview of added / removed / changed items + floorplans, then apply. Never silent. */
export function UpgradeCard() {
  const { t, i18n } = useTranslation();
  const { device, setDevice, reload, reloadConfig } = useDevice();
  const check = useAsync<UpgradePreview | null>(
    () => api.checkUpgrade(device.id),
    [device.id, device.demo?.libraryVersion],
  );
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [lock, setLock] = useState<{ user: string; since: string } | null>(null);
  const preview = check.data;

  async function apply(force = false) {
    setBusy(true);
    try {
      setDevice(await api.applyUpgrade(device.id, force));
      toast.success(t("upgrade.done"));
      reload();
      reloadConfig();
      check.reload();
      setLock(null);
    } catch (e) {
      const held = deviceLockOf(e);
      if (held) setLock(held);
      else toast.error(errorMessage(e));
    } finally {
      setBusy(false);
      setConfirm(false);
    }
  }

  if (!device.demo?.demoId) return null;
  if (check.error !== null) {
    return (
      <SectionCard title={t("upgrade.title")}>
        <ErrorNotice error={check.error} />
      </SectionCard>
    );
  }
  if (!preview) {
    return (
      <SectionCard title={t("upgrade.title")}>
        <p className="text-sm text-muted-foreground">
          {check.loading ? t("common.loading") : t("upgrade.upToDate")}
        </p>
      </SectionCard>
    );
  }

  return (
    <SectionCard
      title={t("upgrade.title")}
      description={t("upgrade.versions", { from: preview.fromVersion, to: preview.toVersion })}
      actions={<Button onClick={() => setConfirm(true)}>{t("upgrade.apply")}</Button>}
    >
      <div className="flex flex-col gap-3 text-sm">
        <IdList title={t("upgrade.itemsAdded")} ids={preview.itemsAdded} />
        <IdList title={t("upgrade.itemsRemoved")} ids={preview.itemsRemoved} />
        <IdList title={t("upgrade.itemsChanged")} ids={preview.itemsChanged} />
        <IdList title={t("upgrade.floorplansAdded")} ids={preview.floorplansAdded} />
        <IdList title={t("upgrade.floorplansRemoved")} ids={preview.floorplansRemoved} />
        <IdList title={t("upgrade.floorplansChanged")} ids={preview.floorplansChanged} />
        <IdList title={t("upgrade.demoChanges")} ids={preview.demoChanges} />
        <IdList title={t("upgrade.customNowInLibrary")} ids={preview.customNowInLibrary} />
        <Note
          tone="info"
          title={t("upgrade.keptOverrides")}
          hint={t("upgrade.keptOverridesHint")}
          ids={preview.keptOverrides}
        />
        <Note
          tone="warning"
          title={t("upgrade.floorplanConflicts")}
          hint={t("upgrade.floorplanConflictsHint")}
          ids={preview.floorplanConflicts.map((f) => f.name)}
        />
        <Note
          tone="info"
          title={t("upgrade.floorplansEdited")}
          hint={t("upgrade.floorplansEditedHint", { version: preview.toVersion })}
          ids={preview.floorplansEdited}
        />
      </div>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t("upgrade.confirmTitle")}
        description={t("upgrade.confirmText", { to: preview.toVersion })}
        confirmLabel={t("upgrade.apply")}
        busy={busy}
        onConfirm={() => void apply()}
      />
      <ConfirmDialog
        open={lock !== null}
        onOpenChange={(open) => !open && setLock(null)}
        title={t("session.takeOverTitle")}
        description={t("upgrade.takeOverText", {
          user: lock?.user ?? "",
          since: lock ? formatSince(lock.since, new Date(), i18n.language) : "",
        })}
        confirmLabel={t("upgrade.takeOver")}
        busy={busy}
        onConfirm={() => void apply(true)}
      />
    </SectionCard>
  );
}
