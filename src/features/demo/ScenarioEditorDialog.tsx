import { ArrowDownIcon, ArrowUpIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
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
import type { FaultDefinition, LibraryItem_Serialize, Scenario_Serialize } from "@/lib/api";

import { faultKey } from "../control/faults";
import { GeneratorFields } from "../control/GeneratorFields";
import type { Lang } from "../control/itemFilter";
import {
  canTakeGenerator,
  controlOf,
  draftToScenario,
  EMPTY_SCENARIO,
  hasValue,
  newStep,
  type ScenarioDraft,
  type StepDraft,
  scenarioIdFor,
  scenarioToDraft,
  stepProblem,
  targetKey,
  validateScenarioDraft,
} from "./scenarioDraft";

/**
 * Create / edit one scenario of the device: names, description, start flag and the planned steps
 * (set values, set generators, raise or clear alarms). Steps run in order when applied.
 */
export function ScenarioEditorDialog({
  open,
  onOpenChange,
  initial,
  items,
  faults,
  takenIds,
  lang,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Scenario being edited; `null` = new. */
  initial: Scenario_Serialize | null;
  /** Items of the device (selected library items + custom items). */
  items: readonly LibraryItem_Serialize[];
  /** Fault catalog of the device. */
  faults: readonly FaultDefinition[];
  /** Ids used by the other scenarios. */
  takenIds: ReadonlySet<string>;
  lang: Lang;
  onSubmit: (scenario: Scenario_Serialize) => void;
}) {
  const { t } = useTranslation();
  const [d, setD] = useState<ScenarioDraft>(EMPTY_SCENARIO);

  // Seed only when the dialog opens (the parent re-renders while the user types).
  const initialRef = useRef(initial);
  initialRef.current = initial;
  useEffect(() => {
    if (!open) return;
    const init = initialRef.current;
    setD(init ? scenarioToDraft(init) : EMPTY_SCENARIO);
  }, [open]);

  const byId = new Map(items.map((i) => [i.id, i]));
  const valueItems = items.filter(hasValue);
  const generatorItems = items.filter(canTakeGenerator);
  const problem = validateScenarioDraft(d, byId);

  const set = (p: Partial<ScenarioDraft>) => setD((x) => ({ ...x, ...p }));
  const setStep = (i: number, s: StepDraft) =>
    setD((x) => ({ ...x, steps: x.steps.map((o, j) => (j === i ? s : o)) }));
  const removeStep = (i: number) =>
    setD((x) => ({ ...x, steps: x.steps.filter((_, j) => j !== i) }));
  const moveStep = (i: number, by: -1 | 1) =>
    setD((x) => {
      const j = i + by;
      const a = x.steps[i];
      const b = x.steps[j];
      if (!a || !b) return x;
      const steps = x.steps.map((s, k) => (k === i ? b : k === j ? a : s));
      return { ...x, steps };
    });
  const addStep = (kind: StepDraft["kind"]) =>
    setD((x) => ({
      ...x,
      steps: [...x.steps, newStep(kind, kind === "generator" ? generatorItems[0] : valueItems[0])],
    }));

  function submit(e: FormEvent) {
    e.preventDefault();
    if (problem) return;
    const id = initial?.id ?? scenarioIdFor(d.nameEn || d.nameDe, takenIds);
    onSubmit(draftToScenario(d, id));
    onOpenChange(false);
  }

  const itemLabel = (i: LibraryItem_Serialize) => i.title[lang];
  const problemText =
    problem?.kind === "name"
      ? t("setup.scenarios.problems.name")
      : problem?.kind === "description"
        ? t("setup.scenarios.problems.description")
        : problem?.kind === "noSteps"
          ? t("setup.scenarios.problems.noSteps")
          : null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>
            {initial ? t("setup.scenarios.editTitle") : t("setup.scenarios.newTitle")}
          </DialogTitle>
          <DialogDescription>{t("setup.scenarios.editorHint")}</DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={submit}>
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("setup.scenarios.nameDe")} htmlFor="sc-name-de">
              <Input
                id="sc-name-de"
                value={d.nameDe}
                onChange={(e) => set({ nameDe: e.target.value })}
              />
            </Field>
            <Field label={t("setup.scenarios.nameEn")} htmlFor="sc-name-en">
              <Input
                id="sc-name-en"
                value={d.nameEn}
                onChange={(e) => set({ nameEn: e.target.value })}
              />
            </Field>
            <Field label={t("setup.scenarios.descriptionDe")} htmlFor="sc-desc-de">
              <Textarea
                id="sc-desc-de"
                rows={2}
                value={d.descriptionDe}
                onChange={(e) => set({ descriptionDe: e.target.value })}
              />
            </Field>
            <Field label={t("setup.scenarios.descriptionEn")} htmlFor="sc-desc-en">
              <Textarea
                id="sc-desc-en"
                rows={2}
                value={d.descriptionEn}
                onChange={(e) => set({ descriptionEn: e.target.value })}
              />
            </Field>
          </div>
          <div className="flex flex-col gap-1">
            <Label className="font-normal">
              <Checkbox checked={d.start} onCheckedChange={(v) => set({ start: v === true })} />
              {t("setup.scenarios.startLabel")}
            </Label>
            <p className="text-xs text-muted-foreground">{t("setup.scenarios.startHint")}</p>
          </div>
          {initial && (
            <p className="text-xs text-muted-foreground">
              {t("setup.scenarios.id")} <span className="font-mono">{initial.id}</span>
            </p>
          )}

          <div className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="text-sm font-bold text-strong">{t("setup.scenarios.steps")}</h3>
              <div className="flex flex-wrap gap-2">
                <Button type="button" size="sm" variant="outline" onClick={() => addStep("value")}>
                  <PlusIcon />
                  {t("setup.scenarios.addValue")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={generatorItems.length === 0}
                  onClick={() => addStep("generator")}
                >
                  <PlusIcon />
                  {t("setup.scenarios.addGenerator")}
                </Button>
                <Button type="button" size="sm" variant="outline" onClick={() => addStep("fault")}>
                  <PlusIcon />
                  {t("setup.scenarios.addFault")}
                </Button>
              </div>
            </div>
            {d.steps.length === 0 && (
              <p className="text-sm text-muted-foreground">{t("setup.scenarios.noSteps")}</p>
            )}
            <ol className="flex flex-col gap-2">
              {d.steps.map((step, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: steps have no stable id while being edited
                <li key={i} className="flex flex-col gap-2 rounded-md border p-3">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs text-muted-foreground">{i + 1}</span>
                    <Badge variant="secondary">{t(`setup.scenarios.kinds.${step.kind}`)}</Badge>
                    <span className="flex-1" />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label={t("setup.scenarios.moveUp")}
                      disabled={i === 0}
                      onClick={() => moveStep(i, -1)}
                    >
                      <ArrowUpIcon />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label={t("setup.scenarios.moveDown")}
                      disabled={i === d.steps.length - 1}
                      onClick={() => moveStep(i, 1)}
                    >
                      <ArrowDownIcon />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label={t("common.delete")}
                      onClick={() => removeStep(i)}
                    >
                      <Trash2Icon />
                    </Button>
                  </div>
                  <StepFields
                    index={i}
                    step={step}
                    onChange={(s) => setStep(i, s)}
                    byId={byId}
                    choices={step.kind === "generator" ? generatorItems : valueItems}
                    faults={faults}
                    itemLabel={itemLabel}
                    lang={lang}
                  />
                  {stepProblem(step, byId) && (
                    <p className="text-xs text-destructive">
                      {t(`setup.scenarios.problems.${stepProblem(step, byId) ?? "item"}`)}
                    </p>
                  )}
                </li>
              ))}
            </ol>
          </div>

          {problemText && <p className="text-xs text-destructive">{problemText}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={problem !== null}>
              {t("common.apply")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** The fields of one step (item + value / generator, or alarm + raise / clear). */
function StepFields({
  index,
  step,
  onChange,
  byId,
  choices,
  faults,
  itemLabel,
  lang,
}: {
  index: number;
  step: StepDraft;
  onChange: (s: StepDraft) => void;
  byId: ReadonlyMap<string, LibraryItem_Serialize>;
  choices: readonly LibraryItem_Serialize[];
  faults: readonly FaultDefinition[];
  itemLabel: (i: LibraryItem_Serialize) => string;
  lang: Lang;
}) {
  const { t } = useTranslation();
  const id = (name: string) => `sc-step-${index}-${name}`;

  if (step.kind === "fault") {
    const key = targetKey(step.target);
    const known = faults.some((f) => faultKey(f.target) === key);
    return (
      <div className="grid grid-cols-[1fr_10rem] gap-3">
        <Field label={t("setup.scenarios.alarm")} htmlFor={id("fault")}>
          <Select
            value={key}
            onValueChange={(v) => {
              const f = faults.find((x) => faultKey(x.target) === v);
              if (f) onChange({ ...step, target: f.target });
            }}
          >
            <SelectTrigger id={id("fault")}>
              <SelectValue placeholder={t("setup.scenarios.alarmPlaceholder")} />
            </SelectTrigger>
            <SelectContent>
              {!known && key && (
                <SelectItem value={key}>
                  <span className="font-mono text-xs">{key}</span>
                </SelectItem>
              )}
              {faults.map((f) => (
                <SelectItem key={faultKey(f.target)} value={faultKey(f.target)}>
                  {f.text}{" "}
                  <span className="font-mono text-xs text-muted-foreground">{f.alarmType}</span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label={t("setup.scenarios.action")} htmlFor={id("active")}>
          <Select
            value={step.active ? "raise" : "clear"}
            onValueChange={(v) => onChange({ ...step, active: v === "raise" })}
          >
            <SelectTrigger id={id("active")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="raise">{t("setup.scenarios.raise")}</SelectItem>
              <SelectItem value="clear">{t("setup.scenarios.clear")}</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>
    );
  }

  const item = byId.get(step.itemId);
  const itemSelect = (
    <Field label={t("setup.scenarios.item")} htmlFor={id("item")}>
      <Select
        value={step.itemId}
        onValueChange={(v) => {
          const next = byId.get(v);
          // A new item starts from its own default / generator.
          onChange(next ? { ...newStep(step.kind, next) } : step);
        }}
      >
        <SelectTrigger id={id("item")}>
          <SelectValue placeholder={t("setup.scenarios.itemPlaceholder")} />
        </SelectTrigger>
        <SelectContent>
          {!item && step.itemId && (
            <SelectItem value={step.itemId}>
              <span className="font-mono text-xs">{step.itemId}</span>
            </SelectItem>
          )}
          {choices.map((i) => (
            <SelectItem key={i.id} value={i.id}>
              {itemLabel(i)} <span className="font-mono text-xs text-muted-foreground">{i.id}</span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );

  if (step.kind === "generator") {
    return (
      <div className="flex flex-col gap-3">
        {itemSelect}
        <GeneratorFields
          draft={step.generator}
          onChange={(p) => onChange({ ...step, generator: { ...step.generator, ...p } })}
          idPrefix={id("gen")}
          allowNone={false}
        />
      </div>
    );
  }

  const control = item ? controlOf(item) : null;
  const unit = item && "unit" in item ? (item.unit ?? null) : null;
  let valueField: ReactNode;
  if (control?.kind === "switch" || control?.kind === "select") {
    const options =
      control.kind === "switch"
        ? [
            { value: 0, label: t("control.modes.off") },
            { value: 1, label: t("control.modes.on") },
          ]
        : control.options.map((o) => ({ value: o.value, label: o.label[lang] }));
    valueField = (
      <Select value={step.value} onValueChange={(v) => onChange({ ...step, value: v })}>
        <SelectTrigger id={id("value")}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.value} value={String(o.value)}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  } else {
    valueField = (
      <Input
        id={id("value")}
        className="font-mono"
        value={step.value}
        onChange={(e) => onChange({ ...step, value: e.target.value })}
      />
    );
  }
  const hint =
    control?.kind === "slider"
      ? t("setup.scenarios.range", { min: control.min, max: control.max, unit: unit ?? "" })
      : control?.kind === "counter"
        ? t("setup.scenarios.counterMin", { min: control.default })
        : undefined;
  return (
    <div className="grid grid-cols-[1fr_12rem] gap-3">
      {itemSelect}
      <Field label={t("setup.scenarios.value")} htmlFor={id("value")} hint={hint}>
        {valueField}
      </Field>
    </div>
  );
}
