import { SearchIcon, XIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

import { type ChipGroup, type ItemFilter, toggleTag } from "./itemFilter";

/** Full-text search (multi-word AND) + tag chips by namespace. */
export function FilterBar({
  filter,
  onChange,
  groups,
  shown,
  total,
}: {
  filter: ItemFilter;
  onChange: (f: ItemFilter) => void;
  groups: readonly ChipGroup[];
  shown: number;
  total: number;
}) {
  const { t } = useTranslation();
  const active = filter.query !== "" || filter.tags.size > 0;
  return (
    <section className="flex flex-col gap-3" aria-label={t("control.filter")}>
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative w-full max-w-md">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-subtle" />
          <Input
            type="search"
            className="pl-9"
            placeholder={t("control.search")}
            aria-label={t("control.search")}
            value={filter.query}
            onChange={(e) => onChange({ ...filter, query: e.target.value })}
          />
        </div>
        <span className="text-sm text-muted-foreground">
          {t("control.shown", { shown, total })}
        </span>
        {active && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onChange({ query: "", tags: new Set() })}
          >
            <XIcon />
            {t("control.clearFilter")}
          </Button>
        )}
      </div>
      {groups.map((g) => (
        <div key={g.namespace || "__client"} className="flex flex-wrap items-center gap-1.5">
          <span className="mr-1 w-20 text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
            {g.namespace === "" ? t("control.tagsClient") : g.namespace}
          </span>
          {g.tags.map(({ tag, count }) => {
            const on = filter.tags.has(tag);
            return (
              <button
                key={tag}
                type="button"
                aria-pressed={on}
                onClick={() => onChange({ ...filter, tags: toggleTag(filter.tags, tag) })}
                className={cn(
                  "inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-xs font-semibold transition-colors",
                  on
                    ? "border-transparent bg-primary text-primary-foreground"
                    : "bg-card text-foreground hover:bg-accent",
                )}
              >
                {g.namespace === "" ? tag : tag.slice(g.namespace.length + 1)}
                <span className={cn("font-mono", on ? "opacity-70" : "text-muted-foreground")}>
                  {count}
                </span>
              </button>
            );
          })}
        </div>
      ))}
    </section>
  );
}
