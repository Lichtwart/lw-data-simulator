import { CheckIcon, XIcon } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { api, isTauri, type OperationPrompt } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { remainingPercent, secondsRemaining } from "@/lib/format";

import { operationDecision } from "./operationDecision";
import { operationSummary } from "./operationSummary";
import { useSession } from "./SessionProvider";
import { useNow } from "./useNow";

/** One Confirm / Abort card with a countdown to the 60 s timeout (PRD §9). */
export function OperationPromptCard({
  prompt,
  now,
  onAnswer,
}: {
  prompt: OperationPrompt;
  now: number;
  onAnswer: (decision: "confirm" | "abort", reason?: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [reason, setReason] = useState("");
  const seconds = secondsRemaining(prompt.expiresAt, now);
  const expired = seconds === 0;
  const titleId = useId();
  const descId = useId();
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={descId}
      className="flex w-80 flex-col gap-3 rounded-lg border bg-popover p-4 text-popover-foreground shadow-lg"
    >
      <div className="flex flex-col gap-1">
        <span
          id={titleId}
          className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase"
        >
          {t(`operations.kinds.${prompt.kind}`)}
        </span>
        <p id={descId} className="text-sm font-semibold text-strong">
          {operationSummary(prompt, t, i18n.language === "en" ? "en" : "de")}
        </p>
        {prompt.by && (
          <p className="text-xs text-muted-foreground">{t("operations.by", { user: prompt.by })}</p>
        )}
      </div>
      <div className="flex flex-col gap-1">
        <Progress value={remainingPercent(prompt.receivedAt, prompt.expiresAt, now)} />
        <span className="font-mono text-xs text-muted-foreground" aria-live="off">
          {t("operations.countdown", { seconds })}
        </span>
      </div>
      <Input
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        placeholder={t("operations.reason")}
        aria-label={t("operations.reason")}
        disabled={expired}
        maxLength={200}
      />
      <div className="flex gap-2">
        <Button size="sm" disabled={expired} onClick={() => onAnswer("confirm")}>
          <CheckIcon />
          {t("operations.confirm")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={expired}
          onClick={() => onAnswer("abort", reason)}
        >
          <XIcon />
          {t("operations.abort")}
        </Button>
      </div>
    </div>
  );
}

/** How long a card stays visible after its countdown reached 0 (the `operation-resolved` event is due). */
const EXPIRED_GRACE_MS = 5000;

/** Ask the OS to flash / bounce the window when a prompt arrives while the app is in the background. */
function requestAttention() {
  if (!isTauri() || document.hasFocus()) return;
  import("@tauri-apps/api/window")
    .then(({ getCurrentWindow, UserAttentionType }) =>
      getCurrentWindow().requestUserAttention(UserAttentionType.Critical),
    )
    .catch(() => {
      // Missing capability or unsupported platform: the live region still announces it.
    });
}

/** Floating stack of pending visible operations (shown on every screen while a session runs). */
export function OperationPrompts() {
  const { t, i18n } = useTranslation();
  const { session } = useSession();
  const prompts = session?.pendingOperations ?? [];
  const [announcement, setAnnouncement] = useState("");
  const seen = useRef<Set<string>>(new Set());

  // Announce each new prompt once. The live region stays mounted so screen readers pick it up.
  useEffect(() => {
    const fresh = prompts.filter((p) => !seen.current.has(p.operationId));
    if (fresh.length === 0) return;
    for (const p of fresh) seen.current.add(p.operationId);
    setAnnouncement(
      fresh
        .map((p) =>
          t("operations.announce", {
            kind: t(`operations.kinds.${p.kind}`),
            summary: operationSummary(p, t, i18n.language === "en" ? "en" : "de"),
            seconds: secondsRemaining(p.expiresAt),
          }),
        )
        .join(". "),
    );
    requestAttention();
  }, [prompts, t, i18n.language]);

  return (
    <>
      <div role="status" aria-live="assertive" className="sr-only">
        {announcement}
      </div>
      {prompts.length > 0 && <PromptStack prompts={prompts} />}
    </>
  );
}

function PromptStack({ prompts }: { prompts: readonly OperationPrompt[] }) {
  const { t } = useTranslation();
  const now = useNow(1000);
  const [answered, setAnswered] = useState<ReadonlySet<string>>(new Set());

  async function answer(prompt: OperationPrompt, decision: "confirm" | "abort", reason?: string) {
    setAnswered((s) => new Set(s).add(prompt.operationId));
    try {
      await api.answerOperation(prompt.operationId, operationDecision(decision, reason));
    } catch (e) {
      setAnswered((s) => {
        const next = new Set(s);
        next.delete(prompt.operationId);
        return next;
      });
      toast.error(errorMessage(e));
    }
  }

  return (
    <section
      className="fixed right-4 bottom-4 z-40 flex flex-col gap-3"
      aria-label={t("operations.region")}
    >
      {prompts
        .filter(
          (p) => !answered.has(p.operationId) && now - Date.parse(p.expiresAt) < EXPIRED_GRACE_MS,
        )
        .map((p) => (
          <OperationPromptCard
            key={p.operationId}
            prompt={p}
            now={now}
            onAnswer={(d, reason) => void answer(p, d, reason)}
          />
        ))}
    </section>
  );
}
