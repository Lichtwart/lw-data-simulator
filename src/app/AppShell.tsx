import {
  CpuIcon,
  LogOutIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  SettingsIcon,
} from "lucide-react";
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

import { shortcutLabel } from "./sidebar";
import { useSidebar } from "./useSidebar";

const SHORTCUT = shortcutLabel(typeof navigator === "undefined" ? "" : navigator.platform);

/**
 * Layout for all authenticated screens: Night Blue sidebar + content outlet. The sidebar can be
 * hidden (content takes the full width); on narrow windows it overlays the content.
 */
export function AppShell() {
  const { t } = useTranslation();
  const { auth, loading, logout } = useAuth();
  const update = useUpdateCheck();
  const { session } = useSession();
  const sidebar = useSidebar();

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

  const { layout } = sidebar;
  const hideLabel = t("nav.hideMenu");
  const showLabel = t("nav.showMenu");

  return (
    <div className="flex h-full">
      {layout === "overlay" && sidebar.narrow && (
        <button
          type="button"
          aria-label={hideLabel}
          className="fixed inset-0 z-30 bg-black/40"
          onClick={sidebar.close}
        />
      )}
      {layout === "hidden" && !sidebar.narrow && (
        // Notion-style peek: the pointer at the left edge brings the menu back as an overlay.
        <div
          aria-hidden
          className="fixed inset-y-0 left-0 z-20 w-2"
          onMouseEnter={() => sidebar.setPeek(true)}
        />
      )}
      {layout === "hidden" && (
        <Button
          variant="outline"
          size="icon"
          className="fixed top-2 left-2 z-20 size-8 shadow-xs"
          aria-label={showLabel}
          title={`${showLabel} (${SHORTCUT})`}
          onClick={sidebar.toggle}
        >
          <PanelLeftOpenIcon />
        </Button>
      )}
      <aside
        hidden={layout === "hidden"}
        aria-label={t("nav.menu")}
        onMouseLeave={sidebar.narrow ? undefined : () => sidebar.setPeek(false)}
        className={cn(
          "flex w-56 shrink-0 flex-col gap-6 overflow-y-auto bg-sidebar p-4 text-sidebar-foreground",
          layout === "overlay" && "fixed inset-y-0 left-0 z-40 shadow-lg",
          layout === "hidden" && "hidden",
        )}
      >
        <div className="flex items-start justify-between gap-2">
          <div className="flex flex-col gap-1 px-2 pt-2">
            <img src={logoWhite} alt={t("app.brand")} className="h-6 w-auto self-start" />
            <span className="text-xs text-sidebar-muted">{t("app.title")}</span>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="size-8 shrink-0 text-sidebar-muted hover:bg-white/10 hover:text-sidebar-foreground"
            aria-label={hideLabel}
            title={`${hideLabel} (${SHORTCUT})`}
            onClick={sidebar.narrow ? sidebar.close : sidebar.toggle}
          >
            <PanelLeftCloseIcon />
          </Button>
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
      <main className={cn("min-w-0 flex-1 overflow-auto", layout !== "docked" && "pt-8")}>
        <Outlet />
      </main>
      <OperationPrompts />
    </div>
  );
}
