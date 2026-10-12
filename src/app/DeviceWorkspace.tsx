import { ArrowLeftIcon, LockIcon, PencilIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, Navigate, useNavigate, useParams } from "react-router";

import { ErrorNotice } from "@/components/ErrorNotice";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ActivityLogPanel } from "@/features/activity/ActivityLogPanel";
import { useAuth } from "@/features/auth/AuthProvider";
import { ControlPanel } from "@/features/control/ControlPanel";
import { SetupPanel } from "@/features/demo/SetupPanel";
import { DeviceContext } from "@/features/devices/DeviceContext";
import { DeviceFormDialog } from "@/features/devices/DeviceFormDialog";
import { RawFirePanel } from "@/features/raw-fire/RawFirePanel";
import { useSession } from "@/features/session/SessionProvider";
import { api } from "@/lib/api";
import { formatSince, isLockFresh } from "@/lib/format";
import { useAsync } from "@/lib/useAsync";

export const WORKSPACE_TABS = ["control", "setup", "raw", "activity"] as const;
export type WorkspaceTab = (typeof WORKSPACE_TABS)[number];

function isTab(v: string | undefined): v is WorkspaceTab {
  return WORKSPACE_TABS.includes(v as WorkspaceTab);
}

/** Device workspace: header + tabs Control / Setup / Raw fire / Activity log (tab is in the URL). */
export function DeviceWorkspace() {
  const { deviceId, tab } = useParams();
  if (!deviceId) return <Navigate to="/devices" replace />;
  if (!isTab(tab)) return <Navigate to={`/devices/${deviceId}/control`} replace />;
  return <Workspace key={deviceId} deviceId={deviceId} tab={tab} />;
}

function Workspace({ deviceId, tab }: { deviceId: string; tab: WorkspaceTab }) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { auth } = useAuth();
  const { session } = useSession();
  const [editing, setEditing] = useState(false);
  const [configRevision, setConfigRevision] = useState(0);
  const reloadConfig = useCallback(() => setConfigRevision((n) => n + 1), []);
  const state = useAsync(() => api.getDemoDevice(deviceId), [deviceId]);
  const device = state.data;
  const formValues = useMemo(
    () =>
      device
        ? {
            id: device.id,
            name: device.name,
            description: device.description,
            position: device.position,
          }
        : null,
    [device],
  );

  if (!device) {
    return (
      <div className="flex flex-col gap-4 p-8">
        <BackLink />
        {state.error !== null ? (
          <ErrorNotice error={state.error} />
        ) : (
          <p className="text-muted-foreground">{t("common.loading")}</p>
        )}
      </div>
    );
  }

  const lock = device.demo?.lock;
  const foreignLock =
    lock && lock.user !== auth?.username && session?.deviceId !== device.id && isLockFresh(lock)
      ? lock
      : null;

  return (
    <DeviceContext.Provider
      value={{
        device,
        reload: state.reload,
        configRevision,
        reloadConfig,
        setDevice: state.setData,
      }}
    >
      <div className="flex flex-col gap-6 p-8">
        <div className="flex flex-col gap-2">
          <BackLink />
          <header className="flex items-start justify-between gap-4">
            <div className="flex min-w-0 flex-col gap-1">
              <span className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                {device.parentGroup?.name ?? t("workspace.title")}
              </span>
              <h1 className="truncate text-xl font-bold text-strong">{device.name}</h1>
              {device.description && (
                <p className="text-sm text-muted-foreground">{device.description}</p>
              )}
              <div className="mt-1 flex flex-wrap items-center gap-2">
                {device.demo?.demoId && <Badge variant="secondary">{device.demo.demoId}</Badge>}
                {device.demo?.libraryVersion && (
                  <span className="font-mono text-xs text-muted-foreground">
                    {device.demo.libraryVersion}
                  </span>
                )}
                {device.upgradeAvailable && (
                  <Link to={`/devices/${device.id}/setup`}>
                    <Badge variant="warning">{t("devices.upgradeAvailable")}</Badge>
                  </Link>
                )}
                {foreignLock && (
                  <Badge variant="info">
                    <LockIcon />
                    {t("devices.inUse", {
                      user: foreignLock.user,
                      since: formatSince(foreignLock.since, new Date(), i18n.language),
                    })}
                  </Badge>
                )}
                {device.serialNumber && (
                  <span className="font-mono text-xs text-muted-foreground">
                    {device.serialNumber}
                  </span>
                )}
              </div>
            </div>
            <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
              <PencilIcon />
              {t("devices.edit")}
            </Button>
          </header>
        </div>
        <Tabs value={tab} onValueChange={(v) => navigate(`/devices/${device.id}/${v}`)}>
          <TabsList>
            {WORKSPACE_TABS.map((key) => (
              <TabsTrigger key={key} value={key}>
                {t(`workspace.tabs.${key}`)}
              </TabsTrigger>
            ))}
          </TabsList>
          <TabsContent value="control">
            <ControlPanel deviceId={device.id} />
          </TabsContent>
          <TabsContent value="setup">
            <SetupPanel />
          </TabsContent>
          <TabsContent value="raw">
            <RawFirePanel deviceId={device.id} />
          </TabsContent>
          <TabsContent value="activity">
            <ActivityLogPanel />
          </TabsContent>
        </Tabs>
      </div>
      <DeviceFormDialog
        open={editing}
        onOpenChange={setEditing}
        device={formValues}
        onSaved={state.setData}
      />
    </DeviceContext.Provider>
  );
}

function BackLink() {
  const { t } = useTranslation();
  return (
    <Link
      to="/devices"
      className="inline-flex w-fit items-center gap-1.5 text-sm font-semibold text-muted-foreground hover:text-strong"
    >
      <ArrowLeftIcon className="size-4" />
      {t("workspace.back")}
    </Link>
  );
}
