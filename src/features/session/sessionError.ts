/** The fatal `session-error` cases with their own text (the backend message is English prose). */
export type SessionErrorKey = "session.lockTakenOver" | "session.workerStopped" | "session.crashed";

/** Dedicated i18n key for a known fatal session error, else `null`. */
export function sessionErrorKey(e: {
  code: string;
  message: string;
  fatal: boolean;
}): SessionErrorKey | null {
  if (!e.fatal) return null;
  if (e.code === "deviceLocked") return "session.lockTakenOver";
  if (e.code === "internal") {
    const m = e.message.toLowerCase();
    if (m.includes("outbound worker")) return "session.workerStopped";
    if (m.includes("crashed")) return "session.crashed";
  }
  return null;
}

/** Error codes whose `errors.<code>` text needs no values or only `{{details}}`. */
const DETAILS_CODES = new Set(["network", "notFound", "invalidInput", "library"]);
const PLAIN_CODES = new Set([
  "unknown",
  "notAuthenticated",
  "authFailed",
  "keychain",
  "noSession",
  "sessionActive",
  "io",
  "internal",
]);

/**
 * i18n key and values for the toast text of a `session-error` event. The event only carries the
 * English message, so codes whose text needs structured values (`http`: status / message,
 * `deviceLocked`: user / since) fall back to the generic `code: message` line instead of showing
 * raw `{{placeholders}}`.
 */
export function sessionErrorText(e: { code: string; message: string; fatal: boolean }): {
  key: string;
  values: Record<string, string>;
} {
  const special = sessionErrorKey(e);
  if (special) return { key: special, values: {} };
  if (PLAIN_CODES.has(e.code)) return { key: `errors.${e.code}`, values: {} };
  if (DETAILS_CODES.has(e.code)) return { key: `errors.${e.code}`, values: { details: e.message } };
  return { key: "session.errorGeneric", values: { code: e.code, message: e.message } };
}
