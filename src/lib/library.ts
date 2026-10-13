import {
  api,
  type DemoPack_Serialize,
  type LibraryItem_Serialize,
  type SlaveDefinition,
} from "./api";

export interface LibraryData {
  items: LibraryItem_Serialize[];
  slaves: SlaveDefinition[];
  demos: DemoPack_Serialize[];
}

let cached: Promise<LibraryData> | null = null;

/** The bundled library is immutable for the app's lifetime: fetch once, reuse everywhere. */
export function loadLibrary(): Promise<LibraryData> {
  if (!cached) {
    cached = Promise.all([api.listLibraryItems(), api.listSlaves(), api.listDemos()])
      .then(([items, slaves, demos]) => ({ items, slaves, demos }))
      .catch((e) => {
        cached = null;
        throw e;
      });
  }
  return cached;
}

/** Library items overlaid by device-local custom items (custom ids never collide, D6). */
export function mergeItems(
  library: LibraryItem_Serialize[],
  custom: LibraryItem_Serialize[],
): LibraryItem_Serialize[] {
  const byId = new Map(library.map((i) => [i.id, i]));
  for (const c of custom) byId.set(c.id, c);
  return [...byId.values()];
}

/**
 * The scenarios of a device (mirror of Rust `setup::merge_scenarios`): the library demo's
 * (`base`, in order) without the `hidden` ones, each replaced by the device's custom scenario
 * with the same id, followed by the other custom ones.
 */
export function mergeScenarios<S extends { id: string }>(
  base: readonly S[],
  custom: readonly S[],
  hidden: readonly string[],
): S[] {
  const out = base
    .filter((s) => !hidden.includes(s.id))
    .map((s) => custom.find((c) => c.id === s.id) ?? s);
  for (const c of custom) if (!base.some((s) => s.id === c.id)) out.push(c);
  return out;
}

/** Item selection of a setup: `tags ∪ includes − excludes` (mirror of Rust `select_by`). */
export function selectItems(
  items: readonly LibraryItem_Serialize[],
  setup: { tags: readonly string[]; includes: readonly string[]; excludes: readonly string[] },
): LibraryItem_Serialize[] {
  return items.filter(
    (i) =>
      !setup.excludes.includes(i.id) &&
      (i.tags.some((t) => setup.tags.includes(t)) || setup.includes.includes(i.id)),
  );
}

/** Library slaves plus custom slaves, by key. */
export function mergeSlaves(
  library: SlaveDefinition[],
  custom: SlaveDefinition[],
): SlaveDefinition[] {
  const byKey = new Map(library.map((s) => [s.key, s]));
  for (const c of custom) byKey.set(c.key, c);
  return [...byKey.values()];
}
