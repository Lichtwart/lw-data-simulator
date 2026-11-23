import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "@/i18n";
import type { DemoDevice_Serialize } from "@/lib/api";

const previewApplyDemo = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api", async (orig) => {
  const real = await orig<typeof import("@/lib/api")>();
  return { ...real, isTauri: () => true, api: { previewApplyDemo } };
});
vi.mock("@/lib/library", () => ({
  loadLibrary: async () => ({
    demos: [{ id: "retail", name: { de: "Einzelhandel", en: "Retail" }, scenarios: [] }],
  }),
}));

import { DeviceContext } from "../devices/DeviceContext";
import { ApplyDemoCard } from "./ApplyDemoCard";

const device = {
  id: "7",
  demo: { demoId: "retail", libraryVersion: "1.0.0", language: "de" },
} as unknown as DemoDevice_Serialize;

function preview(languageChange: { from: string; to: string } | null) {
  return {
    itemCount: 3,
    slaves: [],
    floorplanConflicts: [],
    warnings: [],
    language: "de",
    languageChange,
  };
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  previewApplyDemo.mockReset();
});
afterEach(cleanup);

function renderCard() {
  render(
    <DeviceContext.Provider
      value={{
        device,
        reload: () => {},
        configRevision: 0,
        reloadConfig: () => {},
        setDevice: () => {},
      }}
    >
      <ApplyDemoCard />
    </DeviceContext.Provider>,
  );
}

describe("ApplyDemoCard language", () => {
  it("previews with the device language and warns on a language change", async () => {
    previewApplyDemo.mockResolvedValue(preview({ from: "de", to: "en" }));
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Preview" }));
    expect(await screen.findByText(/changes from Deutsch to English/)).toBeTruthy();
    expect(screen.getByText(/Cumulocity starts new series/)).toBeTruthy();
    expect(previewApplyDemo.mock.calls[0]?.[1]).toMatchObject({ demoId: "retail", language: "de" });
  });

  it("shows no warning without a language change", async () => {
    previewApplyDemo.mockResolvedValue(preview(null));
    renderCard();
    fireEvent.click(await screen.findByRole("button", { name: "Preview" }));
    await waitFor(() => expect(screen.getByText(/3 items will be set up/)).toBeTruthy());
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
