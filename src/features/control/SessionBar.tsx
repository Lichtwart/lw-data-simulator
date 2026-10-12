import { CameraIcon, SendIcon, SquareIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatSince } from "@/lib/format";

import { RealtimeIndicator } from "../session/RealtimeIndicator";
import { useSession } from "../session/SessionProvider";
import type { SessionHead } from "../session/sessionState";
import { clampInterval, MAX_SEND_INTERVAL_SEC, MIN_SEND_INTERVAL_SEC } from "./sessionBarUtils";

/** Session controls: stop, send interval, auto-accept, send all now, save snapshot. */
export function SessionBar({ session }: { session: SessionHead }) {
  const { t, i18n } = useTranslation();
  const { adopt, mark } = useSession();
  const [interval, setIntervalText] = useState(String(session.sendIntervalSec));
  const [busy, setBusy] = useState(false);

  useEffect(() => setIntervalText(String(session.sendIntervalSec)), [session.sendIntervalSec]);

  async function act(fn: () => Promise<unknown>, success?: string) {
    try {
      await fn();
      if (success) toast.success(success);
    } catch (e) {
      toast.error(errorMessage(e));
    }
  }

  async function commitInterval() {
    const raw = Number(interval);
    const n = clampInterval(raw);
    if (n !== null && Math.round(raw) < MIN_SEND_INTERVAL_SEC)
      toast.info(t("control.intervalMin", { min: MIN_SEND_INTERVAL_SEC }));
    if (n === null || n === session.sendIntervalSec) {
      setIntervalText(String(session.sendIntervalSec));
      return;
    }
    setIntervalText(String(n));
    await act(() => api.setSendInterval(n));
  }

  async function stop() {
    setBusy(true);
    await act(async () => {
      const m = mark();
      await api.stopSession();
      adopt(null, m);
    });
    setBusy(false);
  }

  return (
    <section
      aria-label={t("control.session")}
      className="flex flex-wrap items-center gap-x-6 gap-y-3 rounded-lg border bg-card px-4 py-3 shadow-xs"
    >
      <div className="flex flex-col gap-1">
        <div className="flex items-center gap-2">
          <Badge variant={session.status === "running" ? "success" : "secondary"}>
            {t(`control.status.${session.status}`)}
          </Badge>
          {session.realisticMode && <Badge variant="info">{t("control.realistic")}</Badge>}
        </div>
        <span className="text-xs text-muted-foreground">
          {t("control.since", { time: formatSince(session.startedAt, new Date(), i18n.language) })}
          {session.lastSentAt && (
            <>
              {" · "}
              {t("control.lastSent", {
                time: formatSince(session.lastSentAt, new Date(), i18n.language),
              })}
            </>
          )}
        </span>
        <RealtimeIndicator />
      </div>

      <div className="flex items-center gap-2">
        <Label htmlFor="send-interval" className="whitespace-nowrap">
          {t("control.interval")}
        </Label>
        <Input
          id="send-interval"
          type="number"
          inputMode="numeric"
          min={MIN_SEND_INTERVAL_SEC}
          max={MAX_SEND_INTERVAL_SEC}
          className="w-20 font-mono"
          value={interval}
          onChange={(e) => setIntervalText(e.target.value)}
          onBlur={() => void commitInterval()}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        <span className="text-sm text-muted-foreground">{t("control.seconds")}</span>
      </div>

      <div className="flex items-center gap-2">
        <Switch
          id="auto-accept"
          checked={session.autoAccept}
          onCheckedChange={(enabled) => void act(() => api.setAutoAccept(enabled))}
        />
        <Label htmlFor="auto-accept" className="font-normal">
          {t("control.autoAccept")}
        </Label>
      </div>

      <div className="ml-auto flex flex-wrap items-center gap-2">
        <Button
          variant="signal"
          onClick={() => void act(() => api.sendAllNow(), t("control.sent"))}
        >
          <SendIcon />
          {t("control.sendAll")}
        </Button>
        <Button
          variant="outline"
          onClick={() => void act(() => api.saveSnapshot(), t("control.snapshotSaved"))}
        >
          <CameraIcon />
          {t("control.saveSnapshot")}
        </Button>
        <Button variant="outline" onClick={() => void stop()} disabled={busy}>
          <SquareIcon />
          {t("control.stop")}
        </Button>
      </div>
    </section>
  );
}
