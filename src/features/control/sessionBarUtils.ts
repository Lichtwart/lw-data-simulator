/** Max periodic send interval: the floorplan plugin marks values stale after 900 s (D5). */
export const MAX_SEND_INTERVAL_SEC = 899;

/** Min periodic send interval: the backend rejects less with `invalidInput`. */
export const MIN_SEND_INTERVAL_SEC = 5;

/** Clamp user input to a whole number of seconds in 5..899 (`null` for non-numbers). */
export function clampInterval(input: number): number | null {
  if (!Number.isFinite(input)) return null;
  return Math.min(MAX_SEND_INTERVAL_SEC, Math.max(MIN_SEND_INTERVAL_SEC, Math.round(input)));
}
