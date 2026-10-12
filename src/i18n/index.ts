/**
 * i18n setup (react-i18next). German + English from day one; no hard-coded UI strings.
 * Add every key to BOTH de.json and en.json — `src/i18n/i18n.test.ts` enforces parity.
 * Backend errors map to `errors.<AppError code>` (see `errorKey` in `@/lib/api`).
 */
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import de from "./de.json";
import en from "./en.json";

export const LANGUAGES = ["de", "en"] as const;
export type Language = (typeof LANGUAGES)[number];

const STORAGE_KEY = "lw.language";

function initialLanguage(): Language {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "de" || stored === "en") return stored;
  } catch {
    // storage unavailable
  }
  return typeof navigator !== "undefined" && navigator.language.startsWith("en") ? "en" : "de";
}

export function setLanguage(lng: Language) {
  try {
    localStorage.setItem(STORAGE_KEY, lng);
  } catch {
    // storage unavailable
  }
  void i18n.changeLanguage(lng);
}

void i18n.use(initReactI18next).init({
  resources: { de: { translation: de }, en: { translation: en } },
  lng: initialLanguage(),
  fallbackLng: "de",
  interpolation: { escapeValue: false },
  returnNull: false,
});

// Keep <html lang> in sync (screen readers, hyphenation), also for the initial language.
function syncHtmlLang(lng: string) {
  if (typeof document !== "undefined") document.documentElement.lang = lng;
}
i18n.on("languageChanged", syncHtmlLang);
syncHtmlLang(i18n.language);

export default i18n;
