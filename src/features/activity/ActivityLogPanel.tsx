import { ClipboardIcon, Trash2Icon } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { type ActivityEntry, api, events } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatTimestamp } from "@/lib/format";
import { useAppEvent } from "@/lib/useAppEvent";
import { useAsync } from "@/lib/useAsync";

import { appendEntry, filterEntries, isFailed, mergeEntries } from "./log";

/** Activity log (PRD §16): last 500 requests, newest first, "copy as curl" (password masked). */
export function ActivityLogPanel() {
  const { t, i18n } = useTranslation();
  const initial = useAsync(() => api.activityLog(), []);
  const [entries, setEntries] = useState<ActivityEntry[]>([]);
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<number | null>(null);

  useEffect(() => {
    // Merge, don't replace: events that arrived while the log was loading must survive.
    if (initial.data) setEntries((cur) => mergeEntries(initial.data ?? [], cur));
  }, [initial.data]);
  useAppEvent(events.activityLogged, (e) => setEntries((cur) => appendEntry(cur, e)));

  const shown = useMemo(() => filterEntries(entries, query).slice().reverse(), [entries, query]);

  const toggle = useCallback((id: number) => setExpanded((cur) => (cur === id ? null : id)), []);

  const copyCurl = useCallback(
    async (id: number) => {
      try {
        await navigator.clipboard.writeText(await api.activityAsCurl(id));
        toast.success(t("activity.copied"));
      } catch (e) {
        toast.error(errorMessage(e));
      }
    },
    [t],
  );

  async function clear() {
    try {
      await api.clearActivityLog();
      setEntries([]);
    } catch (e) {
      toast.error(errorMessage(e));
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          type="search"
          className="max-w-sm"
          placeholder={t("activity.search")}
          aria-label={t("activity.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="text-sm text-muted-foreground">
          {t("activity.count", { count: entries.length })}
        </span>
        <Button
          variant="outline"
          size="sm"
          className="ml-auto"
          onClick={() => void clear()}
          disabled={entries.length === 0}
        >
          <Trash2Icon />
          {t("activity.clear")}
        </Button>
      </div>
      {initial.error !== null && <ErrorNotice error={initial.error} />}
      {shown.length === 0 ? (
        <p className="text-muted-foreground">{t("activity.empty")}</p>
      ) : (
        <div className="overflow-auto rounded-lg border bg-card">
          <table className="w-full text-left text-sm">
            <thead className="border-b text-xs tracking-[0.1em] text-muted-foreground uppercase">
              <tr>
                <th className="px-3 py-2 font-bold">{t("activity.time")}</th>
                <th className="px-3 py-2 font-bold">{t("activity.method")}</th>
                <th className="px-3 py-2 font-bold">{t("activity.path")}</th>
                <th className="px-3 py-2 font-bold">{t("activity.status")}</th>
                <th className="px-3 py-2 font-bold">{t("activity.body")}</th>
                <th className="px-3 py-2 text-right font-bold">{t("activity.duration")}</th>
                <th className="px-3 py-2" />
              </tr>
            </thead>
            <tbody className="font-mono text-xs">
              {shown.map((e) => (
                <ActivityRow
                  key={e.id}
                  entry={e}
                  locale={i18n.language}
                  open={expanded === e.id}
                  onToggle={toggle}
                  onCopy={copyCurl}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

const ActivityRow = memo(function ActivityRow({
  entry: e,
  locale,
  open,
  onToggle,
  onCopy,
}: {
  entry: ActivityEntry;
  locale: string;
  open: boolean;
  onToggle: (id: number) => void;
  onCopy: (id: number) => void;
}) {
  const { t } = useTranslation();
  const failed = isFailed(e);
  return (
    <>
      <tr className="border-b last:border-0 hover:bg-accent/50">
        <td className="px-3 py-2 whitespace-nowrap text-muted-foreground">
          {formatTimestamp(e.time, locale)}
        </td>
        <td className="px-3 py-2 font-semibold">{e.method}</td>
        <td className="max-w-[28rem] truncate px-3 py-2">
          <button
            type="button"
            className="max-w-full truncate text-left hover:underline"
            aria-expanded={open}
            onClick={() => onToggle(e.id)}
          >
            {e.path}
          </button>
        </td>
        <td className="px-3 py-2">
          <Badge variant={failed ? "danger" : "success"}>
            {e.status ?? t("activity.noStatus")}
          </Badge>
        </td>
        <td className="max-w-[16rem] truncate px-3 py-2 text-muted-foreground">
          {e.error ?? e.responseBody ?? e.requestBody ?? ""}
        </td>
        <td className="px-3 py-2 text-right text-muted-foreground">{e.durationMs} ms</td>
        <td className="px-3 py-2 text-right">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onCopy(e.id)}
            aria-label={t("activity.copyCurl")}
          >
            <ClipboardIcon />
            {t("activity.copyCurl")}
          </Button>
        </td>
      </tr>
      {open && (
        <tr className="border-b bg-muted/40">
          <td colSpan={7} className="px-3 py-3">
            <dl className="grid gap-2">
              {e.error && <Detail label={t("activity.error")} value={e.error} />}
              {e.requestBody && <Detail label={t("activity.request")} value={e.requestBody} />}
              {e.responseBody && <Detail label={t("activity.response")} value={e.responseBody} />}
              {!e.error && !e.requestBody && !e.responseBody && (
                <span className="text-muted-foreground">{t("activity.noBody")}</span>
              )}
            </dl>
          </td>
        </tr>
      )}
    </>
  );
});

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="mb-0.5 font-sans text-xs font-bold text-muted-foreground">{label}</dt>
      <dd className="max-h-40 overflow-auto break-all whitespace-pre-wrap">{value}</dd>
    </div>
  );
}
