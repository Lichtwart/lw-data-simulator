import type { Language } from "./api";

/** The UI language as a content language (anything but English is German). */
export function uiLanguage(lng: string | undefined): Language {
  return lng?.startsWith("en") ? "en" : "de";
}

/**
 * Default content language for an apply: the device's own language when it already has a demo
 * (absent = `de`, DECISIONS D12), otherwise the UI language, as on device creation.
 */
export function defaultContentLanguage(
  demo: { demoId?: string | null; language?: Language | null } | null | undefined,
  ui: Language,
): Language {
  if (!demo?.demoId) return ui;
  return demo.language ?? "de";
}
