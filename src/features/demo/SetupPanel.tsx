import { useTranslation } from "react-i18next";

import { ErrorNotice } from "@/components/ErrorNotice";
import type { DaliGroup_Serialize, DigitalInputEntry_Serialize } from "@/lib/api";

import { ApplyDemoCard } from "./ApplyDemoCard";
import { CustomEditor } from "./CustomEditor";
import { DaliGroupsEditor } from "./DaliGroupsEditor";
import { DigitalInputsEditor } from "./DigitalInputsEditor";
import { SetupDirtyGuard } from "./dirtyGuard";
import { ExportCard } from "./ExportCard";
import { ScenariosEditor } from "./ScenariosEditor";
import { SetupSettingsCard } from "./SetupSettingsCard";
import { UpgradeCard } from "./UpgradeCard";
import { useDeviceConfig } from "./useDeviceConfig";

const NO_GROUPS: DaliGroup_Serialize[] = [];
const NO_INPUTS: DigitalInputEntry_Serialize[] = [];

/** Setup tab: apply / upgrade a demo pack, edit the device's demo setup, export as pack. */
export function SetupPanel() {
  const { t } = useTranslation();
  const config = useDeviceConfig();

  return (
    <SetupDirtyGuard>
      <div className="flex max-w-4xl flex-col gap-5">
        <UpgradeCard />
        <ApplyDemoCard />
        <SetupSettingsCard />
        {config.error !== null ? (
          <section className="flex flex-col gap-2">
            <h2 className="text-base font-bold text-strong">{t("setup.config.title")}</h2>
            <ErrorNotice error={config.error} />
          </section>
        ) : (
          <>
            <DaliGroupsEditor
              groups={config.data?.groups ?? NO_GROUPS}
              onSaved={(groups) => config.data && config.setData({ ...config.data, groups })}
            />
            <DigitalInputsEditor
              inputs={config.data?.digitalInputs ?? NO_INPUTS}
              onSaved={(digitalInputs) =>
                config.data && config.setData({ ...config.data, digitalInputs })
              }
            />
          </>
        )}
        <CustomEditor />
        <ScenariosEditor />
        <ExportCard />
      </div>
    </SetupDirtyGuard>
  );
}
