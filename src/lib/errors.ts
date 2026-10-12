import i18n from "@/i18n";
import { ApiError, errorKey, errorValues } from "./api";
import { formatSince } from "./format";

/** Translated, user-facing text for any thrown value (`errors.<code>` + details). */
export function errorMessage(err: unknown): string {
  const values = { ...errorValues(err) };
  if (typeof values.since === "string")
    values.since = formatSince(values.since, new Date(), i18n.language);
  const text = i18n.t(errorKey(err), values);
  // `authFailed` carries the server's reason (e.g. "Account locked") as `details`, if any.
  return hasCode(err, "authFailed") && values.details ? `${text} (${values.details})` : text;
}

/** True for a backend `AppError` with the given code. */
function hasCode(err: unknown, code: ApiError["error"]["code"]): boolean {
  return err instanceof ApiError && err.error.code === code;
}

/** `{ user, since }` of a `deviceLocked` backend error (another presenter's lock), else `null`. */
export function deviceLockOf(err: unknown): { user: string; since: string } | null {
  return err instanceof ApiError && err.error.code === "deviceLocked"
    ? { user: err.error.details.user, since: err.error.details.since }
    : null;
}
