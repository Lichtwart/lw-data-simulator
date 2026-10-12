import { PlusIcon, Trash2Icon } from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

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
import { api, type LibraryItem_Serialize, type SlaveDefinition } from "@/lib/api";

import { draftToItem, EMPTY_DRAFT, type ItemDraft, itemToDraft, validateDraft } from "./itemDraft";
import { parseIntIn } from "./parse";

const REGISTER_TYPES = ["holding", "input", "coil"] as const;
const DATATYPES = ["float32", "int16", "uint16", "int32", "uint32"] as const;
const SEVERITIES = ["CRITICAL", "MAJOR", "MINOR", "WARNING"] as const;
const CONTROLS = ["slider", "switch", "select", "counter", "none"] as const;

/**
 * Create / edit a device-local item (Modbus register or device-level custom alarm), or — with
 * `series` — generate a numbered series from it ("generate series" helper, PRD §11.3).
 */
export function ItemEditorDialog({
  open,
  onOpenChange,
  initial,
  series,
  slaves,
  takenIds,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Item being edited; `null` = new. */
  initial: LibraryItem_Serialize | null;
  series?: boolean;
  slaves: readonly SlaveDefinition[];
  /** Ids that are in use (library + other custom items). */
  takenIds: ReadonlySet<string>;
  onSubmit: (items: LibraryItem_Serialize[]) => void;
}) {
  const { t } = useTranslation();
  const [d, setD] = useState<ItemDraft>(EMPTY_DRAFT);
  const [count, setCount] = useState("4");
  const [start, setStart] = useState("1");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const [collisions, setCollisions] = useState<string[]>([]);

  // Initialise the draft only when the dialog opens. `initial` / `slaves` change identity whenever
  // the parent re-renders; resetting on those would wipe what the user is typing.
  const initialRef = useRef(initial);
  initialRef.current = initial;
  const slavesRef = useRef(slaves);
  slavesRef.current = slaves;
  useEffect(() => {
    if (!open) return;
    const init = initialRef.current;
    setD((init && itemToDraft(init)) || { ...EMPTY_DRAFT, slave: slavesRef.current[0]?.key ?? "" });
    setError(null);
    setCollisions([]);
  }, [open]);

  const set = (p: Partial<ItemDraft>) => setD((x) => ({ ...x, ...p }));
  const setOption = (i: number, p: Partial<ItemDraft["options"][number]>) =>
    setD((x) => ({ ...x, options: x.options.map((o, j) => (j === i ? { ...o, ...p } : o)) }));
  const editing = initial !== null && !series;
  const problem = validateDraft(d, {
    takenIds,
    // series ids are numbered by the backend; an edited item keeps its id
    checkId: !editing && !series,
  });
  const countN = parseIntIn(count, 1, 64);
  const startN = parseIntIn(start, 0, 999);
  const seriesInvalid = series && (countN === null || startN === null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (problem || seriesInvalid) return;
    setBusy(true);
    setError(null);
    setCollisions([]);
    try {
      const template = draftToItem(d);
      if (series && countN !== null && startN !== null) {
        const items = await api.generateItemSeries({ template, count: countN, start: startN });
        // Never overwrite existing custom items or shadow library items (D6).
        const clash = items.map((i) => i.id).filter((id) => takenIds.has(id));
        if (clash.length > 0) {
          setCollisions(clash);
          return;
        }
        onSubmit(items);
      } else {
        // `draftToItem` yields the Deserialize shape; defaults are filled by the backend on save.
        // Fields the form does not edit (generator, effects) are kept when editing.
        const kept =
          editing && initial && "generator" in initial
            ? {
                generator: initial.generator,
                ...("effects" in initial ? { effects: initial.effects } : {}),
              }
            : {};
        onSubmit([{ ...template, ...kept } as LibraryItem_Serialize]);
      }
      onOpenChange(false);
    } catch (err) {
      setError(err ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  const isAlarm = d.kind === "customAlarm";

  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {series ? t("setup.items.seriesTitle") : t("setup.items.editorTitle")}
          </DialogTitle>
          <DialogDescription>
            {series ? t("setup.items.seriesHint") : t("setup.items.editorHint")}
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          {!editing && (
            <Field label={t("setup.items.kind")} htmlFor="it-kind">
              <Select value={d.kind} onValueChange={(v) => set({ kind: v as ItemDraft["kind"] })}>
                <SelectTrigger id="it-kind">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="modbus">{t("setup.items.kinds.modbus")}</SelectItem>
                  <SelectItem value="customAlarm">{t("setup.items.kinds.customAlarm")}</SelectItem>
                </SelectContent>
              </Select>
            </Field>
          )}
          <div className="grid grid-cols-2 gap-3">
            <Field
              label={series ? t("setup.items.idPrefix") : t("setup.items.id")}
              htmlFor="it-id"
              hint={t("setup.items.idHint")}
              error={
                !editing && (problem === "id" || problem === "idTaken")
                  ? t(problem === "id" ? "setup.items.idInvalid" : "setup.items.idTaken")
                  : null
              }
            >
              <Input
                id="it-id"
                className="font-mono"
                value={d.id}
                disabled={editing}
                onChange={(e) => set({ id: e.target.value })}
              />
            </Field>
            <Field
              label={t("setup.items.tags")}
              htmlFor="it-tags"
              hint={t("setup.settings.listHint")}
            >
              <Input
                id="it-tags"
                className="font-mono"
                value={d.tags}
                onChange={(e) => set({ tags: e.target.value })}
              />
            </Field>
            <Field label={t("setup.items.titleDe")} htmlFor="it-de">
              <Input
                id="it-de"
                value={d.titleDe}
                onChange={(e) => set({ titleDe: e.target.value })}
              />
            </Field>
            <Field label={t("setup.items.titleEn")} htmlFor="it-en">
              <Input
                id="it-en"
                value={d.titleEn}
                onChange={(e) => set({ titleEn: e.target.value })}
              />
            </Field>
          </div>

          {isAlarm ? (
            <div className="grid grid-cols-2 gap-3">
              <Field label={t("setup.items.severity")} htmlFor="it-sev">
                <Select
                  value={d.severity}
                  onValueChange={(v) => set({ severity: v as ItemDraft["severity"] })}
                >
                  <SelectTrigger id="it-sev">
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
              </Field>
              <Field label={t("setup.items.alarmText")} htmlFor="it-text">
                <Input
                  id="it-text"
                  value={d.text}
                  onChange={(e) => set({ text: e.target.value })}
                />
              </Field>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-3">
                <Field label={t("setup.items.slave")} htmlFor="it-slave">
                  <Select value={d.slave} onValueChange={(v) => set({ slave: v })}>
                    <SelectTrigger id="it-slave">
                      <SelectValue placeholder={t("setup.items.slavePlaceholder")} />
                    </SelectTrigger>
                    <SelectContent>
                      {slaves.map((s) => (
                        <SelectItem key={s.key} value={s.key}>
                          {s.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
                <Field
                  label={t("setup.items.unit")}
                  htmlFor="it-unit"
                  error={problem === "unit" ? t("setup.items.unitInvalid") : null}
                >
                  <Input
                    id="it-unit"
                    value={d.unit}
                    onChange={(e) => set({ unit: e.target.value })}
                  />
                </Field>
                <Field label={t("setup.items.registerType")} htmlFor="it-reg">
                  <Select
                    value={d.registerType}
                    onValueChange={(v) => set({ registerType: v as ItemDraft["registerType"] })}
                  >
                    <SelectTrigger id="it-reg">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {REGISTER_TYPES.map((r) => (
                        <SelectItem key={r} value={r}>
                          {r}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
                <Field label={t("setup.items.datatype")} htmlFor="it-dt">
                  <Select
                    value={d.datatype}
                    onValueChange={(v) => set({ datatype: v as ItemDraft["datatype"] })}
                  >
                    <SelectTrigger id="it-dt">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {DATATYPES.map((r) => (
                        <SelectItem key={r} value={r}>
                          {r}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
              </div>
              <Label className="font-normal">
                <Checkbox
                  checked={d.writable}
                  onCheckedChange={(v) => set({ writable: v === true })}
                />
                {t("setup.items.writable")}
              </Label>
              {problem === "writable" && (
                <p className="text-xs text-destructive">{t("setup.items.writableInvalid")}</p>
              )}
              <Field label={t("setup.items.control")} htmlFor="it-control">
                <Select
                  value={d.controlKind}
                  onValueChange={(v) => set({ controlKind: v as ItemDraft["controlKind"] })}
                >
                  <SelectTrigger id="it-control">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CONTROLS.map((c) => (
                      <SelectItem key={c} value={c}>
                        {t(`setup.items.controls.${c}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              {d.controlKind !== "none" && (
                <div className="grid grid-cols-4 gap-3">
                  {d.controlKind === "slider" && (
                    <>
                      <Field label={t("setup.items.min")} htmlFor="it-min">
                        <Input
                          id="it-min"
                          className="font-mono"
                          value={d.min}
                          onChange={(e) => set({ min: e.target.value })}
                        />
                      </Field>
                      <Field label={t("setup.items.max")} htmlFor="it-max">
                        <Input
                          id="it-max"
                          className="font-mono"
                          value={d.max}
                          onChange={(e) => set({ max: e.target.value })}
                        />
                      </Field>
                      <Field label={t("setup.items.step")} htmlFor="it-step">
                        <Input
                          id="it-step"
                          className="font-mono"
                          value={d.step}
                          onChange={(e) => set({ step: e.target.value })}
                        />
                      </Field>
                    </>
                  )}
                  <Field label={t("setup.items.default")} htmlFor="it-def">
                    <Input
                      id="it-def"
                      className="font-mono"
                      value={d.defaultValue}
                      onChange={(e) => set({ defaultValue: e.target.value })}
                    />
                  </Field>
                </div>
              )}
              {d.controlKind === "select" && (
                <div className="flex flex-col gap-2">
                  <span className="text-sm font-semibold text-strong">
                    {t("setup.items.options")}
                  </span>
                  {d.options.map((o, i) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: options have no stable id while being typed
                    <div key={i} className="grid grid-cols-[5rem_1fr_1fr_auto] items-center gap-2">
                      <Input
                        aria-label={t("setup.items.optionValue")}
                        placeholder={t("setup.items.optionValue")}
                        className="font-mono"
                        value={o.value}
                        onChange={(e) => setOption(i, { value: e.target.value })}
                      />
                      <Input
                        aria-label={t("setup.items.optionDe")}
                        placeholder={t("setup.items.optionDe")}
                        value={o.de}
                        onChange={(e) => setOption(i, { de: e.target.value })}
                      />
                      <Input
                        aria-label={t("setup.items.optionEn")}
                        placeholder={t("setup.items.optionEn")}
                        value={o.en}
                        onChange={(e) => setOption(i, { en: e.target.value })}
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={t("common.delete")}
                        onClick={() => set({ options: d.options.filter((_, j) => j !== i) })}
                      >
                        <Trash2Icon />
                      </Button>
                    </div>
                  ))}
                  <div>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        set({
                          options: [
                            ...d.options,
                            { value: String(d.options.length), de: "", en: "" },
                          ],
                        })
                      }
                    >
                      <PlusIcon />
                      {t("setup.items.addOption")}
                    </Button>
                  </div>
                </div>
              )}
              {problem === "control" && (
                <p className="text-xs text-destructive">{t("setup.items.controlInvalid")}</p>
              )}
            </>
          )}

          {series && (
            <div className="grid grid-cols-2 gap-3 rounded-md border bg-muted/50 p-3">
              <Field label={t("setup.items.count")} htmlFor="it-count">
                <Input
                  id="it-count"
                  className="font-mono"
                  value={count}
                  onChange={(e) => setCount(e.target.value)}
                />
              </Field>
              <Field label={t("setup.items.start")} htmlFor="it-start">
                <Input
                  id="it-start"
                  className="font-mono"
                  value={start}
                  onChange={(e) => setStart(e.target.value)}
                />
              </Field>
            </div>
          )}

          {collisions.length > 0 && (
            <p className="text-xs text-destructive">
              {t("setup.items.idTaken")} <span className="font-mono">{collisions.join(", ")}</span>
            </p>
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
            <Button type="submit" disabled={busy || problem !== null || Boolean(seriesInvalid)}>
              {series ? t("setup.items.generate") : t("common.apply")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
