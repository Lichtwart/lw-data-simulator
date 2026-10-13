import { useTranslation } from "react-i18next";

import { Field } from "@/components/Field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

import { GENERATOR_KINDS, type GeneratorDraft } from "./generatorDraft";

/**
 * The fields of a generator (kind + its parameters). Shared by the generator dialog and the
 * scenario editor's generator steps; `idPrefix` keeps the input ids unique on one page.
 */
export function GeneratorFields({
  draft: d,
  onChange,
  idPrefix,
  allowNone = true,
}: {
  draft: GeneratorDraft;
  onChange: (patch: Partial<GeneratorDraft>) => void;
  idPrefix: string;
  /** Offer "no generator". */
  allowNone?: boolean;
}) {
  const { t } = useTranslation();
  const id = (name: string) => `${idPrefix}-${name}`;
  const kinds = allowNone ? GENERATOR_KINDS : GENERATOR_KINDS.filter((k) => k !== "none");
  return (
    <>
      <Field label={t("generator.kind")} htmlFor={id("kind")}>
        <Select
          value={d.kind}
          onValueChange={(v) => onChange({ kind: v as GeneratorDraft["kind"] })}
        >
          <SelectTrigger id={id("kind")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {kinds.map((k) => (
              <SelectItem key={k} value={k}>
                {k === "none" ? t("generator.none") : t(`control.generators.${k}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      {d.kind === "randomWalk" && (
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("generator.center")} htmlFor={id("center")}>
            <Input
              id={id("center")}
              className="font-mono"
              value={d.center}
              onChange={(e) => onChange({ center: e.target.value })}
            />
          </Field>
          <Field label={t("generator.spread")} htmlFor={id("spread")}>
            <Input
              id={id("spread")}
              className="font-mono"
              value={d.spread}
              onChange={(e) => onChange({ spread: e.target.value })}
            />
          </Field>
          <Field label={t("generator.step")} htmlFor={id("step")}>
            <Input
              id={id("step")}
              className="font-mono"
              value={d.step}
              onChange={(e) => onChange({ step: e.target.value })}
            />
          </Field>
        </div>
      )}
      {d.kind === "sine" && (
        <div className="grid grid-cols-3 gap-3">
          <Field label={t("generator.period")} htmlFor={id("period")}>
            <Input
              id={id("period")}
              className="font-mono"
              value={d.period}
              onChange={(e) => onChange({ period: e.target.value })}
            />
          </Field>
          <Field label={t("generator.amplitude")} htmlFor={id("amp")}>
            <Input
              id={id("amp")}
              className="font-mono"
              value={d.amplitude}
              onChange={(e) => onChange({ amplitude: e.target.value })}
            />
          </Field>
          <Field label={t("generator.centerOptional")} htmlFor={id("center")}>
            <Input
              id={id("center")}
              className="font-mono"
              value={d.center}
              onChange={(e) => onChange({ center: e.target.value })}
            />
          </Field>
        </div>
      )}
      {d.kind === "dailyProfile" && (
        <Field
          label={t("generator.points")}
          htmlFor={id("points")}
          hint={t("generator.pointsHint")}
        >
          <Textarea
            id={id("points")}
            rows={3}
            className="font-mono"
            value={d.points}
            onChange={(e) => onChange({ points: e.target.value })}
          />
        </Field>
      )}
      {d.kind === "counter" && (
        <Field label={t("generator.rate")} htmlFor={id("rate")} className="max-w-xs">
          <Input
            id={id("rate")}
            className="font-mono"
            value={d.ratePerHour}
            onChange={(e) => onChange({ ratePerHour: e.target.value })}
          />
        </Field>
      )}
    </>
  );
}
