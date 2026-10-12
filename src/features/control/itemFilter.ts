import type { LibraryItem_Serialize, SlaveDefinition } from "@/lib/api";

/** Pure search / filter helpers for the control panel (PRD §11.5). */

export type Lang = "de" | "en";

/** Namespace of a tag: `category:cooling` → `category`, plain tags → `""` (client tags). */
export function tagNamespace(tag: string): string {
  const i = tag.indexOf(":");
  return i < 0 ? "" : tag.slice(0, i);
}

export interface ChipGroup {
  /** `""` = client tags (`retail`, `gas-station`), else `category` / `zone` / … */
  namespace: string;
  tags: { tag: string; count: number }[];
}

/** Filter chips: one group per tag namespace, client tags first, then alphabetical. */
export function buildChipGroups(
  items: readonly Pick<LibraryItem_Serialize, "tags">[],
): ChipGroup[] {
  const counts = new Map<string, Map<string, number>>();
  for (const item of items) {
    for (const tag of item.tags) {
      const ns = tagNamespace(tag);
      const m = counts.get(ns) ?? new Map<string, number>();
      m.set(tag, (m.get(tag) ?? 0) + 1);
      counts.set(ns, m);
    }
  }
  return [...counts.entries()]
    .sort(([a], [b]) => (a === b ? 0 : a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)))
    .map(([namespace, m]) => ({
      namespace,
      tags: [...m.entries()]
        .map(([tag, count]) => ({ tag, count }))
        .sort((x, y) => x.tag.localeCompare(y.tag)),
    }));
}

export interface ItemFilter {
  query: string;
  /** Selected tags. Within a namespace they are OR-ed, across namespaces AND-ed. */
  tags: ReadonlySet<string>;
}

/** Everything the search looks at: title (de + en), id, tags, slave name. */
function searchHaystack(item: LibraryItem_Serialize, slaveName?: string | null): string {
  const parts = [item.title.de, item.title.en, item.id, ...item.tags];
  if (slaveName) parts.push(slaveName);
  return parts.join("\n").toLowerCase();
}

/** Multi-word search: every whitespace separated word must appear somewhere (AND). */
export function matchesQuery(haystack: string, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return words.every((w) => haystack.includes(w));
}

/** Tag selection: OR inside a namespace, AND across namespaces. Empty selection matches all. */
export function matchesTags(itemTags: readonly string[], selected: ReadonlySet<string>): boolean {
  if (selected.size === 0) return true;
  const byNs = new Map<string, string[]>();
  for (const tag of selected) {
    const ns = tagNamespace(tag);
    byNs.set(ns, [...(byNs.get(ns) ?? []), tag]);
  }
  for (const tags of byNs.values()) {
    if (!tags.some((t) => itemTags.includes(t))) return false;
  }
  return true;
}

export function filterItems<T extends LibraryItem_Serialize>(
  items: readonly T[],
  filter: ItemFilter,
  slaves: readonly SlaveDefinition[],
): T[] {
  const slaveNames = new Map(slaves.map((s) => [s.key, s.name]));
  return items.filter((item) => {
    if (!matchesTags(item.tags, filter.tags)) return false;
    if (filter.query.trim() === "") return true;
    const slave = item.kind === "modbus" ? slaveNames.get(item.slave) : undefined;
    return matchesQuery(searchHaystack(item, slave), filter.query);
  });
}

/** Add / remove a tag from a selection (returns a new set). */
export function toggleTag(selected: ReadonlySet<string>, tag: string): Set<string> {
  const next = new Set(selected);
  if (next.has(tag)) next.delete(tag);
  else next.add(tag);
  return next;
}
