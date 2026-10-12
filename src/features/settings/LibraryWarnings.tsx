import { useTranslation } from "react-i18next";

/** Non-fatal problems found while loading the bundled library (nothing when there are none). */
export function LibraryWarnings({ warnings }: { warnings: readonly string[] }) {
  const { t } = useTranslation();
  if (warnings.length === 0) return null;
  return (
    <div
      role="status"
      className="flex flex-col gap-1 rounded-md border border-warning bg-warning-bg p-3 text-sm"
    >
      <p className="font-semibold text-strong">
        {t("settings.libraryWarnings", { count: warnings.length })}
      </p>
      <ul className="list-disc pl-5 font-mono text-xs">
        {warnings.map((w) => (
          <li key={w}>{w}</li>
        ))}
      </ul>
    </div>
  );
}
