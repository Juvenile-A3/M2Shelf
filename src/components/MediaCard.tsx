import { memo, useEffect, useRef, useState } from "react";
import type { MediaNode, ViewMode } from "../types/media";
import { canBindBangumi, formatDate, nodeDisplayTitle } from "../lib/format";
import { Icon } from "./Icon";
import { useCoverDataUrl } from "../hooks/useCoverDataUrl";
import { useI18n } from "../lib/i18n";
import { shouldContainPosterArtwork } from "../lib/poster";

interface MediaCardProps {
  node: MediaNode;
  viewMode: ViewMode;
  onOpen: (node: MediaNode) => void;
  onMenu: (event: React.MouseEvent, node: MediaNode) => void;
  onBangumi: (node: MediaNode) => void;
  onRetryCover: (node: MediaNode) => void;
  coverRevision: number;
  watchedAt?: string;
  editMode?: boolean;
  selected?: boolean;
  onSelect?: (node: MediaNode) => void;
}

function MediaCardComponent({ node, viewMode, onOpen, onMenu, onBangumi, onRetryCover, coverRevision, watchedAt, editMode = false, selected = false, onSelect }: MediaCardProps) {
  const { t } = useI18n();
  const cardRef = useRef<HTMLElement>(null);
  const [coverRequested, setCoverRequested] = useState(false);
  const { coverUrl: cover, coverFailed: coverReadFailed, coverLoading } = useCoverDataUrl(node, coverRevision, coverRequested);
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => {
    setCoverRequested(false);
    const card = cardRef.current;
    if (!card || typeof IntersectionObserver === "undefined") {
      setCoverRequested(true);
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      setCoverRequested(true);
      observer.disconnect();
    }, { rootMargin: "700px 0px" });
    observer.observe(card);
    return () => observer.disconnect();
  }, [node.id]);
  useEffect(() => setImageFailed(false), [cover]);
  const videos = node.totalVideoCount ?? node.directVideoCount ?? 0;
  const container = node.nodeType === "CONTAINER" || node.nodeType === "MIXED";
  const systemTag = node.nodeType === "CONTAINER"
    ? { icon: "folder" as const, label: t("card.systemSeries") }
    : node.nodeType === "MIXED"
      ? { icon: "archive" as const, label: t("card.systemOtherResources") }
      : { icon: "work" as const, label: t("card.systemWork") };
  const bindable = canBindBangumi(node);
  const title = nodeDisplayTitle(node);
  const coverFailed = coverReadFailed || imageFailed;
  const coverError = node.binding != null && !coverLoading && (Boolean(node.binding.coverDownloadError) || !cover || coverFailed);

  return (
    <article
      ref={cardRef}
      className={`media-card media-card-${viewMode} ${watchedAt ? "has-watch-time" : ""} ${editMode ? "is-editing" : ""} ${selected ? "is-selected" : ""}`}
      onContextMenu={(event) => { event.preventDefault(); onMenu(event, node); }}
    >
      <button aria-pressed={editMode ? selected : undefined} className="media-card-open" onClick={() => editMode ? onSelect?.(node) : onOpen(node)} type="button">
        <span className={`cover-frame ${cover ? "has-cover" : ""} ${container ? "is-container" : ""}`}>
          {cover && !coverFailed ? <img alt={t("card.coverAlt", { title })} decoding="async" loading="lazy" onError={() => setImageFailed(true)} onLoad={(event) => event.currentTarget.classList.toggle("is-wide-artwork", shouldContainPosterArtwork(event.currentTarget.naturalWidth, event.currentTarget.naturalHeight))} src={cover} /> : (
            <span className="cover-placeholder">
              <span className="cover-art"><Icon name={container ? "folder-open" : "work"} /></span>
              <small>{coverError ? t("card.coverFailed") : container ? t("card.resourceContainer") : t("card.noCover")}</small>
            </span>
          )}
          <span className="type-pill system-tag"><Icon name={systemTag.icon} />{systemTag.label}</span>
          {editMode && <span aria-hidden="true" className="selection-indicator"><Icon name={selected ? "check" : "plus"} /></span>}
        </span>
        <span className="media-card-copy">
          <strong title={title}>{title}</strong>
          {watchedAt && <time className="media-card-watch-time" dateTime={watchedAt}>{t("recent.watchedAt", { time: formatDate(watchedAt) })}</time>}
          <small>
            {videos > 0 ? t("card.videoCount", { count: videos }) : container ? t("card.childCount", { count: node.childMediaBranchCount ?? 0 }) : t("card.awaitingScan")}
          </small>
          {viewMode === "list" && !node.binding && <span className="card-path">{node.absolutePath}</span>}
          {(node.userTags?.length ?? 0) > 0 && (
            <span aria-label={t("card.customTags")} className="media-card-tags">
              {node.userTags.map((tag) => <span className="user-tag-pill" key={tag.id}>{tag.name}</span>)}
            </span>
          )}
        </span>
      </button>
      {!editMode && bindable && !node.binding && (
        <button className="quick-bind" onClick={() => onBangumi(node)} title={t("card.searchCover")} type="button"><Icon name="plus" /><span>{t("provider.bangumi")}</span></button>
      )}
      {!editMode && bindable && node.binding && coverError && (
        <button className="quick-bind is-retry" onClick={() => onRetryCover(node)} title={t("card.retryCover")} type="button"><Icon name="refresh" /><span>{t("card.retryShort")}</span></button>
      )}
      {!editMode && <button className="card-menu" aria-label={t("card.moreActions", { title })} onClick={(event) => onMenu(event, node)} type="button"><Icon name="more" /></button>}
    </article>
  );
}

export const MediaCard = memo(MediaCardComponent);
