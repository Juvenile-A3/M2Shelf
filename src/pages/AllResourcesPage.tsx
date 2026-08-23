import { useMemo } from "react";
import type { AllResourcesResult, CollectionSort, MediaNode, ViewMode } from "../types/media";
import { EmptyState } from "../components/EmptyState";
import { Icon } from "../components/Icon";
import { LoadingState } from "../components/LoadingState";
import { PosterGrid } from "../components/PosterGrid";
import { SelectionToolbar } from "../components/SelectionToolbar";
import { TagFilter } from "../components/TagFilter";
import { compareMediaNodes, nodeDisplayTitle } from "../lib/format";
import { useI18n } from "../lib/i18n";

interface AllResourcesPageProps {
  data: AllResourcesResult | null;
  loading: boolean;
  viewMode: ViewMode;
  onViewMode: (mode: ViewMode) => void;
  filter: string;
  onFilter: (value: string) => void;
  tagFilterId: number | null;
  onTagFilter: (tagId: number | null) => void;
  sort: CollectionSort;
  onSort: (value: CollectionSort) => void;
  onOpenNode: (node: MediaNode) => void;
  onMenu: (event: React.MouseEvent, node: MediaNode) => void;
  onBangumi: (node: MediaNode) => void;
  onRetryCover: (node: MediaNode) => void;
  onScan: () => void;
  onAddRoot: () => void;
  onSearch: (query: string) => void;
  coverRevision: number;
  editMode: boolean;
  selectedNodeIds: ReadonlySet<number>;
  matchBusy: boolean;
  onEditMode: (active: boolean) => void;
  onToggleSelection: (node: MediaNode) => void;
  onSelectAll: (nodeIds: number[]) => void;
  onClearSelection: () => void;
  onBatchTags: () => void;
  onBatchFavorites: () => void;
  onBatchMenu: (event: React.MouseEvent<HTMLButtonElement>) => void;
  onMatch: (nodeIds: number[] | null, rematchExisting: boolean) => void;
}

export function AllResourcesPage({ data, loading, viewMode, onViewMode, filter, onFilter, tagFilterId, onTagFilter, sort, onSort, onOpenNode, onMenu, onBangumi, onRetryCover, onScan, onAddRoot, onSearch, coverRevision, editMode, selectedNodeIds, matchBusy, onEditMode, onToggleSelection, onSelectAll, onClearSelection, onBatchTags, onBatchFavorites, onBatchMenu, onMatch }: AllResourcesPageProps) {
  const { language, number, t } = useI18n();
  const effectiveTagFilterId = tagFilterId != null && data?.nodes.some((node) => node.userTags.some((tag) => tag.id === tagFilterId))
    ? tagFilterId
    : null;
  const nodes = useMemo(() => {
    const query = filter.trim().toLocaleLowerCase();
    return [...(data?.nodes ?? [])]
      .filter((node) => {
        const title = nodeDisplayTitle(node, language).toLocaleLowerCase(language);
        return !query
          || title.includes(query)
          || node.displayName.toLocaleLowerCase().includes(query)
          || node.folderName.toLocaleLowerCase().includes(query)
          || node.userTags.some((tag) => tag.name.toLocaleLowerCase().includes(query));
      })
      .filter((node) => effectiveTagFilterId == null || node.userTags.some((tag) => tag.id === effectiveTagFilterId))
      .sort((a, b) => compareMediaNodes(a, b, sort, language));
  }, [data?.nodes, effectiveTagFilterId, filter, language, sort]);
  const hasActiveFilter = Boolean(filter.trim()) || effectiveTagFilterId != null;

  if (loading && !data) return <LoadingState label={t("all.loading")} />;
  if (!data) return <EmptyState eyebrow={t("all.eyebrow")} title={t("all.emptyTitle")} description={t("all.emptyDescription")} action={<button className="button primary" onClick={onAddRoot} type="button"><Icon name="plus" />{t("app.addMediaDirectory")}</button>} />;

  return (
    <section className="browse-page all-resources-page">
      <header className="page-toolbar">
        <div className="toolbar-topline">
          <span className="all-resources-location"><Icon name="archive" />{t("all.crossLibrary")}</span>
          <div className="toolbar-actions">
            <button aria-pressed={editMode} className={`button secondary edit-mode-button ${editMode ? "is-active" : ""}`} onClick={() => onEditMode(!editMode)} type="button"><Icon name="edit" />{editMode ? t("selection.exit") : t("selection.editMode")}</button>
            <button className="button secondary scan-button" disabled={matchBusy || (editMode && selectedNodeIds.size === 0)} onClick={() => onMatch(editMode ? [...selectedNodeIds] : null, editMode)} type="button"><Icon name="bangumi" />{matchBusy ? t("selection.matching") : editMode ? t("selection.rematchSelectedCount", { count: number(selectedNodeIds.size) }) : t("all.matchExisting")}</button>
            <button className="button secondary scan-button" disabled={matchBusy} onClick={onScan} type="button"><Icon name="refresh" />{t("all.scanAndMatch")}</button>
          </div>
        </div>
        <div className="page-title-row">
          <div><h1>{t("all.title")}</h1><p>{t("all.description")}</p></div>
          <div className="browse-controls">
            <label className="search-field"><Icon name="search" /><input aria-label={t("all.filterAria")} onChange={(event) => onFilter(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && filter.trim()) onSearch(filter.trim()); }} placeholder={t("all.filterPlaceholder")} value={filter} />{filter && <button aria-label={t("all.clearFilter")} onClick={() => onFilter("")} type="button"><Icon name="close" /></button>}</label>
            <TagFilter nodes={data.nodes} value={tagFilterId} onChange={onTagFilter} />
            <label className="sort-field"><span className="sr-only">{t("common.sort")}</span><select aria-label={t("common.sort")} onChange={(event) => onSort(event.target.value as CollectionSort)} value={sort}><option value="title-asc">{t("sort.titleAsc")}</option><option value="title-desc">{t("sort.titleDesc")}</option><option value="added-desc">{t("sort.addedDesc")}</option><option value="added-asc">{t("sort.addedAsc")}</option></select></label>
            <div className="view-toggle" aria-label={t("common.displayMode")}><button className={viewMode === "grid" ? "is-active" : ""} onClick={() => onViewMode("grid")} title={t("common.grid")} type="button"><Icon name="grid" /></button><button className={viewMode === "list" ? "is-active" : ""} onClick={() => onViewMode("list")} title={t("common.list")} type="button"><Icon name="list" /></button></div>
          </div>
        </div>
      </header>
      <div className="page-content">
        {editMode && <SelectionToolbar selectedCount={selectedNodeIds.size} visibleCount={nodes.length} busy={matchBusy} onSelectAll={() => onSelectAll(nodes.map((node) => node.id))} onClear={onClearSelection} onTags={onBatchTags} onFavorites={onBatchFavorites} onMore={onBatchMenu} onRematch={() => onMatch([...selectedNodeIds], true)} onExit={() => onEditMode(false)} />}
        <div className="all-resources-summary"><strong>{t("app.projectCount", { count: number(nodes.length) })}</strong>{hasActiveFilter && <span>{t("all.totalCount", { count: number(data.totalCount) })}</span>}</div>
        {nodes.length > 0 && <PosterGrid nodes={nodes} viewMode={viewMode} onOpen={onOpenNode} onMenu={onMenu} onBangumi={onBangumi} onRetryCover={onRetryCover} coverRevision={coverRevision} editMode={editMode} selectedNodeIds={selectedNodeIds} onSelect={onToggleSelection} />}
        {nodes.length === 0 && <EmptyState compact icon={hasActiveFilter ? "search" : "folder"} title={hasActiveFilter ? t("all.noMatch") : t("all.temporarilyEmpty")} description={hasActiveFilter ? t("all.noMatchDescription") : t("all.emptyScanDescription")} />}
      </div>
    </section>
  );
}
