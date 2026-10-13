import { PlusIcon, Trash2Icon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  api,
  type DigitalInputEntry_Deserialize,
  type DigitalInputEntry_Serialize,
  type DigitalInputType,
} from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { useSeededForm } from "@/lib/useSeededForm";

import { useDevice } from "../devices/DeviceContext";
import { useReportDirty } from "./dirtyGuard";
import { parseIntIn, parseNumberList } from "./parse";
import { SectionCard } from "./SectionCard";

const TYPES: DigitalInputType[] = ["measurement", "impulseCounter", "daliAction"];
const MAX_INPUTS = 10;

let rowCounter = 0;
/** Stable client-side React key for a row (the index is editable, so it cannot be the key). */
const newRowKey = () => `i${rowCounter++}`;

interface Row {
  /** Client-only React key. */
  key: string;
  base: DigitalInputEntry_Serialize;
  index: string;
  title: string;
  type: DigitalInputType;
  isActive: boolean;
  unit: string;
  initCount: string;
  increment: string;
  highLevelAction: string;
  lowLevelAction: string;
  groups: string;
}

function toRow(e: DigitalInputEntry_Serialize): Row {
  return {
    key: newRowKey(),
    base: e,
    index: String(e.index),
    title: e.title,
    type: e.type,
    isActive: e.isActive,
    unit: e.unit ?? "",
    initCount: e.initCount != null ? String(e.initCount) : "",
    increment: e.increment != null ? String(e.increment) : "",
    highLevelAction: e.highLevelAction ?? "",
    lowLevelAction: e.lowLevelAction ?? "",
    groups: (e.groups ?? []).join(", "),
  };
}

function rowsToEntries(rows: readonly Row[]): DigitalInputEntry_Deserialize[] | null {
  const out: DigitalInputEntry_Deserialize[] = [];
  for (const r of rows) {
    const index = parseIntIn(r.index, 1, MAX_INPUTS);
    if (index === null || out.some((e) => e.index === index)) return null;
    const entry: DigitalInputEntry_Deserialize = {
      index,
      title: r.title.trim(),
      type: r.type,
      isActive: r.isActive,
    };
    if (r.type === "impulseCounter") {
      const init = r.initCount.trim() === "" ? null : parseIntIn(r.initCount, 0, 2 ** 31 - 1);
      const inc = r.increment.trim() === "" ? null : parseIntIn(r.increment, 1, 2 ** 31 - 1);
      if (
        (r.initCount.trim() !== "" && init === null) ||
        (r.increment.trim() !== "" && inc === null)
      )
        return null;
      entry.unit = r.unit.trim() === "" ? null : r.unit.trim();
      entry.initCount = init;
      entry.increment = inc;
    }
    if (r.type === "daliAction") {
      const groups = parseNumberList(r.groups, 1, 16);
      if (groups === null) return null;
      entry.groups = groups;
      entry.highLevelAction = r.highLevelAction.trim() === "" ? null : r.highLevelAction.trim();
      entry.lowLevelAction = r.lowLevelAction.trim() === "" ? null : r.lowLevelAction.trim();
    }
    out.push(entry);
  }
  return out;
}

/** Edit the device's digital inputs incl. titles (`lw_demo.digitalInputs`, ≤ 10 inputs; D1, D6, D10). */
export function DigitalInputsEditor({
  inputs,
  onSaved,
}: {
  inputs: DigitalInputEntry_Serialize[];
  onSaved: (inputs: DigitalInputEntry_Serialize[]) => void;
}) {
  const { t } = useTranslation();
  const { device } = useDevice();
  const seed = useMemo(() => inputs.map(toRow), [inputs]);
  const {
    value: rows,
    setValue: setRows,
    dirty,
    discard,
    markSaved,
  } = useSeededForm(seed, (rs) => JSON.stringify(rs.map(({ key: _key, ...r }) => r)));
  useReportDirty("inputs", dirty);
  const [busy, setBusy] = useState(false);

  const parsed = rowsToEntries(rows);

  function patch(i: number, p: Partial<Row>) {
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  }

  function add() {
    const used = new Set(rows.map((r) => Number(r.index)));
    const index =
      Array.from({ length: MAX_INPUTS }, (_, k) => k + 1).find((n) => !used.has(n)) ?? 1;
    setRows((rs) => [...rs, toRow({ index, title: "", type: "measurement", isActive: true })]);
  }

  async function save() {
    if (!parsed) return;
    setBusy(true);
    try {
      const written = await api.setDigitalInputs(device.id, parsed);
      markSaved();
      onSaved(written);
      toast.success(t("setup.saved"));
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <SectionCard
      title={t("setup.inputs.title")}
      description={t("setup.inputs.description")}
      dirty={dirty}
      onDiscard={discard}
      actions={
        <Button size="sm" variant="outline" onClick={add} disabled={rows.length >= MAX_INPUTS}>
          <PlusIcon />
          {t("setup.inputs.add")}
        </Button>
      }
    >
      {rows.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("setup.inputs.empty")}</p>
      )}
      <ul className="flex flex-col gap-3">
        {rows.map((r, i) => (
          <li key={r.key} className="flex flex-col gap-2 rounded-md border p-3">
            <div className="grid grid-cols-[4rem_1fr_12rem_auto_auto] items-center gap-2">
              <Input
                aria-label={t("setup.inputs.index")}
                className="font-mono"
                value={r.index}
                onChange={(e) => patch(i, { index: e.target.value })}
              />
              <Input
                aria-label={t("setup.inputs.titleField")}
                placeholder={t("setup.inputs.titleField")}
                value={r.title}
                onChange={(e) => patch(i, { title: e.target.value })}
              />
              <Select
                value={r.type}
                onValueChange={(v) => patch(i, { type: v as DigitalInputType })}
              >
                <SelectTrigger aria-label={t("setup.inputs.type")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TYPES.map((ty) => (
                    <SelectItem key={ty} value={ty}>
                      {t(`setup.inputs.types.${ty}`)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Switch
                checked={r.isActive}
                aria-label={t("setup.inputs.active")}
                onCheckedChange={(v) => patch(i, { isActive: v })}
              />
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("common.delete")}
                onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}
              >
                <Trash2Icon />
              </Button>
            </div>
            {r.type === "impulseCounter" && (
              <div className="grid grid-cols-3 gap-2">
                <Input
                  aria-label={t("setup.inputs.unit")}
                  placeholder={t("setup.inputs.unit")}
                  value={r.unit}
                  onChange={(e) => patch(i, { unit: e.target.value })}
                />
                <Input
                  aria-label={t("setup.inputs.initCount")}
                  placeholder={t("setup.inputs.initCount")}
                  className="font-mono"
                  value={r.initCount}
                  onChange={(e) => patch(i, { initCount: e.target.value })}
                />
                <Input
                  aria-label={t("setup.inputs.increment")}
                  placeholder={t("setup.inputs.increment")}
                  className="font-mono"
                  value={r.increment}
                  onChange={(e) => patch(i, { increment: e.target.value })}
                />
              </div>
            )}
            {r.type === "daliAction" && (
              <div className="grid grid-cols-3 gap-2">
                <Input
                  aria-label={t("setup.inputs.highAction")}
                  placeholder={t("setup.inputs.highAction")}
                  className="font-mono"
                  value={r.highLevelAction}
                  onChange={(e) => patch(i, { highLevelAction: e.target.value })}
                />
                <Input
                  aria-label={t("setup.inputs.lowAction")}
                  placeholder={t("setup.inputs.lowAction")}
                  className="font-mono"
                  value={r.lowLevelAction}
                  onChange={(e) => patch(i, { lowLevelAction: e.target.value })}
                />
                <Input
                  aria-label={t("setup.inputs.groups")}
                  placeholder={t("setup.inputs.groups")}
                  className="font-mono"
                  value={r.groups}
                  onChange={(e) => patch(i, { groups: e.target.value })}
                />
              </div>
            )}
          </li>
        ))}
      </ul>
      {rows.length > 0 && !parsed && (
        <p className="text-xs text-destructive">{t("setup.inputs.invalid")}</p>
      )}
      <div>
        <Button disabled={busy || !parsed} onClick={() => void save()}>
          {t("common.save")}
        </Button>
      </div>
    </SectionCard>
  );
}
