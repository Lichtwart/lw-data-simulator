import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import type { Control } from "@/lib/api";
import { formatNumber, formatValue, stepDecimals } from "@/lib/format";

import type { Lang } from "./itemFilter";

interface Props {
  control: Control;
  value: number;
  unit?: string | null;
  label: string;
  lang: Lang;
  disabled?: boolean;
  /** May return a promise resolving `false` when the backend rejected the value (so previews can be reverted). */
  onSet: (value: number) => unknown;
}

/** The item's control: slider / switch / select / counter (PRD §8). */
export function ItemControl({ control, value, unit, label, lang, disabled, onSet }: Props) {
  switch (control.kind) {
    case "slider":
      return (
        <SliderControl
          control={control}
          value={value}
          unit={unit}
          label={label}
          lang={lang}
          disabled={disabled}
          onSet={onSet}
        />
      );
    case "switch":
      return (
        <Switch
          checked={value >= 0.5}
          aria-label={label}
          disabled={disabled}
          onCheckedChange={(on) => onSet(on ? 1 : 0)}
        />
      );
    case "select":
      return (
        <Select value={String(value)} disabled={disabled} onValueChange={(v) => onSet(Number(v))}>
          <SelectTrigger className="h-8" aria-label={label}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {control.options.map((o) => (
              <SelectItem key={o.value} value={String(o.value)}>
                {o.label[lang]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    case "counter":
      return (
        <CounterControl value={value} unit={unit} lang={lang} disabled={disabled} onSet={onSet} />
      );
  }
}

function SliderControl({
  control,
  value,
  unit,
  label,
  lang,
  disabled,
  onSet,
}: Props & { control: Extract<Control, { kind: "slider" }> }) {
  const [draft, setDraft] = useState<number | null>(null);
  const dragging = useRef(false);
  // A new value from the backend ends the preview - but never while the thumb is being dragged
  // (a generator tick would make it jump back under the cursor).
  // biome-ignore lint/correctness/useExhaustiveDependencies: reacts to `value` only
  useEffect(() => {
    if (!dragging.current) setDraft(null);
  }, [value]);
  const shown = draft ?? value;
  const decimals = stepDecimals(control.step);
  return (
    <div className="flex flex-col gap-1.5">
      <span className="font-mono text-sm text-strong">
        {unit ? formatValue(shown, unit, lang) : formatNumber(shown, decimals, lang)}
      </span>
      <Slider
        min={control.min}
        max={control.max}
        step={control.step}
        value={[shown]}
        disabled={disabled}
        aria-label={label}
        onValueChange={(v) => {
          dragging.current = true;
          setDraft(v[0] ?? null);
        }}
        onValueCommit={(v) => {
          dragging.current = false;
          const next = v[0];
          if (next === undefined) return;
          void Promise.resolve(onSet(next)).then((ok) => {
            // Rejected: the value did not change, so drop the preview (else it shows a value never set).
            if (ok === false) setDraft(null);
          });
        }}
      />
    </div>
  );
}

function CounterControl({
  value,
  unit,
  lang,
  disabled,
  onSet,
}: Pick<Props, "value" | "unit" | "lang" | "disabled" | "onSet">) {
  const { t } = useTranslation();
  // Quick clicks before the backend echoes the new value must add up, not repeat the same target.
  const [optimistic, setOptimistic] = useState<number | null>(null);
  const inflight = useRef(0);
  const valueRef = useRef(value);
  valueRef.current = value;
  // biome-ignore lint/correctness/useExhaustiveDependencies: reacts to `value` only
  useEffect(() => {
    if (inflight.current === 0) setOptimistic(null);
  }, [value]);

  async function add(n: number) {
    const next = (optimistic ?? value) + n;
    setOptimistic(next);
    inflight.current += 1;
    const ok = await Promise.resolve(onSet(next));
    inflight.current -= 1;
    if (inflight.current === 0 && (ok === false || valueRef.current === next)) setOptimistic(null);
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="font-mono text-sm text-strong">
        {formatValue(optimistic ?? value, unit, lang)}
      </span>
      {[1, 10, 100].map((n) => (
        <Button
          key={n}
          size="sm"
          variant="outline"
          disabled={disabled}
          aria-label={t("control.counterAdd", { n })}
          onClick={() => void add(n)}
        >
          +{n}
        </Button>
      ))}
    </div>
  );
}
