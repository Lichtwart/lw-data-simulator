import type { FaultDefinition, LibraryItem_Serialize } from "@/lib/api";

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
export function buildChipGroups(items: readonly { tags: readonly string[] }[]): ChipGroup[] {
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
  /** Only entries with an alarm in effect. */
  activeOnly: boolean;
}

export const EMPTY_FILTER: ItemFilter = { query: "", tags: new Set(), activeOnly: false };

/** Everything the search looks at for an item: title (de + en), id, tags, slave name. */
export function itemHaystack(item: LibraryItem_Serialize, slaveName?: string | null): string {
  const parts = [item.title.de, item.title.en, item.id, ...item.tags];
  if (slaveName) parts.push(slaveName);
  return parts.join("\n").toLowerCase();
}

/** Search text of one alarm: its text, its Cumulocity type and an optional UI label. */
export function faultHaystack(fault: FaultDefinition, label?: string): string {
  return [fault.text, fault.alarmType, label ?? ""].join("\n").toLowerCase();
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

/**
 * One card of the control list: an item (with the alarms that belong to it, e.g. a lamp's
 * native alarms) or an alarm card (DALI bus, a slave's timeout + alarms, a custom alarm item).
 */
export interface FilterableEntry {
  /** `item`: a value card, its alarms are a menu. `alarms`: every alarm is listed. */
  type: "item" | "alarms";
  tags: readonly string[];
  /** Lower-case search text of the card itself. */
  haystack: string;
  faults: readonly FaultDefinition[];
  /** {@link faultHaystack} of each fault, same order. */
  faultHaystacks: readonly string[];
}

export type IsActive = (fault: FaultDefinition) => boolean;

/**
 * Tags, "active alarms only" and the text search (which also finds alarm texts and types)
 * applied to the control list.
 */
export function filterEntries<E extends FilterableEntry>(
  entries: readonly E[],
  filter: ItemFilter,
  isActive: IsActive,
): E[] {
  const query = filter.query.trim();
  return entries.filter((e) => {
    if (!matchesTags(e.tags, filter.tags)) return false;
    if (filter.activeOnly && !e.faults.some(isActive)) return false;
    if (query === "") return true;
    return matchesQuery([e.haystack, ...e.faultHaystacks].join("\n"), query);
  });
}

/**
 * Alarms listed on the card itself: all of an alarm card; of an item card the ones in effect
 * plus the ones the search is about (it needs the alarm's own text to match, i.e. the card alone
 * does not match). The rest stay in the card's menu.
 */
export function inlineFaults(
  entry: FilterableEntry,
  query: string,
  isActive: IsActive,
): FaultDefinition[] {
  if (entry.type === "alarms") return [...entry.faults];
  const q = query.trim();
  const searched = q !== "" && !matchesQuery(entry.haystack, q);
  return entry.faults.filter(
    (f, i) =>
      isActive(f) ||
      (searched && matchesQuery(`${entry.haystack}\n${entry.faultHaystacks[i] ?? ""}`, q)),
  );
}

/** Add / remove a tag from a selection (returns a new set). */
export function toggleTag(selected: ReadonlySet<string>, tag: string): Set<string> {
  const next = new Set(selected);
  if (next.has(tag)) next.delete(tag);
  else next.add(tag);
  return next;
}
