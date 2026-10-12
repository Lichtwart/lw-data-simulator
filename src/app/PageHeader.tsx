import type { ReactNode } from "react";

/** Screen heading. FT Polar Light (display face) — only for headings ≥ 28px. */
export function PageHeader({ title, actions }: { title: string; actions?: ReactNode }) {
  return (
    <header className="flex items-end justify-between gap-4">
      <h1 className="font-display text-[length:var(--lw-fs-h2)] font-light leading-[var(--lw-lh-heading)] tracking-[var(--lw-tr-heading)] text-strong">
        {title}
      </h1>
      {actions}
    </header>
  );
}
