import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { LibraryWarnings } from "./LibraryWarnings";

afterEach(cleanup);

describe("LibraryWarnings", () => {
  it("lists the load warnings (F17)", async () => {
    await i18n.changeLanguage("en");
    render(<LibraryWarnings warnings={["floorplan x is not used by a demo"]} />);
    expect(screen.getByText("Library warnings (1)")).toBeTruthy();
    expect(screen.getByText("floorplan x is not used by a demo")).toBeTruthy();
  });

  it("renders nothing without warnings", () => {
    const { container } = render(<LibraryWarnings warnings={[]} />);
    expect(container.innerHTML).toBe("");
  });
});
