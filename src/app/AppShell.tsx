import { CpuIcon, LogOutIcon, SettingsIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Navigate, NavLink, Outlet } from "react-router";
import { toast } from "sonner";

import logoWhite from "@/assets/logos/lichtwart-white.svg";
import { Button } from "@/components/ui/button";
import { useAuth } from "@/features/auth/AuthProvider";
import { OperationPrompts } from "@/features/session/OperationPrompts";
import { RealtimeIndicator } from "@/features/session/RealtimeIndicator";
import { useSession } from "@/features/session/SessionProvider";
import { UpdateBadge } from "@/features/updates/UpdateBadge";
import { useUpdateCheck } from "@/features/updates/useUpdateCheck";
import { errorMessage } from "@/lib/errors";
import { cn } from "@/lib/utils";

/** Layout for all authenticated screens: Night Blue sidebar + content outlet. */
export function AppShell() {
  const { t } = useTranslation();
  const { auth, loading, logout } = useAuth();
  const update = useUpdateCheck();
  const { session } = useSession();

  if (loading) {
    return <div className="p-8 text-muted-foreground">{t("common.loading")}</div>;
  }
  if (!auth) return <Navigate to="/login" replace />;

  const navClass = ({ isActive }: { isActive: boolean }) =>
    cn(
      "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-semibold transition-colors",
      isActive
        ? "bg-white/10 text-sidebar-foreground"
        : "text-sidebar-muted hover:text-sidebar-foreground",
    );

  return (
    <div className="flex h-full">
      <aside className="flex w-56 shrink-0 flex-col gap-6 bg-sidebar p-4 text-sidebar-foreground">
        <div className="flex flex-col gap-1 px-2 pt-2">
          <img src={logoWhite} alt={t("app.brand")} className="h-6 w-auto self-start" />
          <span className="text-xs text-sidebar-muted">{t("app.title")}</span>
        </div>
        <nav className="flex flex-col gap-1">
          <NavLink to="/devices" className={navClass}>
            <CpuIcon className="size-4" />
            {t("nav.devices")}
          </NavLink>
          <NavLink to="/settings" className={navClass}>
            <SettingsIcon className="size-4" />
            {t("nav.settings")}
          </NavLink>
        </nav>
        <div className="mt-auto flex flex-col gap-3 px-2">
          {session && (
            <NavLink
              to={`/devices/${session.deviceId}/control`}
              className="flex flex-col gap-1 rounded-md bg-white/10 px-3 py-2 text-xs hover:bg-white/15"
            >
              <span className="text-sidebar-muted">{t("nav.session")}</span>
              <span className="truncate font-semibold">{session.deviceName}</span>
              <RealtimeIndicator onDark />
            </NavLink>
          )}
          {update.update && <UpdateBadge update={update.update} />}
          <div className="text-xs text-sidebar-muted">
            <div className="font-mono">{auth.tenant.tenantId}</div>
            <div>{auth.username}</div>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="justify-start text-sidebar-muted hover:bg-white/10 hover:text-sidebar-foreground"
            onClick={() => logout().catch((e) => toast.error(errorMessage(e)))}
          >
            <LogOutIcon />
            {t("nav.logout")}
          </Button>
        </div>
      </aside>
      <main className="min-w-0 flex-1 overflow-auto">
        <Outlet />
      </main>
      <OperationPrompts />
    </div>
  );
}
