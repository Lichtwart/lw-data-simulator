import { useTranslation } from "react-i18next";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import { useSession } from "./SessionProvider";

/** Dot + label for the realtime (operations) connection of the running session. */
export function RealtimeIndicator({ className, onDark }: { className?: string; onDark?: boolean }) {
  const { t } = useTranslation();
  const { session, realtimeError } = useSession();
  if (!session) return null;
  const connected = session.realtimeConnected;
  const label = connected ? t("realtime.connected") : t("realtime.disconnected");
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className={cn(
            "inline-flex items-center gap-1.5 text-xs",
            onDark ? "text-sidebar-muted" : "text-muted-foreground",
            className,
          )}
        >
          <span
            aria-hidden
            className={cn("size-2 rounded-full", connected ? "bg-success" : "bg-destructive")}
          />
          {t("realtime.label")}: {label}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {connected ? t("realtime.hintOk") : (realtimeError ?? t("realtime.hintDown"))}
      </TooltipContent>
    </Tooltip>
  );
}
