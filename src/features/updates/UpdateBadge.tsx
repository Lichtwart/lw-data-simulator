import { relaunch } from "@tauri-apps/plugin-process";
import type { Update } from "@tauri-apps/plugin-updater";
import { DownloadIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Progress } from "@/components/ui/progress";

/** "New version" badge; a click (after confirmation) downloads, installs and restarts. */
export function UpdateBadge({ update }: { update: Update }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);

  async function install() {
    setBusy(true);
    let total = 0;
    let done = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength ?? 0;
        else if (event.event === "Progress") {
          done += event.data.chunkLength;
          setProgress(total > 0 ? (done / total) * 100 : null);
        }
      });
      await relaunch();
    } catch (e) {
      setBusy(false);
      toast.error(t("updates.failed"), { description: e instanceof Error ? e.message : String(e) });
    }
  }

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex w-fit items-center gap-1.5 rounded-full bg-warning-bg px-2.5 py-1 text-xs font-semibold text-strong transition-opacity hover:opacity-90"
      >
        <DownloadIcon className="size-3" />
        {t("nav.updateAvailable")} {update.version}
      </button>
      <ConfirmDialog
        open={open}
        onOpenChange={(o) => !busy && setOpen(o)}
        title={t("updates.title", { version: update.version })}
        description={t("updates.text")}
        confirmLabel={busy ? t("updates.installing") : t("updates.install")}
        busy={busy}
        onConfirm={() => void install()}
      >
        {busy && <Progress value={progress ?? 5} />}
      </ConfirmDialog>
    </>
  );
}
