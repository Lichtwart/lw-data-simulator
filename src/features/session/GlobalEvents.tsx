import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { events } from "@/lib/api";
import { useAppEvent } from "@/lib/useAppEvent";
import { operationToast } from "./operationToast";
import { sessionErrorText } from "./sessionError";

/** App-wide event → toast mapping: session errors, operation outcomes, expired login. */
export function GlobalEvents() {
  const { t } = useTranslation();

  useAppEvent(events.sessionError, (e) => {
    const title = e.fatal ? t("session.errorFatal") : t("session.error");
    const text = sessionErrorText(e);
    const description = (t as (k: string, o?: object) => string)(text.key, text.values);
    if (e.fatal) toast.error(title, { description, duration: Number.POSITIVE_INFINITY });
    else toast.error(title, { description });
  });

  useAppEvent(events.operationResolved, (r) => {
    const n = operationToast(r);
    if (!n) return;
    toast[n.level](t(n.key, { type: r.operationType }), { description: n.description });
  });

  useAppEvent(events.authExpired, () => toast.warning(t("login.expired")));

  return null;
}
