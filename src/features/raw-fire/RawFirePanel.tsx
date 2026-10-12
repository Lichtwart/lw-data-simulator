import { RotateCwIcon, SendIcon } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import {
  api,
  events,
  type FiredRecord,
  type FireRequest,
  type RawTarget,
  type Severity,
} from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatTimestamp, parseDecimal } from "@/lib/format";
import { useAppEvent } from "@/lib/useAppEvent";
import { useAsync } from "@/lib/useAsync";

import { describeFired, upsertHistory } from "./history";

const SEVERITIES: Severity[] = ["CRITICAL", "MAJOR", "MINOR", "WARNING"];
const TARGETS: RawTarget[] = ["measurement", "alarm", "event"];

function validJson(text: string): boolean {
  try {
    JSON.parse(text);
    return true;
  } catch {
    return false;
  }
}

/** Raw fire (PRD §10): measurement / alarm / event / raw JSON forms + history with resend. */
export function RawFirePanel({ deviceId }: { deviceId: string }) {
  const { t, i18n } = useTranslation();
  const history = useAsync(() => api.rawHistory(), []);
  const [records, setRecords] = useState<FiredRecord[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    // Merge, don't replace: records fired while the history was loading must survive.
    const loaded = history.data;
    if (loaded)
      setRecords((cur) =>
        cur.reduce(
          (acc, r) => upsertHistory(acc, r),
          [...loaded].sort((a, b) => b.id - a.id),
        ),
      );
  }, [history.data]);
  useAppEvent(events.rawFired, (r) => setRecords((cur) => upsertHistory(cur, r)));

  // measurement
  const [mType, setMType] = useState("");
  const [mFragment, setMFragment] = useState("");
  const [mSeries, setMSeries] = useState("");
  const [mValue, setMValue] = useState("");
  const [mUnit, setMUnit] = useState("");
  // alarm
  const [aType, setAType] = useState("");
  const [aSeverity, setASeverity] = useState<Severity>("MAJOR");
  const [aText, setAText] = useState("");
  // event
  const [eType, setEType] = useState("");
  const [eText, setEText] = useState("");
  const [eExtra, setEExtra] = useState("");
  // json
  const [jTarget, setJTarget] = useState<RawTarget>("measurement");
  const [jBody, setJBody] = useState("{\n  \n}");

  const value = parseDecimal(mValue);
  const extraInvalid = eExtra.trim() !== "" && !validJson(eExtra);
  const jsonInvalid = !validJson(jBody);

  async function fire(request: FireRequest) {
    setBusy(true);
    setError(null);
    try {
      const rec = await api.fireRaw(deviceId, request);
      setRecords((cur) => upsertHistory(cur, rec));
      toast.success(t("raw.fired", { status: rec.status }));
    } catch (e) {
      setError(e ?? new Error("unknown"));
    } finally {
      setBusy(false);
    }
  }

  const onMeasurement = (e: FormEvent) => {
    e.preventDefault();
    if (value === null) return;
    void fire({
      kind: "measurement",
      type: mType.trim(),
      fragment: mFragment.trim(),
      series: mSeries.trim(),
      value,
      unit: mUnit.trim() === "" ? null : mUnit.trim(),
    });
  };
  const onAlarm = (e: FormEvent) => {
    e.preventDefault();
    void fire({ kind: "alarm", type: aType.trim(), severity: aSeverity, text: aText });
  };
  const onEvent = (e: FormEvent) => {
    e.preventDefault();
    void fire({
      kind: "event",
      type: eType.trim(),
      text: eText,
      extraJson: eExtra.trim() === "" ? null : eExtra,
    });
  };
  const onJson = (e: FormEvent) => {
    e.preventDefault();
    void fire({ kind: "json", target: jTarget, body: jBody });
  };

  async function resend(r: FiredRecord) {
    try {
      const rec = await api.resendRaw(r.id);
      setRecords((cur) => upsertHistory(cur, rec));
      toast.success(t("raw.fired", { status: rec.status }));
    } catch (e) {
      toast.error(errorMessage(e));
    }
  }

  const mine = records.filter((r) => r.deviceId === deviceId);
  const submit = (label: string, disabled: boolean) => (
    <div>
      <Button type="submit" disabled={busy || disabled}>
        <SendIcon />
        {label}
      </Button>
    </div>
  );

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className="flex flex-col gap-4 rounded-lg border bg-card p-5 shadow-xs">
        <h2 className="text-base font-bold text-strong">{t("raw.title")}</h2>
        <Tabs defaultValue="measurement">
          <TabsList>
            <TabsTrigger value="measurement">{t("raw.measurement")}</TabsTrigger>
            <TabsTrigger value="alarm">{t("raw.alarm")}</TabsTrigger>
            <TabsTrigger value="event">{t("raw.event")}</TabsTrigger>
            <TabsTrigger value="json">{t("raw.json")}</TabsTrigger>
          </TabsList>

          <TabsContent value="measurement">
            <form className="flex flex-col gap-3" onSubmit={onMeasurement}>
              <div className="grid grid-cols-2 gap-3">
                <Field label={t("raw.type")} htmlFor="m-type">
                  <Input
                    id="m-type"
                    className="font-mono"
                    value={mType}
                    onChange={(e) => setMType(e.target.value)}
                    required
                  />
                </Field>
                <Field label={t("raw.fragment")} htmlFor="m-frag">
                  <Input
                    id="m-frag"
                    className="font-mono"
                    value={mFragment}
                    onChange={(e) => setMFragment(e.target.value)}
                    required
                  />
                </Field>
                <Field label={t("raw.series")} htmlFor="m-series">
                  <Input
                    id="m-series"
                    className="font-mono"
                    value={mSeries}
                    onChange={(e) => setMSeries(e.target.value)}
                    required
                  />
                </Field>
                <Field
                  label={t("raw.value")}
                  htmlFor="m-value"
                  error={mValue !== "" && value === null ? t("raw.valueInvalid") : null}
                >
                  <Input
                    id="m-value"
                    className="font-mono"
                    inputMode="decimal"
                    value={mValue}
                    onChange={(e) => setMValue(e.target.value)}
                    required
                  />
                </Field>
                <Field label={t("raw.unit")} htmlFor="m-unit">
                  <Input
                    id="m-unit"
                    className="font-mono"
                    value={mUnit}
                    onChange={(e) => setMUnit(e.target.value)}
                  />
                </Field>
              </div>
              {submit(
                t("raw.fire"),
                value === null || !mType.trim() || !mFragment.trim() || !mSeries.trim(),
              )}
            </form>
          </TabsContent>

          <TabsContent value="alarm">
            <form className="flex flex-col gap-3" onSubmit={onAlarm}>
              <div className="grid grid-cols-2 gap-3">
                <Field label={t("raw.type")} htmlFor="a-type">
                  <Input
                    id="a-type"
                    className="font-mono"
                    value={aType}
                    onChange={(e) => setAType(e.target.value)}
                    required
                  />
                </Field>
                <Field label={t("raw.severity")} htmlFor="a-sev">
                  <Select value={aSeverity} onValueChange={(v) => setASeverity(v as Severity)}>
                    <SelectTrigger id="a-sev">
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
              </div>
              <Field label={t("raw.text")} htmlFor="a-text">
                <Input
                  id="a-text"
                  value={aText}
                  onChange={(e) => setAText(e.target.value)}
                  required
                />
              </Field>
              {submit(t("raw.fire"), !aType.trim() || !aText.trim())}
            </form>
          </TabsContent>

          <TabsContent value="event">
            <form className="flex flex-col gap-3" onSubmit={onEvent}>
              <Field label={t("raw.type")} htmlFor="e-type">
                <Input
                  id="e-type"
                  className="font-mono"
                  value={eType}
                  onChange={(e) => setEType(e.target.value)}
                  required
                />
              </Field>
              <Field label={t("raw.text")} htmlFor="e-text">
                <Input
                  id="e-text"
                  value={eText}
                  onChange={(e) => setEText(e.target.value)}
                  required
                />
              </Field>
              <Field
                label={t("raw.extraJson")}
                htmlFor="e-extra"
                hint={t("raw.extraJsonHint")}
                error={extraInvalid ? t("raw.jsonInvalid") : null}
              >
                <Textarea
                  id="e-extra"
                  className="font-mono"
                  rows={4}
                  value={eExtra}
                  onChange={(e) => setEExtra(e.target.value)}
                />
              </Field>
              {submit(t("raw.fire"), !eType.trim() || !eText.trim() || extraInvalid)}
            </form>
          </TabsContent>

          <TabsContent value="json">
            <form className="flex flex-col gap-3" onSubmit={onJson}>
              <Field label={t("raw.target")} htmlFor="j-target" className="max-w-xs">
                <Select value={jTarget} onValueChange={(v) => setJTarget(v as RawTarget)}>
                  <SelectTrigger id="j-target">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {TARGETS.map((x) => (
                      <SelectItem key={x} value={x}>
                        {t(`raw.${x}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field
                label={t("raw.body")}
                htmlFor="j-body"
                error={jsonInvalid ? t("raw.jsonInvalid") : null}
              >
                <Textarea
                  id="j-body"
                  className="font-mono"
                  rows={10}
                  value={jBody}
                  onChange={(e) => setJBody(e.target.value)}
                />
              </Field>
              {submit(t("raw.fire"), jsonInvalid)}
            </form>
          </TabsContent>
        </Tabs>
        {error !== null && <ErrorNotice error={error} />}
      </div>

      <section className="flex flex-col gap-3 rounded-lg border bg-card p-5 shadow-xs">
        <h2 className="text-base font-bold text-strong">{t("raw.history")}</h2>
        {mine.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("raw.historyEmpty")}</p>
        ) : (
          <ul className="flex max-h-[32rem] flex-col divide-y overflow-auto">
            {mine.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  className="flex w-full items-center gap-3 px-1 py-2 text-left hover:bg-accent"
                  title={t("raw.resend")}
                  onClick={() => void resend(r)}
                >
                  <RotateCwIcon className="size-4 shrink-0 text-muted-foreground" />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate font-mono text-xs text-strong">
                      {describeFired(r)}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {formatTimestamp(r.firedAt, i18n.language)} · {r.status}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
