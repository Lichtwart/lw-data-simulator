import { PlusIcon, Trash2Icon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, type DaliGroup_Deserialize, type DaliGroup_Serialize } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { useSeededForm } from "@/lib/useSeededForm";

import { useDevice } from "../devices/DeviceContext";
import { useReportDirty } from "./dirtyGuard";
import { parseIntIn, parseNumberList } from "./parse";
import { SectionCard } from "./SectionCard";

let rowCounter = 0;
/** Stable client-side React key for a row (ids are editable, so they cannot be the key). */
const newRowKey = () => `g${rowCounter++}`;

interface Row {
  /** Client-only React key. */
  key: string;
  base: DaliGroup_Serialize;
  id: string;
  name: string;
  converters: string;
  minLevel: string;
  maxLevel: string;
}

const MAX_GROUPS = 16;

function toRow(g: DaliGroup_Serialize): Row {
  return {
    key: newRowKey(),
    base: g,
    id: String(g.id),
    name: g.name,
    converters: g.converters.join(", "),
    minLevel: String(g.minLevel),
    maxLevel: String(g.maxLevel),
  };
}

/** Rows → groups; `null` while any row is invalid or ids repeat. */
function rowsToGroups(rows: readonly Row[]): DaliGroup_Deserialize[] | null {
  const out: DaliGroup_Deserialize[] = [];
  for (const r of rows) {
    const id = parseIntIn(r.id, 1, MAX_GROUPS);
    const converters = parseNumberList(r.converters, 1, 64);
    const minLevel = parseIntIn(r.minLevel, 0, 100);
    const maxLevel = parseIntIn(r.maxLevel, 0, 100);
    if (id === null || converters === null || minLevel === null || maxLevel === null) return null;
    if (minLevel > maxLevel || !r.name.trim()) return null;
    if (out.some((g) => g.id === id)) return null;
    out.push({ ...r.base, id, name: r.name.trim(), converters, minLevel, maxLevel });
  }
  return out;
}

/** Edit `lw_configGroups` (≤ 16 groups; D6). */
export function DaliGroupsEditor({
  groups,
  onSaved,
}: {
  groups: DaliGroup_Serialize[];
  onSaved: (groups: DaliGroup_Serialize[]) => void;
}) {
  const { t } = useTranslation();
  const { device } = useDevice();
  const seed = useMemo(() => groups.map(toRow), [groups]);
  const {
    value: rows,
    setValue: setRows,
    dirty,
    discard,
    markSaved,
  } = useSeededForm(seed, (rs) => JSON.stringify(rs.map(({ key: _key, ...r }) => r)));
  useReportDirty("groups", dirty);
  const [busy, setBusy] = useState(false);

  const parsed = rowsToGroups(rows);

  function patch(i: number, p: Partial<Row>) {
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));
  }

  function add() {
    const used = new Set(rows.map((r) => Number(r.id)));
    const id = Array.from({ length: MAX_GROUPS }, (_, k) => k + 1).find((n) => !used.has(n)) ?? 1;
    setRows((rs) => [
      ...rs,
      toRow({
        id,
        name: "",
        converters: [],
        minLevel: 0,
        maxLevel: 100,
        dynamicControl: false,
        automaticSwitch: false,
      }),
    ]);
  }

  async function save() {
    if (!parsed) return;
    setBusy(true);
    try {
      const written = await api.setDaliGroups(device.id, parsed);
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
      title={t("setup.groups.title")}
      description={t("setup.groups.description")}
      dirty={dirty}
      onDiscard={discard}
      actions={
        <Button size="sm" variant="outline" onClick={add} disabled={rows.length >= MAX_GROUPS}>
          <PlusIcon />
          {t("setup.groups.add")}
        </Button>
      }
    >
      {rows.length === 0 && (
        <p className="text-sm text-muted-foreground">{t("setup.groups.empty")}</p>
      )}
      <ul className="flex flex-col gap-2">
        {rows.map((r, i) => (
          <li
            key={r.key}
            className="grid grid-cols-[4rem_1fr_2fr_5rem_5rem_auto] items-center gap-2"
          >
            <Input
              aria-label={t("setup.groups.id")}
              className="font-mono"
              value={r.id}
              onChange={(e) => patch(i, { id: e.target.value })}
            />
            <Input
              aria-label={t("setup.groups.name")}
              placeholder={t("setup.groups.name")}
              value={r.name}
              onChange={(e) => patch(i, { name: e.target.value })}
            />
            <Input
              aria-label={t("setup.groups.converters")}
              placeholder={t("setup.groups.converters")}
              className="font-mono"
              value={r.converters}
              onChange={(e) => patch(i, { converters: e.target.value })}
            />
            <Input
              aria-label={t("setup.groups.min")}
              placeholder={t("setup.groups.min")}
              className="font-mono"
              value={r.minLevel}
              onChange={(e) => patch(i, { minLevel: e.target.value })}
            />
            <Input
              aria-label={t("setup.groups.max")}
              placeholder={t("setup.groups.max")}
              className="font-mono"
              value={r.maxLevel}
              onChange={(e) => patch(i, { maxLevel: e.target.value })}
            />
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("common.delete")}
              onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}
            >
              <Trash2Icon />
            </Button>
          </li>
        ))}
      </ul>
      {rows.length > 0 && !parsed && (
        <p className="text-xs text-destructive">{t("setup.groups.invalid")}</p>
      )}
      <div>
        <Button disabled={busy || !parsed} onClick={() => void save()}>
          {t("common.save")}
        </Button>
      </div>
    </SectionCard>
  );
}
