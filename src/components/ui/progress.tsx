import type * as React from "react";

import { cn } from "@/lib/utils";

/** Thin progress bar (0–100). Night Blue fill; orange stays reserved for the view's one CTA. */
function Progress({
  value,
  className,
  ...props
}: { value: number } & Omit<React.ComponentProps<"div">, "children">) {
  const pct = Math.max(0, Math.min(100, value));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(pct)}
      className={cn("h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
      {...props}
    >
      <div
        className="h-full rounded-full bg-primary transition-[width] duration-300"
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export { Progress };
