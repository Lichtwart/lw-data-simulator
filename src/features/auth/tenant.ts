import type { Tenant } from "@/lib/api";

/** Short label of a saved tenant: `t12345 · x.example.com` (the URL without `https://`). */
export function tenantLabel(tenant: Tenant): string {
  return `${tenant.tenantId} · ${tenant.baseUrl.replace(/^https?:\/\//, "")}`;
}
