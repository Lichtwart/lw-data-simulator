import type { LocalizableText } from "./api";

/**
 * A library text in `lang`: a plain string is the same in both languages, `{de, en}` picks one
 * (DECISIONS D12: slave names, alarm / event texts, group names, floorplan texts may be either).
 */
export function textIn(text: LocalizableText, lang: "de" | "en"): string {
  return typeof text === "string" ? text : text[lang];
}
