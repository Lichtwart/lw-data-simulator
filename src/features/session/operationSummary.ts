import type { TFunction } from "i18next";

import type { OperationPrompt } from "@/lib/api";

/** The prompt's one-line summary in the current language, built from `prompt.detail`. */
export function operationSummary(
  prompt: Pick<OperationPrompt, "detail">,
  t: TFunction,
  lang: "de" | "en",
): string {
  const d = prompt.detail;
  switch (d.kind) {
    case "groupMode":
      return t("operations.summary.groupMode", {
        group: d.group,
        mode: t(`operations.modes.${d.mode}`),
      });
    case "converterState":
      return t("operations.summary.converterState", {
        converter: d.converter,
        state: t(`operations.modes.${d.on ? "on" : "off"}`),
      });
    case "modbusWrite": {
      const values = {
        slave: d.slaveAddress,
        register: d.registerAddress,
        kind: t(`operations.registerKinds.${d.coil ? "coil" : "register"}`),
        value: d.value,
      };
      return d.itemTitle
        ? t("operations.summary.modbusWriteTitled", { ...values, title: d.itemTitle[lang] })
        : t("operations.summary.modbusWrite", values);
    }
  }
}
