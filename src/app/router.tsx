/**
 * Routes (hash router — works from the Tauri asset protocol without a server):
 *
 *   #/login                          LoginScreen          (features/auth)
 *   #/devices                        DeviceListScreen     (features/devices)
 *   #/devices/:deviceId/:tab         DeviceWorkspace      (app/DeviceWorkspace)
 *        tab = control | setup | raw | activity
 *   #/settings                       SettingsScreen       (features/settings)
 *
 * Everything except /login renders inside AppShell, which redirects to /login when
 * there is no authenticated user.
 */
import { createHashRouter, Navigate } from "react-router";

import { LoginScreen } from "@/features/auth/LoginScreen";
import { DeviceListScreen } from "@/features/devices/DeviceListScreen";

import { AppShell } from "./AppShell";

export const router = createHashRouter([
  { path: "/login", element: <LoginScreen /> },
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <Navigate to="/devices" replace /> },
      { path: "devices", element: <DeviceListScreen /> },
      {
        path: "devices/:deviceId",
        element: <Navigate to="control" replace />,
      },
      // Heavy screens load on demand (workspace: setup editors, generators, raw fire, ...).
      {
        path: "devices/:deviceId/:tab",
        lazy: async () => ({
          Component: (await import("./DeviceWorkspace")).DeviceWorkspace,
        }),
      },
      {
        path: "settings",
        lazy: async () => ({
          Component: (await import("@/features/settings/SettingsScreen")).SettingsScreen,
        }),
      },
      { path: "*", element: <Navigate to="/devices" replace /> },
    ],
  },
]);
