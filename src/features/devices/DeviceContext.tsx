import { createContext, useContext } from "react";

import type { DemoDevice_Serialize } from "@/lib/api";

export interface DeviceContextValue {
  device: DemoDevice_Serialize;
  /** Re-fetch the device (after apply / upgrade / setup edits). */
  reload: () => void;
  /** Counts config-changing operations (apply / upgrade); the Setup editors reload on change. */
  configRevision: number;
  /** The device's groups / inputs changed on the server (apply / upgrade): reload the editors. */
  reloadConfig: () => void;
  setDevice: (device: DemoDevice_Serialize) => void;
}

export const DeviceContext = createContext<DeviceContextValue | null>(null);

export function useDevice(): DeviceContextValue {
  const ctx = useContext(DeviceContext);
  if (!ctx) throw new Error("useDevice outside DeviceWorkspace");
  return ctx;
}
