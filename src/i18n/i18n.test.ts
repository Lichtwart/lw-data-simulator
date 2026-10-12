import { describe, expect, it } from "vitest";

import type { AppError } from "@/lib/api";

import de from "./de.json";
import en from "./en.json";

function keys(obj: object, prefix = ""): string[] {
  return Object.entries(obj).flatMap(([k, v]) =>
    v && typeof v === "object" ? keys(v, `${prefix}${k}.`) : [`${prefix}${k}`],
  );
}

describe("i18n", () => {
  it("de and en have the same keys", () => {
    expect(keys(de).sort()).toEqual(keys(en).sort());
  });

  it("has a message for every backend error code", () => {
    // Typed against the generated AppError union: adding a Rust variant fails typecheck here.
    const codes: Record<AppError["code"], true> = {
      notAuthenticated: true,
      authFailed: true,
      http: true,
      network: true,
      notFound: true,
      invalidInput: true,
      library: true,
      keychain: true,
      noSession: true,
      sessionActive: true,
      deviceLocked: true,
      io: true,
      internal: true,
    };
    for (const code of Object.keys(codes)) {
      expect(de.errors).toHaveProperty(code);
      expect(en.errors).toHaveProperty(code);
    }
  });
});
