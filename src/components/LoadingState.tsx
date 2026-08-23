import { useI18n } from "../lib/i18n";

export function LoadingState({ label }: { label?: string }) {
  const { t } = useI18n();
  return (
    <div className="loading-state" role="status">
      <span className="spinner" />
      <p>{label ?? t("loading.resources")}</p>
    </div>
  );
}
