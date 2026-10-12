/**
 * Typed frontend API — the only way the UI talks to the Rust backend.
 *
 * - `api.<command>(...)` — every Tauri command from `bindings.ts` (generated from Rust by
 *   tauri-specta, `npm run gen:types`), unwrapped: resolves with the data or throws `ApiError`.
 * - `events.<event>.listen(cb)` — every backend event (see `useAppEvent` for React).
 * - All domain types are re-exported from here; import types from `@/lib/api`, never from
 *   `./bindings` directly.
 *
 * Phased types: when a Rust type uses serde defaults, bindings contain `Foo_Serialize`
 * (what Rust sends — fields always present) and `Foo_Deserialize` (what Rust accepts —
 * defaulted fields optional). Command results are already typed with the `_Serialize` variant.
 */
import { type AppError, commands, events } from "./bindings";

export type * from "./bindings";
export { events };

/** Error thrown by `api.*` when a command returns `Err(AppError)`. */
export class ApiError extends Error {
  readonly error: AppError;

  constructor(error: AppError) {
    super(error.code);
    this.name = "ApiError";
    this.error = error;
  }
}

type Commands = typeof commands;
type Unwrap<R> = R extends { status: "ok"; data: infer D } ? D : never;

/** `commands` with results unwrapped (throws `ApiError` instead of returning a Result). */
export type Api = {
  [K in keyof Commands]: Commands[K] extends (...args: infer A) => Promise<infer R>
    ? (...args: A) => Promise<Unwrap<R>>
    : never;
};

type AnyResult = { status: "ok"; data: unknown } | { status: "error"; error: AppError };
type AnyCommand = (...args: unknown[]) => Promise<AnyResult>;

export const api: Api = Object.fromEntries(
  Object.entries(commands).map(([name, fn]) => [
    name,
    async (...args: unknown[]) => {
      const result = await (fn as AnyCommand)(...args);
      if (result.status === "ok") return result.data;
      throw new ApiError(result.error);
    },
  ]),
) as Api;

/** Every `errors.*` i18n key a backend error can map to (typed, so `t(errorKey(e))` checks). */
export type ErrorKey = `errors.${AppError["code"]}` | "errors.unknown";

/** i18n key for any thrown value: `errors.<code>` for backend errors, else `errors.unknown`. */
export function errorKey(err: unknown): ErrorKey {
  return err instanceof ApiError ? `errors.${err.error.code}` : "errors.unknown";
}

/**
 * Interpolation values for `t(errorKey(err), errorValues(err))`: the error's `details`.
 * A string becomes `{{details}}`; an object (`http`: `status`, `message`; `deviceLocked`:
 * `user`, `since`) is passed field by field.
 */
export function errorValues(err: unknown): Record<string, string | number> {
  if (!(err instanceof ApiError)) return {};
  const details: unknown = "details" in err.error ? err.error.details : undefined;
  if (typeof details === "string") return { details };
  if (details && typeof details === "object") {
    return Object.fromEntries(
      Object.entries(details).filter(
        (e): e is [string, string | number] => typeof e[1] === "string" || typeof e[1] === "number",
      ),
    );
  }
  return {};
}

/** True when running inside the Tauri webview (false in a plain browser / tests). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
