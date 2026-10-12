import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { PageHeader } from "@/app/PageHeader";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { type UpdateResult, useUpdateCheck } from "@/features/updates/useUpdateCheck";
import { LANGUAGES, type Language, setLanguage } from "@/i18n";
import { type AppInfo, api, isTauri } from "@/lib/api";
import { getThemePreference, setThemePreference, type ThemePreference } from "@/lib/theme";
import { LibraryWarnings } from "./LibraryWarnings";

const THEMES: ThemePreference[] = ["system", "light", "dark"];

/** Settings: language, theme, version info, manual update check. */
export function SettingsScreen() {
  const { t, i18n } = useTranslation();
  const [theme, setTheme] = useState(getThemePreference());
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [libraryWarnings, setLibraryWarnings] = useState<string[]>([]);
  const updates = useUpdateCheck();
  const [checked, setChecked] = useState<UpdateResult | null>(null);
  const langLabel = useId();
  const themeLabel = useId();

  useEffect(() => {
    if (!isTauri()) return;
    api.appInfo().then(setInfo, () => setInfo(null));
    api.libraryInfo().then(
      (l) => setLibraryWarnings(l.warnings),
      () => setLibraryWarnings([]),
    );
  }, []);

  return (
    <div className="flex max-w-2xl flex-col gap-6 p-8">
      <PageHeader title={t("settings.title")} />
      <Card>
        <CardContent className="flex flex-col gap-6">
          <div className="flex flex-col gap-2">
            <Label id={langLabel}>{t("settings.language")}</Label>
            <ToggleGroup
              type="single"
              aria-labelledby={langLabel}
              value={i18n.language}
              onValueChange={(v) => v && setLanguage(v as Language)}
            >
              {LANGUAGES.map((lng: Language) => (
                <ToggleGroupItem key={lng} value={lng}>
                  {t(`languages.${lng}`)}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
          <div className="flex flex-col gap-2">
            <Label id={themeLabel}>{t("settings.theme")}</Label>
            <ToggleGroup
              type="single"
              aria-labelledby={themeLabel}
              value={theme}
              onValueChange={(v) => {
                if (!v) return;
                setTheme(v as ThemePreference);
                setThemePreference(v as ThemePreference);
              }}
            >
              {THEMES.map((th) => (
                <ToggleGroupItem key={th} value={th}>
                  {t(`settings.themes.${th}`)}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
          <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1 text-sm">
            <dt className="text-muted-foreground">{t("settings.version")}</dt>
            <dd className="font-mono">{info?.version ?? "—"}</dd>
            <dt className="text-muted-foreground">{t("settings.libraryVersion")}</dt>
            <dd className="font-mono">{info?.libraryVersion ?? "—"}</dd>
          </dl>
          <LibraryWarnings warnings={libraryWarnings} />
          <div className="flex items-center gap-3">
            <Button
              size="sm"
              variant="outline"
              disabled={updates.checking}
              onClick={() => void updates.recheck().then(setChecked)}
            >
              {t("settings.checkUpdates")}
            </Button>
            {checked !== null && (
              <span className="text-sm text-muted-foreground">
                {checked === "found"
                  ? t("settings.updateFound")
                  : checked === "error"
                    ? t("settings.checkFailed")
                    : t("settings.upToDate")}
              </span>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
