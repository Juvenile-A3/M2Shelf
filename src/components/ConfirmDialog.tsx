import { Icon } from "./Icon";
import { useI18n } from "../lib/i18n";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel?: string;
  destructive?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

export function ConfirmDialog({ open, title, description, confirmLabel, destructive, busy, onConfirm, onClose }: ConfirmDialogProps) {
  const { t } = useI18n();
  if (!open) return null;
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section aria-labelledby="confirm-title" aria-modal="true" className="confirm-dialog" role="dialog">
        <button className="modal-close" aria-label={t("common.close")} onClick={onClose} type="button"><Icon name="close" /></button>
        <span className={`dialog-symbol ${destructive ? "is-danger" : ""}`}><Icon name={destructive ? "warning" : "info"} /></span>
        <h2 id="confirm-title">{title}</h2>
        <p>{description}</p>
        <div className="dialog-actions">
          <button className="button ghost" disabled={busy} onClick={onClose} type="button">{t("common.cancel")}</button>
          <button className={`button ${destructive ? "danger" : "primary"}`} disabled={busy} onClick={onConfirm} type="button">
            {busy ? t("common.processing") : confirmLabel ?? t("common.confirm")}
          </button>
        </div>
      </section>
    </div>
  );
}
