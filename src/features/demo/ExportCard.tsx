import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpenIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, type ExportResult } from "@/lib/api";

import { useDevice } from "../devices/DeviceContext";
import { isValidDemoId, slugify } from "./parse";
import { SectionCard } from "./SectionCard";

/** Export the device as a `library/`-shaped demo pack folder (PRD §13). */
export function ExportCard() {
  const { t } = useTranslation();
  const { device } = useDevice();
  const [demoId, setDemoId] = useState(device.demo?.demoId ?? slugify(device.name));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [result, setResult] = useState<ExportResult | null>(null);

  async function run() {
    setError(null);
    setResult(null);
    setBusy(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: t("export.pickFolder") });
      if (typeof dir !== "string") return;
      setResult(await api.exportDemoPack({ deviceId: device.id, demoId, targetDir: dir }));
    } catch (e) {
      setError(e ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <SectionCard title={t("export.title")} description={t("export.description")}>
      <Field
        label={t("export.demoId")}
        htmlFor="export-id"
        hint={t("export.demoIdHint")}
        className="max-w-sm"
      >
        <Input
          id="export-id"
          className="font-mono"
          value={demoId}
          onChange={(e) => setDemoId(e.target.value)}
        />
      </Field>
      {error !== null && <ErrorNotice error={error} />}
      {result && (
        <div className="flex flex-col gap-2 rounded-md border bg-muted/50 p-4 text-sm">
          <p className="font-semibold text-strong">
            {t("export.done", { count: result.filesWritten.length })}
          </p>
          <p className="font-mono text-xs break-all">{result.path}</p>
          <ul className="max-h-40 overflow-auto font-mono text-xs text-muted-foreground">
            {result.filesWritten.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
          {result.warnings.length > 0 && (
            <ul className="list-disc pl-5 text-muted-foreground">
              {result.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      <div>
        <Button
          variant="outline"
          disabled={busy || !isValidDemoId(demoId)}
          onClick={() => void run()}
        >
          <FolderOpenIcon />
          {t("export.choose")}
        </Button>
      </div>
    </SectionCard>
  );
}
