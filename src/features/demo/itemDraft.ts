import type {
  Control,
  LibraryItem_Deserialize,
  LibraryItem_Serialize,
  ModbusDatatype,
  ModbusRegisterType,
  Severity,
} from "@/lib/api";

import { isValidDeviceAlarmName, isValidItemId, isValidUnit, parseTagList } from "./parse";

/** One row of a select control (value stays text while typing). */
interface OptionDraft {
  value: string;
  de: string;
  en: string;
}

/** Form state of the custom item editor (strings stay strings until validated). */
export interface ItemDraft {
  kind: "modbus" | "customAlarm";
  id: string;
  titleDe: string;
  titleEn: string;
  tags: string;
  slave: string;
  registerType: ModbusRegisterType;
  datatype: ModbusDatatype;
  unit: string;
  writable: boolean;
  controlKind: "none" | "slider" | "switch" | "select" | "counter";
  min: string;
  max: string;
  step: string;
  defaultValue: string;
  /** Options of a select control. */
  options: OptionDraft[];
  severity: Severity;
  text: string;
}

export const EMPTY_DRAFT: ItemDraft = {
  kind: "modbus",
  id: "",
  titleDe: "",
  titleEn: "",
  tags: "",
  slave: "",
  registerType: "holding",
  datatype: "float32",
  unit: "",
  writable: false,
  controlKind: "slider",
  min: "0",
  max: "100",
  step: "1",
  defaultValue: "0",
  options: [
    { value: "0", de: "", en: "" },
    { value: "1", de: "", en: "" },
  ],
  severity: "MAJOR",
  text: "",
};

export type DraftError =
  | "id"
  | "idTaken"
  | "title"
  | "slave"
  | "unit"
  | "control"
  | "text"
  | "writable";

function num(text: string): number | null {
  const s = text.trim().replace(",", ".");
  if (s === "") return null;
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

/** First problem of a draft, or `null` when it can be turned into an item. */
export function validateDraft(
  d: ItemDraft,
  opts: { takenIds: ReadonlySet<string>; checkId?: boolean },
): DraftError | null {
  const checkId = opts.checkId ?? true;
  // An empty id is never valid, not even when editing (it would create a second, nameless item).
  if (!d.id) return "id";
  if (checkId) {
    const valid = d.kind === "customAlarm" ? isValidDeviceAlarmName(d.id) : isValidItemId(d.id);
    if (!valid) return "id";
    if (opts.takenIds.has(d.id)) return "idTaken";
  }
  if (!d.titleDe.trim() || !d.titleEn.trim()) return "title";
  if (d.kind === "customAlarm") return d.text.trim() ? null : "text";
  if (!d.slave) return "slave";
  if (!isValidUnit(d.unit)) return "unit";
  if (d.writable && d.registerType === "input") return "writable";
  if (d.controlKind === "slider") {
    const min = num(d.min);
    const max = num(d.max);
    const step = num(d.step);
    const def = num(d.defaultValue);
    if (min === null || max === null || step === null || def === null) return "control";
    if (min >= max || step <= 0 || def < min || def > max) return "control";
  } else if (d.controlKind === "select") {
    const values = d.options.map((o) => num(o.value));
    const def = num(d.defaultValue);
    if (
      values.length < 2 ||
      values.some((v) => v === null) ||
      new Set(values).size !== values.length
    )
      return "control";
    if (d.options.some((o) => !o.de.trim() || !o.en.trim())) return "control";
    if (def === null || !values.includes(def)) return "control";
  } else if (d.controlKind !== "none" && num(d.defaultValue) === null) return "control";
  return null;
}

/** Draft → library item. Call only after `validateDraft` returned `null`. */
export function draftToItem(d: ItemDraft): LibraryItem_Deserialize {
  const title = { de: d.titleDe.trim(), en: d.titleEn.trim() };
  const tags = parseTagList(d.tags);
  if (d.kind === "customAlarm") {
    return {
      kind: "customAlarm",
      id: d.id,
      title,
      tags,
      severity: d.severity,
      text: d.text.trim(),
    };
  }
  const def = num(d.defaultValue) ?? 0;
  let control: Control | null = null;
  if (d.controlKind === "slider") {
    control = {
      kind: "slider",
      min: num(d.min) ?? 0,
      max: num(d.max) ?? 100,
      step: num(d.step) ?? 1,
      default: def,
    };
  } else if (d.controlKind === "switch") control = { kind: "switch", default: def >= 0.5 ? 1 : 0 };
  else if (d.controlKind === "counter") control = { kind: "counter", default: def };
  else if (d.controlKind === "select") {
    control = {
      kind: "select",
      options: d.options.map((o) => ({
        value: num(o.value) ?? 0,
        label: { de: o.de.trim(), en: o.en.trim() },
      })),
      default: def,
    };
  }
  return {
    kind: "modbus",
    id: d.id,
    slave: d.slave,
    title,
    tags,
    registerType: d.registerType,
    datatype: d.datatype,
    unit: d.unit === "" ? null : d.unit,
    writable: d.writable,
    control,
  };
}

/** Item → draft; `null` for kinds the editor does not handle (the native firmware kinds). */
export function itemToDraft(item: LibraryItem_Serialize): ItemDraft | null {
  const base = {
    ...EMPTY_DRAFT,
    id: item.id,
    titleDe: item.title.de,
    titleEn: item.title.en,
    tags: item.tags.join(", "),
  };
  if (item.kind === "customAlarm") {
    return { ...base, kind: "customAlarm", severity: item.severity, text: item.text };
  }
  if (item.kind !== "modbus") return null;
  const c = item.control;
  return {
    ...base,
    kind: "modbus",
    slave: item.slave,
    registerType: item.registerType ?? (c?.kind === "switch" ? "coil" : "holding"),
    datatype: item.datatype ?? "float32",
    unit: item.unit ?? "",
    writable: item.writable,
    controlKind: c ? c.kind : "none",
    min: c?.kind === "slider" ? String(c.min) : "0",
    max: c?.kind === "slider" ? String(c.max) : "100",
    step: c?.kind === "slider" ? String(c.step) : "1",
    defaultValue: c ? String(c.default) : "0",
    options:
      c?.kind === "select"
        ? c.options.map((o) => ({ value: String(o.value), de: o.label.de, en: o.label.en }))
        : EMPTY_DRAFT.options,
  };
}
