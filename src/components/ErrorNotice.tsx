import { AlertTriangleIcon } from "lucide-react";

import { errorMessage } from "@/lib/errors";

/** Inline error box for a failed load / action. */
export function ErrorNotice({ error, className }: { error: unknown; className?: string }) {
  return (
    <div
      role="alert"
      className={`flex items-start gap-2 rounded-md border border-destructive/30 bg-danger-bg p-3 text-sm text-strong ${className ?? ""}`}
    >
      <AlertTriangleIcon className="mt-0.5 size-4 shrink-0 text-destructive" />
      <span>{errorMessage(error)}</span>
    </div>
  );
}
