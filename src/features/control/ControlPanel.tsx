import { PlayIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Button } from "@/components/ui/button";
import { api, type FaultDefinition } from "@/lib/api";
import { loadLibrary, mergeItems, mergeScenarios, mergeSlaves } from "@/lib/library";
import { useAsync } from "@/lib/useAsync";

import { useDevice } from "../devices/DeviceContext";
import { useDeviceSession, useDeviceSessionHead, useSession } from "../session/SessionProvider";
import { useStartSession } from "../session/useStartSession";
import { AlarmCard, ClearAllAlarms } from "./FaultSections";
import { FilterBar } from "./FilterBar";
import { activeFaultPredicate, buildEntries, encodeStatuses } from "./faults";
import { GeneratorDialog, type GeneratorTarget } from "./GeneratorDialog";
import { GroupModes } from "./GroupModes";
import { ItemCard } from "./ItemCard";
import {
  buildChipGroups,
  EMPTY_FILTER,
  filterEntries,
  type ItemFilter,
  inlineFaults,
  type Lang,
} from "./itemFilter";
import { compareItems } from "./itemOrder";
import { ScenarioBar } from "./ScenarioBar";
import { SessionBar } from "./SessionBar";

/**
 * Control tab (PRD §8): session bar, scenarios + reset, groups, one filterable list of item cards
 * and alarm cards (lamp alarms on the lamp, slave alarms with the slave's registers).
 */
export function ControlPanel({ deviceId }: { deviceId: string }) {
  // Head only: the value-tick subscription lives in `Running`, so the tab switch doesn't re-render per tick.
  const session = useDeviceSessionHead(deviceId);
  return session ? <Running deviceId={deviceId} /> : <NotRunning deviceId={deviceId} />;
}

function NotRunning({ deviceId }: { deviceId: string }) {
  const { t } = useTranslation();
  const { device } = useDevice();
  const { session: other } = useSession();
  const { start, busy, dialog } = useStartSession(deviceId);
  return (
    <div className="flex max-w-xl flex-col items-start gap-4 rounded-lg border bg-card p-6 shadow-xs">
      <h2 className="text-lg font-bold text-strong">{t("control.empty")}</h2>
      <p className="text-sm text-muted-foreground">
        {other ? t("control.otherRunning", { name: other.deviceName }) : t("control.emptyHint")}
      </p>
      {!device.demo?.demoId && (
        <p className="text-sm text-muted-foreground">
          {t("control.noDemo")}{" "}
          <Link className="font-semibold text-strong underline" to={`/devices/${device.id}/setup`}>
            {t("control.goToSetup")}
          </Link>
        </p>
      )}
      <Button variant="signal" onClick={() => void start()} disabled={busy}>
        <PlayIcon />
        {t("control.start")}
      </Button>
      {dialog}
    </div>
  );
}

function Running({ deviceId }: { deviceId: string }) {
  const { t, i18n } = useTranslation();
  const lang: Lang = i18n.language === "en" ? "en" : "de";
  const { device } = useDevice();
  const session = useDeviceSession(deviceId);
  const library = useAsync(() => loadLibrary(), []);
  const catalog = useAsync<FaultDefinition[]>(
    () => api.faultCatalog(deviceId),
    [deviceId, session?.startedAt],
  );
  const [filter, setFilter] = useState<ItemFilter>(EMPTY_FILTER);
  const [genItemId, setGenItemId] = useState<string | null>(null);

  const customItems = device.demo?.customItems;
  const customSlaves = device.demo?.customSlaves;
  const items = useMemo(
    () => mergeItems(library.data?.items ?? [], customItems ?? []),
    [library.data, customItems],
  );
  const slaves = useMemo(
    () => mergeSlaves(library.data?.slaves ?? [], customSlaves ?? []),
    [library.data, customSlaves],
  );
  const metaById = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);
  const slaveNames = useMemo(() => new Map(slaves.map((s) => [s.key, s.name])), [slaves]);

  // The list's shape depends on which items run, not on their values: rebuilt only when the item
  // set changes, so the cards' fault arrays keep their identity (memo) across value ticks.
  const itemIds = session?.items.map((s) => s.itemId).join("\n") ?? "";
  const rows = useMemo(
    () =>
      itemIds
        .split("\n")
        .flatMap((id) => {
          const meta = metaById.get(id);
          return meta ? [{ meta }] : [];
        })
        .sort((a, b) => compareItems(a.meta, b.meta)),
    [itemIds, metaById],
  );

  const entries = useMemo(
    () =>
      buildEntries(rows, catalog.data ?? [], slaves, {
        lang,
        daliBus: t("control.daliBus"),
        slave: (name) => t("control.slave", { name }),
        slaveTimeout: t("control.slaveTimeout"),
      }),
    [rows, catalog.data, slaves, lang, t],
  );
  const chipGroups = useMemo(() => buildChipGroups(entries), [entries]);
  const pack = library.data?.demos.find((d) => d.id === device.demo?.demoId);
  const customScenarios = device.demo?.customScenarios;
  const hiddenScenarios = device.demo?.hiddenScenarios;
  const scenarios = useMemo(
    () => mergeScenarios(pack?.scenarios ?? [], customScenarios ?? [], hiddenScenarios ?? []),
    [pack, customScenarios, hiddenScenarios],
  );

  if (!session) return null;

  const stateById = new Map(session.items.map((s) => [s.itemId, s]));
  const genMeta = genItemId ? metaById.get(genItemId) : undefined;
  const genState = genItemId ? stateById.get(genItemId) : undefined;
  const genTarget: GeneratorTarget | null =
    genMeta && genState
      ? {
          id: genMeta.id,
          title: genMeta.title[lang],
          generator: genState.generator,
          center: genMeta.kind === "customAlarm" ? 0 : (genMeta.control?.default ?? 0),
        }
      : null;

  const isActive = activeFaultPredicate(session.activeFaults);
  const shown = filterEntries(entries, filter, isActive);
  const unknown = session.items.length - rows.length;

  return (
    <div className="flex flex-col gap-5">
      <SessionBar session={session} />
      {session.restartNeeded && (
        <p
          role="status"
          className="rounded-md border border-warning bg-warning-bg p-3 text-sm text-strong"
        >
          {t("control.restartNeeded")}
        </p>
      )}

      <ScenarioBar scenarios={scenarios} lang={lang} />

      <GroupModes groups={session.groups} />
      {catalog.error !== null && <ErrorNotice error={catalog.error} />}

      {library.error !== null && <ErrorNotice error={library.error} />}
      <FilterBar
        filter={filter}
        onChange={setFilter}
        groups={chipGroups}
        shown={shown.length}
        total={entries.length}
        activeAlarms={session.activeFaults.length}
        alarmActions={<ClearAllAlarms deviceId={deviceId} />}
      />

      {shown.length === 0 ? (
        <p className="text-muted-foreground">{t("control.noItems")}</p>
      ) : (
        <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(15rem,100%),1fr))] gap-3">
          {shown.map((entry) => {
            const statuses = encodeStatuses(entry.faults, session.activeFaults);
            if (entry.type === "alarms") {
              return (
                <li key={entry.key} className="flex">
                  <AlarmCard
                    title={entry.title}
                    subtitle={entry.subtitle}
                    faults={entry.faults}
                    statuses={statuses}
                  />
                </li>
              );
            }
            const { meta } = entry.row;
            const state = stateById.get(meta.id);
            if (!state) return null;
            return (
              <li key={entry.key} className="flex">
                <ItemCard
                  meta={meta}
                  state={state}
                  lang={lang}
                  lampFaults={entry.faults}
                  lampFaultStatuses={statuses}
                  inlineFaultTypes={inlineFaults(entry, filter.query, isActive)
                    .map((f) => f.alarmType)
                    .join(",")}
                  slaveName={meta.kind === "modbus" ? slaveNames.get(meta.slave) : null}
                  onEditGenerator={setGenItemId}
                />
              </li>
            );
          })}
        </ul>
      )}
      <GeneratorDialog item={genTarget} onClose={() => setGenItemId(null)} />
      {unknown > 0 && (
        <p className="text-xs text-muted-foreground">
          {t("control.unknownItems", { count: unknown })}
        </p>
      )}
    </div>
  );
}
