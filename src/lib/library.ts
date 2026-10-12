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

/** Library slaves plus custom slaves, by key. */
export function mergeSlaves(
  library: SlaveDefinition[],
  custom: SlaveDefinition[],
): SlaveDefinition[] {
  const byKey = new Map(library.map((s) => [s.key, s]));
  for (const c of custom) byKey.set(c.key, c);
  return [...byKey.values()];
}
