import { api } from "@/lib/api";
import { useAsync } from "@/lib/useAsync";

import { useDevice } from "../devices/DeviceContext";

/**
 * The device's DALI groups and digital inputs for the Setup editors. Reloads when the device,
 * its library version or `configRevision` changes: an apply with the same demo and library
 * version still rewrites the config, and a stale editor would overwrite it on save.
 */
export function useDeviceConfig() {
  const { device, configRevision } = useDevice();
  return useAsync(
    () => api.getDeviceConfig(device.id),
    [device.id, device.demo?.libraryVersion, device.demo?.demoId, configRevision],
  );
}
