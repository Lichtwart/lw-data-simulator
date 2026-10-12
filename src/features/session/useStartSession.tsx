import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ApiError, api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatSince } from "@/lib/format";

import { useSession } from "./SessionProvider";

type Pending = { kind: "switch" } | { kind: "force"; user: string; since: string };

/**
 * Start a session with the two confirmations the contract asks for:
 * `sessionActive` → "stop and switch?", `deviceLocked` → "take over?" (retry with `force`).
 * Render `dialog` somewhere in the tree.
 */
export function useStartSession(deviceId: string, autoAccept = false) {
  const { t, i18n } = useTranslation();
  const { adopt, mark } = useSession();
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);

  async function start(force = false) {
    setBusy(true);
    try {
      const m = mark();
      adopt(await api.startSession({ deviceId, force, autoAccept }), m);
      setPending(null);
    } catch (e) {
      if (e instanceof ApiError && e.error.code === "sessionActive") {
        setPending({ kind: "switch" });
      } else if (e instanceof ApiError && e.error.code === "deviceLocked") {
        setPending({ kind: "force", user: e.error.details.user, since: e.error.details.since });
      } else {
        setPending(null);
        toast.error(errorMessage(e));
      }
    } finally {
      setBusy(false);
    }
  }

  async function confirm() {
    if (!pending) return;
    if (pending.kind === "force") return start(true);
    setBusy(true);
    try {
      const m = mark();
      await api.stopSession();
      adopt(null, m);
    } catch (e) {
      toast.error(errorMessage(e));
      setBusy(false);
      return;
    }
    setBusy(false);
    await start(false);
  }

  const dialog = (
    <ConfirmDialog
      open={pending !== null}
      onOpenChange={(open) => !open && setPending(null)}
      title={pending?.kind === "force" ? t("session.takeOverTitle") : t("session.switchTitle")}
      description={
        pending?.kind === "force"
          ? t("session.takeOverText", {
              user: pending.user,
              since: formatSince(pending.since, new Date(), i18n.language),
            })
          : t("session.switchText")
      }
      confirmLabel={pending?.kind === "force" ? t("session.takeOver") : t("session.switch")}
      busy={busy}
      onConfirm={() => void confirm()}
    />
  );

  return { start: () => start(false), busy, dialog };
}
