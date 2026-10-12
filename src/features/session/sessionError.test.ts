import { describe, expect, it } from "vitest";

import { sessionErrorKey, sessionErrorText } from "./sessionError";

describe("sessionErrorKey", () => {
  it("maps the fatal cases", () => {
    const e = (code: string, message: string, fatal = true) => ({ code, message, fatal });
    expect(sessionErrorKey(e("deviceLocked", "x"))).toBe("session.lockTakenOver");
    expect(
      sessionErrorKey(e("internal", "the outbound worker stopped; the session cannot send")),
    ).toBe("session.workerStopped");
    expect(sessionErrorKey(e("internal", "the simulation session crashed"))).toBe(
      "session.crashed",
    );
    expect(sessionErrorKey(e("internal", "other"))).toBeNull();
    expect(sessionErrorKey(e("deviceLocked", "x", false))).toBeNull();
  });
});

describe("sessionErrorText", () => {
  const e = (code: string, message: string, fatal = false) => ({ code, message, fatal });
  it("never selects a text that needs values the event does not carry", () => {
    // `errors.http` needs {{status}} / {{message}}: use the generic line with the message.
    expect(sessionErrorText(e("http", "HTTP 503: down"))).toEqual({
      key: "session.errorGeneric",
      values: { code: "http", message: "HTTP 503: down" },
    });
    expect(sessionErrorText(e("deviceLocked", "x")).key).toBe("session.errorGeneric");
  });
  it("uses the code text where details are enough", () => {
    expect(sessionErrorText(e("network", "timeout"))).toEqual({
      key: "errors.network",
      values: { details: "timeout" },
    });
    expect(sessionErrorText(e("internal", "x")).key).toBe("errors.internal");
    expect(sessionErrorText(e("deviceLocked", "x", true)).key).toBe("session.lockTakenOver");
  });
});
