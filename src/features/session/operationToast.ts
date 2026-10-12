import type { OperationResolved } from "@/lib/api";

export interface OperationToast {
  level: "info" | "warning" | "error";
  key:
    | "operations.timedOut"
    | "operations.offline"
    | "operations.failed"
    | "operations.withdrawn"
    | "operations.autoAccepted";
  description?: string;
}

/** The toast for a resolved operation, or `null` when nothing needs saying (confirmed, stop). */
export function operationToast(
  r: Pick<OperationResolved, "resolution" | "reason">,
): OperationToast | null {
  switch (r.resolution) {
    case "timedOut":
      return { level: "warning", key: "operations.timedOut" };
    case "simulatorOffline":
      return { level: "warning", key: "operations.offline" };
    case "error":
      return { level: "error", key: "operations.failed", description: r.reason ?? undefined };
    case "withdrawn":
      return { level: "info", key: "operations.withdrawn" };
    case "autoAccepted":
      return { level: "info", key: "operations.autoAccepted" };
    default:
      // confirmed / aborted / autoAnswered, and `stopped` (a deliberate stop is not an error)
      return null;
  }
}
