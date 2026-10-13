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
import { FaultSectionsPanel } from "./FaultSections";
import { FilterBar } from "./FilterBar";
import { encodeStatuses, groupFaults, itemFaultKeyOf, itemFaultOwner } from "./faults";
import { GeneratorDialog, type GeneratorTarget } from "./GeneratorDialog";
import { GroupModes } from "./GroupModes";
import { ItemCard } from "./ItemCard";
import { buildChipGroups, filterItems, type ItemFilter, type Lang } from "./itemFilter";
import { compareItems } from "./itemOrder";
import { ScenarioBar } from "./ScenarioBar";
import { SessionBar } from "./SessionBar";

const EMPTY_FAULTS: readonly FaultDefinition[] = [];

/** Control tab (PRD §8): session bar, scenarios + reset, groups, faults, filterable item cards. */
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
  const [filter, setFilter] = useState<ItemFilter>({ query: "", tags: new Set() });
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

  const rows = useMemo(() => {
    if (!session) return [];
    return session.items
      .flatMap((state) => {
        const meta = metaById.get(state.itemId);
        return meta && meta.kind !== "customAlarm" ? [{ meta, state }] : [];
      })
      .sort((a, b) => compareItems(a.meta, b.meta));
  }, [session, metaById]);

  const lampFaultMap = useMemo(() => {
    const m = new Map<string, FaultDefinition[]>();
    for (const f of catalog.data ?? []) {
      const owner = itemFaultOwner(f.target);
      if (owner) m.set(owner, [...(m.get(owner) ?? []), f]);
    }
    return m;
  }, [catalog.data]);
  const sections = useMemo(() => groupFaults(catalog.data ?? []), [catalog.data]);
  const pack = library.data?.demos.find((d) => d.id === device.demo?.demoId);
  const customScenarios = device.demo?.customScenarios;
  const hiddenScenarios = device.demo?.hiddenScenarios;
  const scenarios = useMemo(
    () => mergeScenarios(pack?.scenarios ?? [], customScenarios ?? [], hiddenScenarios ?? []),
    [pack, customScenarios, hiddenScenarios],
  );

  if (!session) return null;

  const genRow = genItemId ? rows.find((r) => r.meta.id === genItemId) : undefined;
  const genTarget: GeneratorTarget | null = genRow
    ? {
        id: genRow.meta.id,
        title: genRow.meta.title[lang],
        generator: genRow.state.generator,
        center: genRow.meta.control?.default ?? 0,
      }
    : null;

  const metas = rows.map((r) => r.meta);
  const visible = new Set(filterItems(metas, filter, slaves).map((m) => m.id));
  const chipGroups = buildChipGroups(metas);
  const shownRows = rows.filter((r) => visible.has(r.meta.id));
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
      <FaultSectionsPanel
        deviceId={deviceId}
        sections={sections}
        slaves={slaves}
        activeFaults={session.activeFaults}
      />

      {library.error !== null && <ErrorNotice error={library.error} />}
      <FilterBar
        filter={filter}
        onChange={setFilter}
        groups={chipGroups}
        shown={shownRows.length}
        total={rows.length}
      />

      {shownRows.length === 0 ? (
        <p className="text-muted-foreground">{t("control.noItems")}</p>
      ) : (
        <ul className="grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-3">
          {shownRows.map(({ meta, state }) => {
            const owner = itemFaultKeyOf(meta);
            const faults = (owner !== null ? lampFaultMap.get(owner) : undefined) ?? EMPTY_FAULTS;
            const statuses = encodeStatuses(faults, session.activeFaults);
            return (
              <li key={meta.id} className="flex">
                <ItemCard
                  meta={meta}
                  state={state}
                  lang={lang}
                  lampFaults={faults}
                  lampFaultStatuses={statuses}
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
