import { CopyIcon, PencilIcon, PlusIcon, RotateCcwIcon, Trash2Icon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { api, type FaultDefinition, type Scenario_Serialize } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { loadLibrary, mergeItems, mergeScenarios, selectItems } from "@/lib/library";
import { useAsync } from "@/lib/useAsync";
import { useSeededForm } from "@/lib/useSeededForm";

import type { Lang } from "../control/itemFilter";
import { useDevice } from "../devices/DeviceContext";
import { useReportDirty } from "./dirtyGuard";
import { ScenarioEditorDialog } from "./ScenarioEditorDialog";
import { SectionCard } from "./SectionCard";
import { duplicateScenario, scenarioOrigin, scenarioPatch, upsertScenario } from "./scenarioDraft";

const NO_SCENARIOS: Scenario_Serialize[] = [];
const NO_FAULTS: FaultDefinition[] = [];

/**
 * The device's scenarios (Setup tab): the demo pack's as edited on this device plus new ones
 * (`lw_demo.customScenarios` / `hiddenScenarios`). Create, edit, duplicate, delete; saved with
 * `update_demo_setup` (a running session takes them at once).
 */
export function ScenariosEditor() {
  const { t, i18n } = useTranslation();
  const lang: Lang = i18n.language === "en" ? "en" : "de";
  const { device, reload } = useDevice();
  const demo = device.demo;
  const library = useAsync(() => loadLibrary(), []);
  const catalog = useAsync<FaultDefinition[]>(() => api.faultCatalog(device.id), [device.id, demo]);

  const base = useMemo(
    () => library.data?.demos.find((d) => d.id === demo?.demoId)?.scenarios ?? NO_SCENARIOS,
    [library.data, demo?.demoId],
  );
  const seed = useMemo(
    () => mergeScenarios(base, demo?.customScenarios ?? [], demo?.hiddenScenarios ?? []),
    [base, demo],
  );
  const { value: list, setValue: setList, dirty, discard, markSaved } = useSeededForm(seed);
  useReportDirty("scenarios", dirty);
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<{ scenario: Scenario_Serialize | null } | null>(null);

  const items = useMemo(() => {
    if (!library.data || !demo) return [];
    const all = mergeItems(library.data.items, demo.customItems);
    return selectItems(all, {
      tags: demo.tags,
      // Custom items are selected unless excluded (D6).
      includes: [...demo.includes, ...demo.customItems.map((i) => i.id)],
      excludes: demo.excludes,
    });
  }, [library.data, demo]);
  const takenIds = useMemo(
    () => new Set(list.filter((s) => s.id !== editor?.scenario?.id).map((s) => s.id)),
    [list, editor],
  );
  const deleted = base.filter((b) => !list.some((s) => s.id === b.id));

  if (!demo) return null;

  async function save() {
    setBusy(true);
    try {
      await api.updateDemoSetup(device.id, {
        tags: null,
        includes: null,
        excludes: null,
        daliLampCount: null,
        sendIntervalSec: null,
        realisticMode: null,
        customSlaves: null,
        customItems: null,
        generators: null,
        ...scenarioPatch(base, list),
      });
      markSaved();
      toast.success(t("setup.saved"));
      reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const restore = (id: string) => {
    const lib = base.find((b) => b.id === id);
    if (!lib) return;
    setList((cur) => {
      if (cur.some((s) => s.id === id)) return upsertScenario(cur, lib);
      // A deleted one goes back before the next library scenario that follows it.
      const rank = (sid: string) => base.findIndex((b) => b.id === sid);
      const before = cur.findIndex((x) => rank(x.id) > rank(id));
      const at = before < 0 ? cur.length : before;
      const next = [...cur.slice(0, at), lib, ...cur.slice(at)];
      return lib.start
        ? next.map((x) => (x.id !== id && x.start ? { ...x, start: false } : x))
        : next;
    });
  };

  return (
    <SectionCard
      title={t("setup.scenarios.title")}
      description={t("setup.scenarios.description")}
      dirty={dirty}
      onDiscard={discard}
      actions={
        <Button size="sm" variant="outline" onClick={() => setEditor({ scenario: null })}>
          <PlusIcon />
          {t("setup.scenarios.add")}
        </Button>
      }
    >
      {library.error !== null && <ErrorNotice error={library.error} />}
      {catalog.error !== null && <ErrorNotice error={catalog.error} />}
      {list.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("setup.scenarios.none")}</p>
      ) : (
        <ul className="divide-y rounded-md border">
          {list.map((s) => {
            const origin = scenarioOrigin(base, s);
            return (
              <li key={s.id} className="flex items-start gap-3 px-3 py-2">
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-semibold text-strong">{s.name[lang]}</span>
                    {s.start && <Badge variant="info">{t("setup.scenarios.start")}</Badge>}
                    {origin !== "library" && (
                      <Badge variant="secondary">{t(`setup.scenarios.origin.${origin}`)}</Badge>
                    )}
                    <span className="text-xs text-muted-foreground">
                      {t("setup.scenarios.stepCount", { count: s.set.length })}
                    </span>
                  </div>
                  {s.description && (
                    <p className="text-xs text-muted-foreground">{s.description[lang]}</p>
                  )}
                </div>
                {origin === "edited" && (
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={t("setup.scenarios.restore")}
                    title={t("setup.scenarios.restore")}
                    onClick={() => restore(s.id)}
                  >
                    <RotateCcwIcon />
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("devices.edit")}
                  onClick={() => setEditor({ scenario: s })}
                >
                  <PencilIcon />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("setup.scenarios.duplicate")}
                  title={t("setup.scenarios.duplicate")}
                  onClick={() =>
                    setList((cur) => {
                      const copy = duplicateScenario(s, new Set(cur.map((x) => x.id)), {
                        de: t("setup.scenarios.copySuffix", { lng: "de" }),
                        en: t("setup.scenarios.copySuffix", { lng: "en" }),
                      });
                      const at = cur.findIndex((x) => x.id === s.id);
                      return [...cur.slice(0, at + 1), copy, ...cur.slice(at + 1)];
                    })
                  }
                >
                  <CopyIcon />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("common.delete")}
                  onClick={() => setList((cur) => cur.filter((x) => x.id !== s.id))}
                >
                  <Trash2Icon />
                </Button>
              </li>
            );
          })}
        </ul>
      )}

      {deleted.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span className="text-muted-foreground">{t("setup.scenarios.deleted")}</span>
          {deleted.map((s) => (
            <Button key={s.id} size="sm" variant="ghost" onClick={() => restore(s.id)}>
              <RotateCcwIcon />
              {s.name[lang]}
            </Button>
          ))}
        </div>
      )}

      <div>
        <Button disabled={busy || !dirty} onClick={() => void save()}>
          {t("common.save")}
        </Button>
      </div>

      <ScenarioEditorDialog
        open={editor !== null}
        onOpenChange={(o) => !o && setEditor(null)}
        initial={editor?.scenario ?? null}
        items={items}
        faults={catalog.data ?? NO_FAULTS}
        takenIds={takenIds}
        lang={lang}
        onSubmit={(s) => setList((cur) => upsertScenario(cur, s))}
      />
    </SectionCard>
  );
}
