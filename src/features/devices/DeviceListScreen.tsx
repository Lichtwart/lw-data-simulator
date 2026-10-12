import {
  ArrowRightIcon,
  LockIcon,
  MoreHorizontalIcon,
  PencilIcon,
  PlusIcon,
  RotateCcwIcon,
  SearchIcon,
  Trash2Icon,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { toast } from "sonner";

import { PageHeader } from "@/app/PageHeader";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ErrorNotice } from "@/components/ErrorNotice";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, type DemoDeviceSummary } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatSince, isLockFresh } from "@/lib/format";
import { useAsync } from "@/lib/useAsync";

import { useSession } from "../session/SessionProvider";
import { useNow } from "../session/useNow";
import { DeviceFormDialog, type DeviceFormValues } from "./DeviceFormDialog";
import { groupDevices } from "./groupDevices";

/** Demo device list (PRD §6.2): grouped, searchable, create / open / edit / reset / delete. */
export function DeviceListScreen() {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { session, adopt, mark } = useSession();

  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const id = setTimeout(() => setDebounced(query.trim()), 250);
    return () => clearTimeout(id);
  }, [query]);

  const list = useAsync(
    () => api.listDemoDevices(debounced === "" ? null : debounced),
    [debounced],
  );
  const sections = useMemo(() => groupDevices(list.data ?? []), [list.data]);

  const [form, setForm] = useState<{ device: DeviceFormValues | null } | null>(null);
  const [toReset, setToReset] = useState<DemoDeviceSummary | null>(null);
  const [deleteFloorplans, setDeleteFloorplans] = useState(false);
  const [toDelete, setToDelete] = useState<DemoDeviceSummary | null>(null);
  const [toOpen, setToOpen] = useState<DemoDeviceSummary | null>(null);
  const [busy, setBusy] = useState(false);

  function open(d: DemoDeviceSummary) {
    if (session && session.deviceId !== d.id) setToOpen(d);
    else navigate(`/devices/${d.id}/control`);
  }

  async function switchAndOpen() {
    if (!toOpen) return;
    setBusy(true);
    try {
      const m = mark();
      await api.stopSession();
      adopt(null, m);
      navigate(`/devices/${toOpen.id}/control`);
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
      setToOpen(null);
    }
  }

  async function edit(d: DemoDeviceSummary) {
    try {
      const full = await api.getDemoDevice(d.id);
      setForm({
        device: {
          id: full.id,
          name: full.name,
          description: full.description,
          position: full.position,
        },
      });
    } catch (e) {
      toast.error(errorMessage(e));
    }
  }

  async function reset() {
    if (!toReset) return;
    setBusy(true);
    try {
      await api.resetDemoDevice(toReset.id, { deleteFloorplans });
      toast.success(t("devices.resetDone"));
      list.reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
      setToReset(null);
    }
  }

  async function remove() {
    if (!toDelete) return;
    setBusy(true);
    try {
      await api.deleteDemoDevice(toDelete.id);
      toast.success(t("devices.deleteDone"));
      list.reload();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setBusy(false);
      setToDelete(null);
    }
  }

  const lang = i18n.language;
  const now = useNow(60_000);

  return (
    <div className="flex flex-col gap-6 p-8">
      <PageHeader
        title={t("devices.title")}
        actions={
          <Button variant="signal" onClick={() => setForm({ device: null })}>
            <PlusIcon />
            {t("devices.create")}
          </Button>
        }
      />
      <div className="relative max-w-md">
        <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-subtle" />
        <Input
          type="search"
          className="pl-9"
          placeholder={t("devices.search")}
          aria-label={t("devices.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {list.error !== null && <ErrorNotice error={list.error} />}
      {list.loading && list.data === undefined && (
        <p className="text-muted-foreground">{t("common.loading")}</p>
      )}
      {!list.loading && list.error === null && sections.length === 0 && (
        <p className="text-muted-foreground">{t("devices.empty")}</p>
      )}

      {sections.map((section) => (
        <section key={section.group?.id ?? "__none__"} className="flex flex-col gap-3">
          <h2 className="text-xs font-bold tracking-[0.14em] text-muted-foreground uppercase">
            {section.group?.name ?? t("devices.ungrouped")}
          </h2>
          <ul className="flex flex-col gap-2">
            {section.devices.map((d) => {
              const running = session?.deviceId === d.id;
              const locked = !running && isLockFresh(d.lock, now);
              return (
                <li
                  key={d.id}
                  className="flex items-center gap-4 rounded-lg border bg-card p-4 shadow-xs"
                >
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-bold text-strong">{d.name}</span>
                      {d.demoId && <Badge variant="secondary">{d.demoId}</Badge>}
                      {d.libraryVersion && (
                        <span className="font-mono text-xs text-muted-foreground">
                          {d.libraryVersion}
                        </span>
                      )}
                      {d.upgradeAvailable && (
                        <Badge variant="warning">{t("devices.upgradeAvailable")}</Badge>
                      )}
                      {running && <Badge variant="success">{t("devices.running")}</Badge>}
                      {locked && d.lock && (
                        <Badge variant="info">
                          <LockIcon />
                          {t("devices.inUse", {
                            user: d.lock.user,
                            since: formatSince(d.lock.since, new Date(), lang),
                          })}
                        </Badge>
                      )}
                    </div>
                    {d.description && (
                      <p className="truncate text-sm text-muted-foreground">{d.description}</p>
                    )}
                  </div>
                  <Button onClick={() => open(d)}>
                    <ArrowRightIcon />
                    {t("devices.open")}
                  </Button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon" aria-label={t("devices.actions")}>
                        <MoreHorizontalIcon />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem onSelect={() => void edit(d)}>
                        <PencilIcon />
                        {t("devices.edit")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onSelect={() => {
                          setDeleteFloorplans(false);
                          setToReset(d);
                        }}
                      >
                        <RotateCcwIcon />
                        {t("devices.reset")}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem destructive onSelect={() => setToDelete(d)}>
                        <Trash2Icon />
                        {t("common.delete")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </li>
              );
            })}
          </ul>
        </section>
      ))}

      <DeviceFormDialog
        open={form !== null}
        onOpenChange={(o) => !o && setForm(null)}
        device={form?.device}
        onSaved={(saved) => {
          list.reload();
          if (!form?.device) navigate(`/devices/${saved.id}/control`);
        }}
      />

      <ConfirmDialog
        open={toReset !== null}
        onOpenChange={(o) => !o && setToReset(null)}
        title={t("devices.resetTitle", { name: toReset?.name })}
        description={t("devices.resetText")}
        confirmLabel={t("devices.reset")}
        busy={busy}
        onConfirm={() => void reset()}
      >
        <Label className="font-normal">
          <Checkbox
            checked={deleteFloorplans}
            onCheckedChange={(v) => setDeleteFloorplans(v === true)}
          />
          {t("devices.resetFloorplans")}
        </Label>
      </ConfirmDialog>

      <ConfirmDialog
        open={toDelete !== null}
        onOpenChange={(o) => !o && setToDelete(null)}
        title={t("devices.deleteTitle", { name: toDelete?.name })}
        description={t("devices.deleteText")}
        confirmLabel={t("common.delete")}
        destructive
        busy={busy}
        onConfirm={() => void remove()}
      />

      <ConfirmDialog
        open={toOpen !== null}
        onOpenChange={(o) => !o && setToOpen(null)}
        title={t("session.switchTitle")}
        description={t("session.switchTextOpen", { current: session?.deviceName })}
        confirmLabel={t("session.switch")}
        busy={busy}
        onConfirm={() => void switchAndOpen()}
      />
    </div>
  );
}
