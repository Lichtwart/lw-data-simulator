import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import i18n from "@/i18n";
import type { OperationPrompt } from "@/lib/api";

import { OperationPromptCard } from "./OperationPrompts";

const prompt: OperationPrompt = {
  revision: 1,
  deviceId: "d1",
  operationId: "op1",
  kind: "switchGroupMode",
  summary: "Group 3 → Auto",
  detail: { kind: "groupMode", group: 3, mode: "auto" },
  by: "anna",
  receivedAt: "2026-10-12T12:00:00Z",
  expiresAt: "2026-10-12T12:01:00Z",
};

afterEach(cleanup);

describe("OperationPromptCard", () => {
  it("shows summary, user and the remaining seconds, and answers", async () => {
    await i18n.changeLanguage("en");
    const onAnswer = vi.fn();
    render(
      <OperationPromptCard
        prompt={prompt}
        now={Date.parse("2026-10-12T12:00:20Z")}
        onAnswer={onAnswer}
      />,
    );
    expect(screen.getByText("Group 3 → Auto")).toBeTruthy();
    expect(screen.getByText("by anna")).toBeTruthy();
    expect(screen.getByText("Aborted automatically in 40 s")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    fireEvent.click(screen.getByRole("button", { name: "Abort" }));
    fireEvent.change(screen.getByLabelText("Reason (optional)"), {
      target: { value: "busy" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Abort" }));
    expect(onAnswer.mock.calls).toEqual([["confirm"], ["abort", ""], ["abort", "busy"]]);
  });
});
