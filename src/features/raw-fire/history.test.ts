import { describe, expect, it } from "vitest";

import type { FiredRecord } from "@/lib/api";

import { describeFired, upsertHistory } from "./history";

const rec = (id: number, request: FiredRecord["request"]): FiredRecord => ({
  id,
  deviceId: "d",
  firedAt: "2026-10-12T12:00:00Z",
  request,
  status: 201,
  createdId: null,
});

describe("raw history", () => {
  it("describes each request kind", () => {
    expect(
      describeFired(
        rec(1, { kind: "measurement", type: "t", fragment: "f", series: "s", value: 3, unit: "C" }),
      ),
    ).toBe("t · f.s = 3 C");
    expect(describeFired(rec(2, { kind: "alarm", type: "a", severity: "MAJOR", text: "x" }))).toBe(
      "a · MAJOR",
    );
    expect(describeFired(rec(3, { kind: "event", type: "e", text: "x", extraJson: null }))).toBe(
      "e",
    );
    expect(describeFired(rec(4, { kind: "json", target: "alarm", body: "{}" }))).toBe(
      "alarm · JSON",
    );
  });

  it("upserts newest first, replaces by id and caps the length", () => {
    const e = (id: number) => rec(id, { kind: "event", type: "e", text: "x", extraJson: null });
    let list: FiredRecord[] = [];
    for (let i = 1; i <= 4; i++) list = upsertHistory(list, e(i), 3);
    expect(list.map((r) => r.id)).toEqual([4, 3, 2]);
    list = upsertHistory(list, { ...e(3), status: 500 }, 3);
    expect(list.map((r) => r.id)).toEqual([4, 3, 2]);
    expect(list[1]?.status).toBe(500);
  });
});
