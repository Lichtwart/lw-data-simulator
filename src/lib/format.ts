/** Pure formatting / parsing helpers (unit-tested in `format.test.ts`). */

/** Advisory device locks are ignored after 10 minutes (PRD §6.3). */
const LOCK_TTL_MS = 10 * 60 * 1000;

/** Number with at most `maxDecimals` decimals, trailing zeros removed (`18.50` → `18.5`). */
export function formatNumber(value: number, maxDecimals = 2, locale = "de"): string {
  if (!Number.isFinite(value)) return "–";
  return new Intl.NumberFormat(locale, { maximumFractionDigits: maxDecimals }).format(value);
}

/** Value with unit and a (thin) space: `18.5 °C`, `50 %`. Numbers always come with units. */
export function formatValue(value: number, unit?: string | null, locale = "de"): string {
  const n = formatNumber(value, 2, locale);
  return unit ? `${n} ${unit}` : n;
}

/** Decimals worth showing for a slider step (`0.5` → 1, `1` → 0, `0.05` → 2). */
export function stepDecimals(step: number): number {
  if (!Number.isFinite(step) || step <= 0) return 2;
  const s = String(step);
  const dot = s.indexOf(".");
  return dot < 0 ? 0 : Math.min(4, s.length - dot - 1);
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** `12:04` for today, otherwise `12.10.2026, 12:04` (locale aware). Invalid input → "–". */
export function formatSince(iso: string, now: Date = new Date(), locale = "de"): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "–";
  const time = new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" });
  if (isSameDay(d, now)) return time.format(d);
  const date = new Intl.DateTimeFormat(locale, { dateStyle: "medium" });
  return `${date.format(d)}, ${time.format(d)}`;
}

/** Full time stamp with seconds, for logs. */
export function formatTimestamp(iso: string, locale = "de"): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "–";
  return new Intl.DateTimeFormat(locale, {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(d);
}

/** True while an advisory lock is younger than the 10-minute TTL. */
export function isLockFresh(lock: { since: string } | null | undefined, now = Date.now()): boolean {
  if (!lock) return false;
  const t = Date.parse(lock.since);
  return Number.isFinite(t) && now - t <= LOCK_TTL_MS;
}

/** Parse a decimal that may use a comma (`48,137`). Returns `null` for empty / invalid input. */
export function parseDecimal(text: string): number | null {
  const s = text.trim().replace(",", ".");
  if (s === "" || !/^[-+]?\d*\.?\d+$|^[-+]?\d+\.$/.test(s)) return null;
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

export type CoordinateError = "latitude" | "longitude";

/**
 * Validate the lat/lng text fields. Both empty → `{ position: null }` (no position);
 * one empty or out of range → an error naming the field.
 */
export function parsePosition(
  lat: string,
  lng: string,
): { position: { lat: number; lng: number } | null; error: CoordinateError | null } {
  if (lat.trim() === "" && lng.trim() === "") return { position: null, error: null };
  const la = parseDecimal(lat);
  if (la === null || la < -90 || la > 90) return { position: null, error: "latitude" };
  const lo = parseDecimal(lng);
  if (lo === null || lo < -180 || lo > 180) return { position: null, error: "longitude" };
  return { position: { lat: la, lng: lo }, error: null };
}

/** Coordinate for a text field: 5 decimals (~1 m). */
export function formatCoordinate(n: number): string {
  return String(Math.round(n * 1e5) / 1e5);
}

/** Seconds left until `expiresAt` (never negative, rounded up). */
export function secondsRemaining(expiresAt: string, now: number = Date.now()): number {
  const t = Date.parse(expiresAt);
  if (!Number.isFinite(t)) return 0;
  return Math.max(0, Math.ceil((t - now) / 1000));
}

/** Share (0–100) of the prompt window that is left. */
export function remainingPercent(
  receivedAt: string,
  expiresAt: string,
  now: number = Date.now(),
): number {
  const start = Date.parse(receivedAt);
  const end = Date.parse(expiresAt);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 0;
  return Math.max(0, Math.min(100, ((end - now) / (end - start)) * 100));
}
