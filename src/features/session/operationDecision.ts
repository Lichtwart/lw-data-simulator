import type { OperationDecision } from "@/lib/api";

/** The decision to send; an empty abort reason is left out (the backend then uses its default). */
export function operationDecision(
  decision: "confirm" | "abort",
  reason?: string,
): OperationDecision {
  if (decision === "confirm") return { decision: "confirm" };
  const text = reason?.trim();
  return text ? { decision: "abort", reason: text } : { decision: "abort", reason: null };
}
