// Typed translation keys: `t("typo.key")` fails `tsc`. German is the source of truth
// (`i18n.test.ts` keeps `en.json` in sync).
import "i18next";

import type de from "./i18n/de.json";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: typeof de };
  }
}
