/** Pure parsers / validators for the demo editor forms (unit-tested in `parse.test.ts`). */

/** `"1, 2 ,3"` → `[1,2,3]`; `null` if any entry is not an integer in `[min, max]`. Empty → `[]`. */
export function parseNumberList(text: string, min: number, max: number): number[] | null {
  const parts = text
    .split(/[\s,;]+/)
    .map((p) => p.trim())
    .filter(Boolean);
  const out: number[] = [];
  for (const p of parts) {
    if (!/^\d+$/.test(p)) return null;
    const n = Number(p);
    if (n < min || n > max) return null;
    if (!out.includes(n)) out.push(n);
  }
  return out.sort((a, b) => a - b);
}

/** `"a, b"` → `["a","b"]` (trimmed, no empties, no duplicates). */
export function parseTagList(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[,\n]/)
        .map((t) => t.trim())
        .filter(Boolean),
    ),
  ];
}

/** Item ids are `[A-Za-z0-9_-]`, 1–19 chars (they become Modbus register names, PRD §11.3). */
export function isValidItemId(id: string): boolean {
  return /^[A-Za-z0-9_-]{1,19}$/.test(id);
}

/** Demo ids are slugs (`^[a-z0-9]+(-[a-z0-9]+)*$`, library `demo.schema.json`), not register names. */
export function isValidDemoId(id: string): boolean {
  return /^[a-z0-9]+(-[a-z0-9]+)*$/.test(id);
}

/** Device-level custom alarm names must not start with two digits plus `_` (D2). */
export function isValidDeviceAlarmName(name: string): boolean {
  return isValidItemId(name) && !/^\d\d_/.test(name);
}

/** Slave keys: non-empty `[A-Za-z0-9_-]`. Slave display names: 1–19 chars (PRD §7.4). */
export function isValidSlaveKey(key: string): boolean {
  return /^[A-Za-z0-9_-]{1,40}$/.test(key);
}

export function isValidSlaveName(name: string): boolean {
  return name.trim().length >= 1 && name.length <= 19;
}

/** Modbus register unit: ≤ 7 chars (PRD §7.4). */
export function isValidUnit(unit: string): boolean {
  return unit.length <= 7;
}

/** Integer field: `null` for empty / non-numeric / out of range. */
export function parseIntIn(text: string, min: number, max: number): number | null {
  if (!/^-?\d+$/.test(text.trim())) return null;
  const n = Number(text);
  return n >= min && n <= max ? n : null;
}

/** Plain slug for a demo id: lowercase, `[a-z0-9-]`. */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
