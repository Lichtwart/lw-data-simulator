import { describe, expect, it } from "vitest";

import { tenantLabel } from "./tenant";

describe("tenantLabel", () => {
  it("shows tenant id and host", () => {
    expect(
      tenantLabel({ key: "t1@x.example.com", tenantId: "t1", baseUrl: "https://x.example.com" }),
    ).toBe("t1 · x.example.com");
  });

  it("keeps a port", () => {
    expect(tenantLabel({ key: "t1@x:8443", tenantId: "t1", baseUrl: "https://x:8443" })).toBe(
      "t1 · x:8443",
    );
  });
});
