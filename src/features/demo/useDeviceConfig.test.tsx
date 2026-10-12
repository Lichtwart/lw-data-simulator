import { act, renderHook, waitFor } from "@testing-library/react";
import { type ReactNode, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DemoDevice_Serialize } from "@/lib/api";

const getDeviceConfig = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api", async (orig) => {
  const real = await orig<typeof import("@/lib/api")>();
  return { ...real, isTauri: () => true, api: { getDeviceConfig } };
});

import { DeviceContext } from "../devices/DeviceContext";
import { useDeviceConfig } from "./useDeviceConfig";

const device = {
  id: "7",
  demo: { demoId: "retail", libraryVersion: "1.0.0" },
} as unknown as DemoDevice_Serialize;

beforeEach(() => {
  getDeviceConfig.mockReset();
  getDeviceConfig.mockResolvedValue({ groups: [], digitalInputs: [] });
});
afterEach(() => vi.clearAllMocks());

describe("useDeviceConfig", () => {
  it("reloads after an apply that keeps device id, demo and library version", async () => {
    let bump: () => void = () => {};
    function Wrapper({ children }: { children: ReactNode }) {
      const [rev, setRev] = useState(0);
      bump = () => setRev((n) => n + 1);
      return (
        <DeviceContext.Provider
          value={{
            device,
            reload: () => {},
            configRevision: rev,
            reloadConfig: () => setRev((n) => n + 1),
            setDevice: () => {},
          }}
        >
          {children}
        </DeviceContext.Provider>
      );
    }
    renderHook(() => useDeviceConfig(), { wrapper: Wrapper });
    await waitFor(() => expect(getDeviceConfig).toHaveBeenCalledTimes(1));
    act(() => bump());
    await waitFor(() => expect(getDeviceConfig).toHaveBeenCalledTimes(2));
  });
});
