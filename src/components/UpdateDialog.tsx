import { useEffect, useRef, useState } from "react";
import type { AppLanguage, UpdateCheckResult, UpdateDownloadStatus } from "../types/media";
import { formatBytes, formatDate } from "../lib/format";
import { useI18n, type MessageKey } from "../lib/i18n";
import { isolateModalSiblings } from "../lib/modalA11y";
import { Icon } from "./Icon";

export type UpdateFailureAction = "download" | "install" | null;

interface UpdateDialogProps {
  open: boolean;
  checkResult: UpdateCheckResult | null;
  downloadStatus: UpdateDownloadStatus;
  failureAction: UpdateFailureAction;
  confirmInstallRequest: number;
  onClose: () => void;
  onDownload: (version: string) => void;
  onInstall: (version: string) => void;
  onRetry: () => void;
}

function releaseNotesFor(result: UpdateCheckResult, language: AppLanguage): string {
  const notes = result.update?.releaseNotes;
  if (!notes) return "";
  return notes[language]?.trim()
    || notes["zh-CN"]?.trim()
    || notes["en-US"]?.trim()
    || Object.values(notes).find((value) => value?.trim())?.trim()
    || "";
}

function failureKey(action: UpdateFailureAction): MessageKey {
  return action === "install" ? "error.updateInstallFailed" : "error.updateDownloadFailed";
}

export function UpdateDialog({ open, checkResult, downloadStatus, failureAction, confirmInstallRequest, onClose, onDownload, onInstall, onRetry }: UpdateDialogProps) {
  const { language, t } = useI18n();
  const [confirming, setConfirming] = useState(false);
  const backdropRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const update = checkResult?.update ?? null;
  const visible = open && Boolean(checkResult && update);
  const busy = downloadStatus.phase === "CHECKING" || downloadStatus.phase === "DOWNLOADING" || downloadStatus.phase === "APPLYING";
  const busyRef = useRef(busy);
  const totalBytes = downloadStatus.totalBytes ?? (update && downloadStatus.version === update.version ? update.downloadSize : null);
  const progress = totalBytes && totalBytes > 0
    ? Math.min(100, Math.max(0, Math.round((downloadStatus.downloadedBytes / totalBytes) * 100)))
    : null;
  const notes = checkResult ? releaseNotesFor(checkResult, language) : "";
  busyRef.current = busy;

  useEffect(() => {
    if (!visible) setConfirming(false);
    else if (confirmInstallRequest > 0) setConfirming(true);
  }, [confirmInstallRequest, visible]);

  useEffect(() => {
    if (!visible) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    const restoreBackground = isolateModalSiblings(backdropRef.current);
    const focusable = () => Array.from(dialog?.querySelectorAll<HTMLElement>("button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])") ?? []);
    const frame = window.requestAnimationFrame(() => focusable()[0]?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busyRef.current) {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusable();
      if (items.length === 0) {
        event.preventDefault();
        dialog?.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", onKeyDown);
      restoreBackground();
      previous?.focus();
    };
  }, [onClose, visible]);

  useEffect(() => {
    if (!visible || downloadStatus.phase !== "APPLYING") return;
    const frame = window.requestAnimationFrame(() => dialogRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [downloadStatus.phase, visible]);

  if (!visible || !checkResult || !update) return null;
  return (
    <div className="modal-backdrop" onPointerDown={(event) => { if (event.currentTarget === event.target && !busy) onClose(); }} ref={backdropRef} role="presentation">
      <section aria-describedby="update-dialog-description" aria-labelledby="update-dialog-title" aria-modal="true" className="update-dialog" ref={dialogRef} role="dialog" tabIndex={-1}>
        <header className="modal-header">
          <span className="modal-heading-icon"><Icon name="download" /></span>
          <div><p className="eyebrow">{t("update.eyebrow")}</p><h2 id="update-dialog-title">{t("update.dialogTitle")}</h2><p id="update-dialog-description">{t("update.dialogDescription")}</p></div>
          <button aria-label={t("common.close")} className="modal-close" disabled={busy} onClick={onClose} title={t("common.close")} type="button"><Icon name="close" /></button>
        </header>
        <div className="update-dialog-body">
          <div className="update-release-overview">
            <div><span>{t("settings.currentVersion")}</span><strong>{checkResult.currentVersion}</strong></div>
            <Icon name="chevron" />
            <div><span>{t("update.availableVersion")}</span><strong>{update.version}</strong></div>
            <small>{t("update.publishedAt")} {formatDate(update.publishedAt)} · {formatBytes(update.downloadSize)}</small>
          </div>

          <section aria-labelledby="update-release-notes-title" className="update-release-notes" tabIndex={0}>
            <h3 id="update-release-notes-title">{t("update.releaseNotes")}</h3>
            <p>{notes || t("update.noReleaseNotes")}</p>
          </section>

          {downloadStatus.phase === "DOWNLOADING" && <div aria-live="polite" className="update-download-progress">
            <div><span>{t("update.downloading")}</span><strong>{progress == null ? t("update.preparingDownload") : `${progress}%`}</strong></div>
            {totalBytes
              ? <progress aria-label={t("update.downloadProgressAria")} max={totalBytes} value={Math.min(downloadStatus.downloadedBytes, totalBytes)} />
              : <progress aria-label={t("update.downloadProgressAria")} />}
            <small>{totalBytes ? t("update.downloadProgress", { downloaded: formatBytes(downloadStatus.downloadedBytes), total: formatBytes(totalBytes) }) : t("update.downloadedAmount", { downloaded: formatBytes(downloadStatus.downloadedBytes) })}</small>
          </div>}

          {downloadStatus.phase === "FAILED" && <div className="update-error" role="alert"><Icon name="warning" /><span><strong>{t("update.operationFailed")}</strong><small>{t(failureKey(failureAction))}</small></span></div>}

          {!confirming && <div className="update-actions">
            {downloadStatus.phase === "FAILED" && <button className="button secondary" onClick={onRetry} type="button"><Icon name="refresh" />{t("common.retry")}</button>}
            {downloadStatus.phase === "IDLE" && <button aria-label={t("update.downloadAria", { version: update.version })} className="button primary" onClick={() => onDownload(update.version)} title={t("update.downloadAria", { version: update.version })} type="button"><Icon name="download" />{t("update.download")}</button>}
            {downloadStatus.phase === "DOWNLOADING" && <button className="button primary is-busy" disabled type="button"><Icon name="refresh" />{t("update.downloading")}</button>}
            {downloadStatus.phase === "READY" && <button aria-label={t("update.installAria", { version: update.version })} className="button primary" disabled={!downloadStatus.canInstall} onClick={() => setConfirming(true)} title={t("update.installAria", { version: update.version })} type="button"><Icon name="refresh" />{t("update.install")}</button>}
            {downloadStatus.phase === "APPLYING" && <button className="button primary is-busy" disabled type="button"><Icon name="refresh" />{t("update.installing")}</button>}
          </div>}

          {confirming && downloadStatus.phase === "READY" && <div className="update-install-confirm" role="alert">
            <span><Icon name="warning" /></span>
            <div><strong>{t("update.installConfirmTitle")}</strong><p>{t("update.installConfirmDescription", { version: update.version })}</p></div>
            <div className="dialog-actions"><button className="button ghost" onClick={() => setConfirming(false)} type="button">{t("common.cancel")}</button><button className="button primary" disabled={!downloadStatus.canInstall} onClick={() => { setConfirming(false); onInstall(update.version); }} type="button"><Icon name="refresh" />{t("update.installAndRestart")}</button></div>
          </div>}
        </div>
        <footer className="modal-footer update-dialog-footer"><span><Icon name="shield" />{t("update.footer")}</span><button disabled={busy} onClick={onClose} type="button">{t("update.later")}</button></footer>
      </section>
    </div>
  );
}
