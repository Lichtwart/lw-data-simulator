import { CopyPlusIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api, type LibraryItem_Serialize, type Severity, type SlaveDefinition } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { loadLibrary, mergeSlaves } from "@/lib/library";
import { useAsync } from "@/lib/useAsync";
import { useSeededForm } from "@/lib/useSeededForm";

import { useDevice } from "../devices/DeviceContext";
import { useReportDirty } from "./dirtyGuard";
import { ItemEditorDialog } from "./ItemEditorDialog";
import { itemToDraft } from "./itemDraft";
import { isValidSlaveKey, isValidSlaveName, parseIntIn } from "./parse";
import { SectionCard } from "./SectionCard";

const SEVERITIES: Severity[] = ["CRITICAL", "MAJOR", "MINOR", "WARNING"];

interface SlaveRow {
  key: string;
  name: string;
  requestInterval: string;
  alarms: SlaveDefinition["alarms"];
}

function toRow(s: SlaveDefinition): SlaveRow {
  return {
    key: s.key,
    name: s.name,
    requestInterval: String(s.requestInterval),
    alarms: s.alarms ?? [],
  };
}

/** Rows → slave definitions, `null` when invalid / keys repeat / key collides with the library. */
function rowsToSlaves(
  rows: readonly SlaveRow[],
  libraryKeys: ReadonlySet<string>,
): SlaveDefinition[] | null {
  const out: SlaveDefinition[] = [];
  for (const r of rows) {
    const interval = parseIntIn(r.requestInterval, 1, 86400);
    if (!isValidSlaveKey(r.key) || !isValidSlaveName(r.name) || interval === null) return null;
    if (libraryKeys.has(r.key) || out.some((s) => s.key === r.key)) return null;
    const alarms = r.alarms ?? [];
    if (alarms.some((a) => !a.name.trim() || !a.text.trim())) return null;
    out.push({ key: r.key, name: r.name, requestInterval: interval, alarms });
  }
  return out;
}

interface Form {
  slaves: SlaveRow[];
  items: LibraryItem_Serialize[];
}

/** Custom slaves and items stored on the device (`lw_demo.customSlaves` / `customItems`, D6). */
export function CustomEditor() {
  const { t } = useTranslation();
  const { device, reload } = useDevice();
  const library = useAsync(() => loadLibrary(), []);
  const demo = device.demo;

  const seed = useMemo<Form>(
    () => ({
      slaves: (demo?.customSlaves ?? []).map(toRow),
      items: demo?.customItems ?? [],
    }),
    [demo],
  );
  const { value: form, setValue: setForm, dirty, discard, markSaved } = useSeededForm(seed);
  useReportDirty("custom", dirty);
  const slaveRows = form.slaves;
  const items = form.items;
  const setSlaveRows = (f: (rs: SlaveRow[]) => SlaveRow[]) =>
    setForm((cur) => ({ ...cur, slaves: f(cur.slaves) }));
  const setItems = (f: (cur: LibraryItem_Serialize[]) => LibraryItem_Serialize[]) =>
    setForm((cur) => ({ ...cur, items: f(cur.items) }));
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<{
    item: LibraryItem_Serialize | null;
    series: boolean;
  } | null>(null);

  const libraryKeys = useMemo(
    () => new Set((library.data?.slaves ?? []).map((s) => s.key)),
    [library.data],
  );
  // Memoised: a fresh array every render would make everything below (and the item dialog) churn.
  const parsedSlaves = useMemo(
    () => rowsToSlaves(slaveRows, libraryKeys),
    [slaveRows, libraryKeys],
  );
  const allSlaves = useMemo(
    () => mergeSlaves(library.data?.slaves ?? [], parsedSlaves ?? []),
    [library.data, parsedSlaves],
  );
  const libraryIds = useMemo(
    () => new Set((library.data?.items ?? []).map((i) => i.id)),
    [library.data],
  );
  const takenIds = useMemo(() => {
    const ids = new Set(libraryIds);
    for (const i of items) if (i.id !== editor?.item?.id) ids.add(i.id);
    return ids;
  }, [libraryIds, items, editor]);

  // Items whose slave no longer exists (e.g. the slave row was deleted) cannot be saved.
  const orphans = useMemo(() => {
    if (!library.data) return [];
    const keys = new Set(allSlaves.map((s) => s.key));
    return items.filter((i) => i.kind === "modbus" && !keys.has(i.slave)).map((i) => i.id);
  }, [items, allSlaves, library.data]);

  if (!demo) return null;

  function patchRow(i: number, p: Partial<SlaveRow>) {
    setSlaveRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  }

  function upsert(next: LibraryItem_Serialize[]) {
    setItems((cur) => {
      const byId = new Map(cur.map((i) => [i.id, i]));
      for (const n of next) byId.set(n.id, n);
      return [...byId.values()];
    });
  }

  async function save() {
    if (!parsedSlaves || orphans.length > 0) return;
    setBusy(true);
    try {
      await api.updateDemoSetup(device.id, {
        tags: null,
        includes: null,
        excludes: null,
        daliLampCount: null,
        sendIntervalSec: null,
        realisticMode: null,
        generators: null,
        customSlaves: parsedSlaves,
        customItems: items,
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
      title={t("setup.custom.title")}
      description={t("setup.custom.description")}
      dirty={dirty}
      onDiscard={discard}
    >
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-bold text-strong">{t("setup.custom.slaves")}</h3>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setSlaveRows((rs) => [
                ...rs,
                { key: "", name: "", requestInterval: "10", alarms: [] },
              ])
            }
          >
            <PlusIcon />
            {t("setup.custom.addSlave")}
          </Button>
        </div>
        {slaveRows.length === 0 && (
          <p className="text-sm text-muted-foreground">{t("setup.custom.noSlaves")}</p>
        )}
        <ul className="flex flex-col gap-3">
          {slaveRows.map((r, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: rows have no stable id while being typed
            <li key={i} className="flex flex-col gap-2 rounded-md border p-3">
              <div className="grid grid-cols-[1fr_1.5fr_6rem_auto] items-center gap-2">
                <Input
                  aria-label={t("setup.custom.slaveKey")}
                  placeholder={t("setup.custom.slaveKey")}
                  className="font-mono"
                  value={r.key}
                  onChange={(e) => patchRow(i, { key: e.target.value })}
                />
                <Input
                  aria-label={t("setup.custom.slaveName")}
                  placeholder={t("setup.custom.slaveName")}
                  maxLength={19}
                  value={r.name}
                  onChange={(e) => patchRow(i, { name: e.target.value })}
                />
                <Input
                  aria-label={t("setup.custom.requestInterval")}
                  placeholder={t("setup.custom.requestInterval")}
                  className="font-mono"
                  value={r.requestInterval}
                  onChange={(e) => patchRow(i, { requestInterval: e.target.value })}
                />
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("common.delete")}
                  onClick={() => setSlaveRows((rs) => rs.filter((_, j) => j !== i))}
                >
                  <Trash2Icon />
                </Button>
              </div>
              {(r.alarms ?? []).map((a, k) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: alarms have no stable id while being typed
                <div key={k} className="grid grid-cols-[1fr_2fr_8rem_auto] items-center gap-2 pl-4">
                  <Input
                    aria-label={t("setup.custom.alarmName")}
                    placeholder={t("setup.custom.alarmName")}
                    className="font-mono"
                    value={a.name}
                    onChange={(e) =>
                      patchRow(i, {
                        alarms: (r.alarms ?? []).map((x, m) =>
                          m === k ? { ...x, name: e.target.value } : x,
                        ),
                      })
                    }
                  />
                  <Input
                    aria-label={t("setup.custom.alarmText")}
                    placeholder={t("setup.custom.alarmText")}
                    value={a.text}
                    onChange={(e) =>
                      patchRow(i, {
                        alarms: (r.alarms ?? []).map((x, m) =>
                          m === k ? { ...x, text: e.target.value } : x,
                        ),
                      })
                    }
                  />
                  <Select
                    value={a.severity}
                    onValueChange={(v) =>
                      patchRow(i, {
                        alarms: (r.alarms ?? []).map((x, m) =>
                          m === k ? { ...x, severity: v as Severity } : x,
                        ),
                      })
                    }
                  >
                    <SelectTrigger aria-label={t("setup.items.severity")}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SEVERITIES.map((s) => (
                        <SelectItem key={s} value={s}>
                          {s}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={t("common.delete")}
                    onClick={() =>
                      patchRow(i, { alarms: (r.alarms ?? []).filter((_, m) => m !== k) })
                    }
                  >
                    <Trash2Icon />
                  </Button>
                </div>
              ))}
              <div className="pl-4">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    patchRow(i, {
                      alarms: [...(r.alarms ?? []), { name: "", text: "", severity: "MAJOR" }],
                    })
                  }
                >
                  <PlusIcon />
                  {t("setup.custom.addAlarm")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
        {slaveRows.length > 0 && !parsedSlaves && (
          <p className="text-xs text-destructive">{t("setup.custom.slavesInvalid")}</p>
        )}
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-sm font-bold text-strong">{t("setup.custom.items")}</h3>
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setEditor({ item: null, series: true })}
            >
              <CopyPlusIcon />
              {t("setup.items.generateSeries")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setEditor({ item: null, series: false })}
            >
              <PlusIcon />
              {t("setup.custom.addItem")}
            </Button>
          </div>
        </div>
        {items.length === 0 && (
          <p className="text-sm text-muted-foreground">{t("setup.custom.noItems")}</p>
        )}
        <ul className="divide-y rounded-md border">
          {items.map((it) => (
            <li key={it.id} className="flex items-center gap-3 px-3 py-2">
              <Badge variant="secondary">{it.kind}</Badge>
              <span className="font-mono text-xs">{it.id}</span>
              <span className="min-w-0 flex-1 truncate text-sm">{it.title.de}</span>
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("devices.edit")}
                title={itemToDraft(it) === null ? t("setup.items.notEditable") : undefined}
                disabled={itemToDraft(it) === null}
                onClick={() => setEditor({ item: it, series: false })}
              >
                <PencilIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("common.delete")}
                onClick={() => setItems((cur) => cur.filter((x) => x.id !== it.id))}
              >
                <Trash2Icon />
              </Button>
            </li>
          ))}
        </ul>
      </div>

      {orphans.length > 0 && (
        <p className="text-xs text-destructive">
          {t("setup.custom.orphans")} <span className="font-mono">{orphans.join(", ")}</span>
        </p>
      )}
      <div>
        <Button disabled={busy || !parsedSlaves || orphans.length > 0} onClick={() => void save()}>
          {t("common.save")}
        </Button>
      </div>

      <ItemEditorDialog
        open={editor !== null}
        onOpenChange={(o) => !o && setEditor(null)}
        initial={editor?.item ?? null}
        series={editor?.series}
        slaves={allSlaves}
        takenIds={takenIds}
        onSubmit={upsert}
      />
    </SectionCard>
  );
}
