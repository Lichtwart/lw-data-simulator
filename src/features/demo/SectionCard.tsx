import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

/** Titled block of the Setup tab. `dirty` shows the "unsaved changes" badge (+ discard). */
export function SectionCard({
  title,
  description,
  actions,
  dirty,
  onDiscard,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  dirty?: boolean;
  onDiscard?: () => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <section className="flex flex-col gap-4 rounded-lg border bg-card p-5 shadow-xs">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-base font-bold text-strong">{title}</h2>
            {dirty && <Badge variant="warning">{t("setup.unsaved")}</Badge>}
            {dirty && onDiscard && (
              <Button size="sm" variant="ghost" className="h-6 px-2" onClick={onDiscard}>
                {t("setup.discard")}
              </Button>
            )}
          </div>
          {description && (
            <p className="max-w-prose text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        {actions}
      </header>
      {children}
    </section>
  );
}
