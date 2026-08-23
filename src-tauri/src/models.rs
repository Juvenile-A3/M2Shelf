use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeType {
    AutoWork,
    Work,
    Container,
    Mixed,
    Ignored,
}

impl NodeType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::AutoWork => "AUTO_WORK",
            Self::Work => "WORK",
            Self::Container => "CONTAINER",
            Self::Mixed => "MIXED",
            Self::Ignored => "IGNORED",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "AUTO_WORK" => Self::AutoWork,
            "WORK" => Self::Work,
            "MIXED" => Self::Mixed,
            "IGNORED" => Self::Ignored,
            _ => Self::Container,
        }
    }

    pub fn is_work(self) -> bool {
        matches!(self, Self::AutoWork | Self::Work)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoverSource {
    Bangumi,
    Manual,
    Placeholder,
}

impl CoverSource {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Bangumi => "BANGUMI",
            Self::Manual => "MANUAL",
            Self::Placeholder => "PLACEHOLDER",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "BANGUMI" => Self::Bangumi,
            "MANUAL" => Self::Manual,
            _ => Self::Placeholder,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ViewMode {
    Grid,
    List,
}

impl ViewMode {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Grid => "GRID",
            Self::List => "LIST",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CollectionSort {
    #[default]
    TitleAsc,
    TitleDesc,
    AddedDesc,
    AddedAsc,
}

impl CollectionSort {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::TitleAsc => "title-asc",
            Self::TitleDesc => "title-desc",
            Self::AddedDesc => "added-desc",
            Self::AddedAsc => "added-asc",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "title-desc" => Self::TitleDesc,
            "added-desc" => Self::AddedDesc,
            "added-asc" => Self::AddedAsc,
            _ => Self::TitleAsc,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CollectionSortScope {
    All,
    Browse,
    Favorites,
}

impl CollectionSortScope {
    pub fn setting_key(self) -> &'static str {
        match self {
            Self::All => "collection_sort_all",
            Self::Browse => "collection_sort_browse",
            Self::Favorites => "collection_sort_favorites",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSortPreferences {
    pub all: CollectionSort,
    pub browse: CollectionSort,
    pub favorites: CollectionSort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRoot {
    pub id: i64,
    pub path: String,
    pub display_name: String,
    pub created_at: String,
    pub last_scan_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataBinding {
    pub id: i64,
    pub node_id: i64,
    pub provider: String,
    pub provider_subject_id: i64,
    pub provider_title: String,
    pub provider_title_cn: Option<String>,
    pub provider_title_en: Option<String>,
    pub provider_title_ja: Option<String>,
    pub provider_title_ko: Option<String>,
    pub provider_date: Option<String>,
    pub provider_image_url: Option<String>,
    pub bound_at: String,
    pub updated_at: String,
    pub cover_cache_path: Option<String>,
    pub cover_download_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserTag {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserTagMembership {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub assigned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteFolder {
    pub id: i64,
    pub name: String,
    pub item_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchMutationResult {
    pub requested: u64,
    pub updated: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaNode {
    pub id: i64,
    pub library_root_id: i64,
    pub parent_node_id: Option<i64>,
    pub absolute_path: String,
    pub folder_name: String,
    pub display_name: String,
    pub node_type: NodeType,
    pub manual_type_override: bool,
    pub cover_source: CoverSource,
    pub cover_cache_path: Option<String>,
    pub direct_video_count: i64,
    pub child_media_branch_count: i64,
    pub total_video_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_seen_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<MetadataBinding>,
    #[serde(default)]
    pub user_tags: Vec<UserTag>,
}

impl MediaNode {
    pub fn can_bind_bangumi(&self) -> bool {
        self.node_type.is_work()
            || (self.node_type == NodeType::Container && self.total_video_count > 0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFile {
    pub id: i64,
    pub node_id: i64,
    pub absolute_path: String,
    pub file_name: String,
    pub extension: String,
    pub file_size: i64,
    pub modified_at: String,
    pub duration_ms: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub codec: Option<String>,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResourceType {
    Document,
    Image,
    Audio,
    Subtitle,
    Archive,
    Font,
    Playlist,
    Other,
}

impl ResourceType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Document => "DOCUMENT",
            Self::Image => "IMAGE",
            Self::Audio => "AUDIO",
            Self::Subtitle => "SUBTITLE",
            Self::Archive => "ARCHIVE",
            Self::Font => "FONT",
            Self::Playlist => "PLAYLIST",
            Self::Other => "OTHER",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "DOCUMENT" => Self::Document,
            "IMAGE" => Self::Image,
            "AUDIO" => Self::Audio,
            "SUBTITLE" => Self::Subtitle,
            "ARCHIVE" => Self::Archive,
            "FONT" => Self::Font,
            "PLAYLIST" => Self::Playlist,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceFile {
    pub id: i64,
    pub node_id: i64,
    pub absolute_path: String,
    pub file_name: String,
    pub extension: String,
    pub file_size: i64,
    pub modified_at: String,
    pub resource_type: ResourceType,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBootstrap {
    pub name: &'static str,
    pub version: &'static str,
    pub database_url: &'static str,
    pub build_date: &'static str,
    pub architecture: &'static str,
    pub website_url: &'static str,
    pub x_url: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreadcrumbItem {
    pub id: i64,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseResult {
    pub root: LibraryRoot,
    pub breadcrumbs: Vec<BreadcrumbItem>,
    pub nodes: Vec<MediaNode>,
    pub media_files: Vec<MediaFile>,
    pub resource_files: Vec<ResourceFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllResourcesResult {
    pub nodes: Vec<MediaNode>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentlyWatchedEntry {
    pub node: MediaNode,
    pub watched_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDetail {
    pub node: MediaNode,
    pub children: Vec<MediaNode>,
    pub media_files: Vec<MediaFile>,
    pub resource_files: Vec<ResourceFile>,
    pub breadcrumbs: Vec<BreadcrumbItem>,
    pub binding: Option<MetadataBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BangumiSearchPrefill {
    pub original_name: String,
    pub extracted_name: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchHitKind {
    Node,
    MediaFile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub kind: SearchHitKind,
    pub node: MediaNode,
    pub media_file: Option<MediaFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BangumiSubject {
    pub subject_id: i64,
    pub title: String,
    pub title_cn: Option<String>,
    #[serde(default)]
    pub title_en: Option<String>,
    #[serde(default)]
    pub title_ja: Option<String>,
    #[serde(default)]
    pub title_ko: Option<String>,
    /// Official aliases used only while ranking provider candidates. Confirmed bindings keep the
    /// existing stable multilingual columns, so this does not change the SQLite schema.
    #[serde(default)]
    pub match_aliases: Vec<String>,
    pub date: Option<String>,
    pub image_url: Option<String>,
    pub summary: Option<String>,
    pub subject_type: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScanStatus {
    Idle,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScanPhase {
    #[default]
    Scanning,
    AutoMatching,
}

impl ScanStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Running => "RUNNING",
            Self::Cancelling => "CANCELLING",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::Failed => "FAILED",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scan_id: String,
    pub root_id: i64,
    pub current_path: String,
    pub folders_scanned: u64,
    pub videos_found: u64,
    pub status: ScanStatus,
    pub errors: u64,
    pub message: Option<String>,
    #[serde(default)]
    pub phase: ScanPhase,
    #[serde(default)]
    pub auto_match_current: u64,
    #[serde(default)]
    pub auto_match_total: u64,
    #[serde(default)]
    pub auto_match_matched: u64,
    #[serde(default)]
    pub auto_match_pending: u64,
    #[serde(default)]
    pub auto_match_unmatched: u64,
    #[serde(default)]
    pub auto_match_errors: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CoverMatchReport {
    pub examined: u64,
    pub matched: u64,
    pub pending: u64,
    pub unmatched: u64,
    pub errors: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStarted {
    pub scan_id: String,
}

pub type RebuildResult = ScanStarted;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub mpv_path: Option<String>,
    pub default_view_mode: ViewMode,
    pub video_extensions: Vec<String>,
    pub bangumi_search_enabled: bool,
    pub cover_cache_directory: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
}

/// Application-owned logical dimensions for the main window. This is intentionally separate
/// from `AppSettings`: frontend settings snapshots must not overwrite native window lifecycle
/// state with a stale value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

fn default_language() -> String {
    "zh-CN".into()
}

fn default_theme() -> String {
    "system".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub file_count: u64,
    pub total_bytes: u64,
    pub cache_directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTestResult {
    pub ok: bool,
    pub message: String,
    pub version: Option<String>,
}
