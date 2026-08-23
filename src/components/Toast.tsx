import { Icon } from "./Icon";
import { useI18n } from "../lib/i18n";

export type ToastKind = "success" | "error" | "info";

export interface ToastMessage {
  id: number;
  kind: ToastKind;
  text: string;
}

interface ToastStackProps {
  toasts: ToastMessage[];
  onDismiss: (id: number) => void;
}

export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  const { t } = useI18n();
  return (
    <div className="toast-stack" aria-live="polite">
      {toasts.map((toast) => (
        <div className={`toast toast-${toast.kind}`} key={toast.id}>
          <Icon name={toast.kind === "error" ? "warning" : toast.kind === "success" ? "check" : "info"} />
          <span>{toast.text}</span>
          <button aria-label={t("common.closeNotice")} onClick={() => onDismiss(toast.id)} type="button"><Icon name="close" /></button>
        </div>
      ))}
    </div>
  );
}
