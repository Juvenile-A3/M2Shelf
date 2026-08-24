use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use tauri::{AppHandle, Emitter};

use crate::{
    auto_match,
    db::{AppResult, Database},
    models::{LibraryRoot, NodeType, ResourceType, ScanPhase, ScanProgress, ScanStatus},
};

#[derive(Debug, Clone)]
pub struct ScanTarget {
    pub root: LibraryRoot,
    pub path: PathBuf,
    pub parent_node_id: Option<i64>,
}

#[derive(Clone)]
pub struct ScanControl {
    pub scan_id: String,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<Mutex<ScanProgress>>,
}

impl ScanControl {
    pub fn progress(&self) -> ScanProgress {
        self.progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ChildMediaSummary {
    pub branch_count: i64,
    pub supplementary_branch_count: i64,
    pub total_videos: i64,
}

#[derive(Debug)]
enum ScanAbort {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanEntryPath {
    /// Path kept in SQLite and shown to the user. This preserves the spelling of the configured
    /// Library Root instead of leaking Windows extended-length canonical path prefixes.
    logical: PathBuf,
    /// Path used for filesystem access. It is canonicalized and checked against the canonical
    /// Library Root immediately before a directory or file is read.
    filesystem: PathBuf,
}

impl From<String> for ScanAbort {
    fn from(value: String) -> Self {
        Self::Failed(value)
    }
}

#[cfg(test)]
pub fn run_scan(
    app: Option<&AppHandle>,
    database: &Database,
    targets: Vec<ScanTarget>,
    control: &ScanControl,
    extensions: &[String],
) {
    run_scan_with_auto_match(app, database, targets, control, extensions, None);
}

pub fn run_scan_with_auto_match(
    app: Option<&AppHandle>,
    database: &Database,
    targets: Vec<ScanTarget>,
    control: &ScanControl,
    extensions: &[String],
    auto_match_cache_root: Option<Result<PathBuf, String>>,
) {
    let extension_set = extensions
        .iter()
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let token = format!(
        "{}:{}",
        Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
        control.scan_id
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scan_inner(app, database, &targets, control, &extension_set, &token)
    }))
    .unwrap_or_else(|_| Err(ScanAbort::Failed("扫描线程发生内部错误。".into())));

    let auto_match_report = if result.is_ok() && !control.cancel.load(Ordering::Relaxed) {
        auto_match_cache_root.as_ref().map(|cache_root| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                auto_match::run_auto_match(
                    database,
                    &targets,
                    cache_root
                        .as_ref()
                        .map(PathBuf::as_path)
                        .map_err(String::as_str),
                    |current, total, node| {
                        update_auto_match_progress(app, control, current, total, node)
                    },
                    || control.cancel.load(Ordering::Relaxed),
                )
            }))
            .unwrap_or(auto_match::AutoMatchReport {
                errors: 1,
                ..auto_match::AutoMatchReport::default()
            })
        })
    } else {
        None
    };

    let mut progress = control
        .progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(report) = auto_match_report {
        if report.examined > 0 || report.errors > 0 {
            progress.phase = ScanPhase::AutoMatching;
        }
        progress.auto_match_matched = report.matched as u64;
        progress.auto_match_pending = report.pending as u64;
        progress.auto_match_unmatched = report.unmatched as u64;
        progress.auto_match_errors = report.errors as u64;
    }
    match result {
        Ok(()) if control.cancel.load(Ordering::Relaxed) => {
            progress.status = ScanStatus::Cancelled;
            progress.message = Some("扫描已停止；已完成的索引结果已保留。".into());
        }
        Ok(()) => {
            progress.status = ScanStatus::Completed;
            progress.message = Some(match auto_match_report {
                Some(report) if report.examined > 0 || report.errors > 0 => format!(
                    "扫描完成；自动匹配 {} 项，待确认 {} 项，未匹配 {} 项，{} 项稍后重试。",
                    report.matched, report.pending, report.unmatched, report.errors
                ),
                _ => "扫描完成。".into(),
            });
        }
        Err(ScanAbort::Cancelled) => {
            progress.status = ScanStatus::Cancelled;
            progress.message = Some("扫描已停止；已完成的索引结果已保留。".into());
        }
        Err(ScanAbort::Failed(message)) => {
            progress.status = ScanStatus::Failed;
            progress.message = Some(message);
            progress.errors += 1;
        }
    }
    let final_progress = progress.clone();
    drop(progress);
    for target in &targets {
        let mut root_progress = final_progress.clone();
        root_progress.root_id = target.root.id;
        let _ = database.finish_scan_run(&root_progress);
    }
    if let Some(app) = app {
        let _ = app.emit("scan-progress", &final_progress);
        let _ = app.emit("scan-completed", &final_progress);
    }
}

/// Matches Nodes already indexed in SQLite without walking or mutating the media filesystem.
/// Reusing `ScanControl` keeps long online runs visible, cancellable, and compatible with the
/// existing progress/completion event flow.
pub fn run_existing_content_match(
    app: Option<&AppHandle>,
    database: &Database,
    nodes: Vec<crate::models::MediaNode>,
    control: &ScanControl,
    cache_root: Result<PathBuf, String>,
    write_mode: auto_match::MatchWriteMode,
) {
    let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        auto_match::run_match_nodes(
            database,
            &nodes,
            cache_root
                .as_ref()
                .map(PathBuf::as_path)
                .map_err(String::as_str),
            write_mode,
            |current, total, node| update_auto_match_progress(app, control, current, total, node),
            || control.cancel.load(Ordering::Relaxed),
        )
    }))
    .unwrap_or(auto_match::AutoMatchReport {
        errors: 1,
        ..auto_match::AutoMatchReport::default()
    });

    let mut progress = control
        .progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    progress.phase = ScanPhase::AutoMatching;
    progress.auto_match_current = report.examined as u64;
    progress.auto_match_total = nodes.len() as u64;
    progress.auto_match_matched = report.matched as u64;
    progress.auto_match_pending = report.pending as u64;
    progress.auto_match_unmatched = report.unmatched as u64;
    progress.auto_match_errors = report.errors as u64;
    if control.cancel.load(Ordering::Relaxed) {
        progress.status = ScanStatus::Cancelled;
        progress.message = Some("现有资源匹配已停止；已完成的安全绑定保留。".into());
    } else {
        progress.status = ScanStatus::Completed;
        progress.message = Some(format!(
            "现有资源匹配完成；自动匹配 {} 项，待确认 {} 项，未匹配 {} 项，{} 项稍后重试。",
            report.matched, report.pending, report.unmatched, report.errors
        ));
    }
    let final_progress = progress.clone();
    drop(progress);
    if let Some(app) = app {
        let _ = app.emit("scan-progress", &final_progress);
        let _ = app.emit("scan-completed", &final_progress);
    }
}

fn update_auto_match_progress(
    app: Option<&AppHandle>,
    control: &ScanControl,
    current: usize,
    total: usize,
    node: &crate::models::MediaNode,
) {
    let snapshot = {
        let mut progress = control
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        progress.current_path = node.absolute_path.clone();
        progress.phase = ScanPhase::AutoMatching;
        progress.auto_match_current = current as u64;
        progress.auto_match_total = total as u64;
        progress.message = Some(format!("正在自动匹配封面与标题（{current}/{total}）…"));
        progress.clone()
    };
    if let Some(app) = app {
        let _ = app.emit("scan-progress", snapshot);
    }
}

/// Resolves a path before it is read and applies a component-aware Library Root boundary.
/// `Path::starts_with` compares path components, so a sibling such as `Media-Backup` cannot be
/// mistaken for a child of `Media`. Canonicalization also resolves junctions and symlinks before
/// the boundary decision is made.
fn canonicalize_within_library_root(path: &Path, canonical_root: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        format!(
            "无法确认扫描路径 {} 的实际位置，已跳过：{error}",
            path.display()
        )
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(format!(
            "扫描路径超出资源库根目录，已跳过：{}",
            path.display()
        ));
    }
    Ok(canonical)
}

fn canonical_file_within_library_root(
    path: &Path,
    canonical_root: &Path,
) -> Result<PathBuf, String> {
    let canonical = canonicalize_within_library_root(path, canonical_root)?;
    if !canonical.is_file() {
        return Err(format!("扫描路径不是文件，已跳过：{}", path.display()));
    }
    Ok(canonical)
}

fn run_scan_inner(
    app: Option<&AppHandle>,
    database: &Database,
    targets: &[ScanTarget],
    control: &ScanControl,
    extensions: &HashSet<String>,
    token: &str,
) -> Result<(), ScanAbort> {
    let connection = database.connect()?;
    let mut visited = HashSet::new();
    for target in targets {
        check_cancel(control)?;
        let configured_root = Path::new(&target.root.path);
        // Revalidate the registered root immediately before each scan. Besides resolving links
        // and junctions, this blocks legacy overlapping-root rows from moving a Node between root
        // owners and cascading application metadata when either root is later removed.
        let canonical_root = database.validate_scan_root(&target.root).map_err(|error| {
            ScanAbort::Failed(format!(
                "资源库根目录校验失败 {}：{error}",
                configured_root.display()
            ))
        })?;
        let canonical_target = canonicalize_within_library_root(&target.path, &canonical_root)
            .map_err(|message| ScanAbort::Failed(format!("拒绝扫描目标：{message}")))?;
        if !canonical_target.is_dir() {
            return Err(ScanAbort::Failed(format!(
                "扫描目标不是目录，已拒绝：{}",
                target.path.display()
            )));
        }
        {
            let mut progress = control
                .progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            progress.root_id = target.root.id;
            progress.current_path = target.path.to_string_lossy().into_owned();
        }
        scan_directory(
            app,
            &connection,
            target.root.id,
            &target.path,
            &canonical_target,
            &canonical_root,
            target.parent_node_id,
            control,
            extensions,
            token,
            &mut visited,
        )?;
        if target.parent_node_id.is_some() {
            let scanned_node_id = connection
                .query_row(
                    "SELECT id FROM nodes WHERE library_root_id=?1 AND absolute_path=?2 COLLATE NOCASE",
                    params![target.root.id, target.path.to_string_lossy()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?;
            refresh_ancestors(&connection, Some(scanned_node_id))?;
        }
    }
    Ok(())
}

fn bdmv_stream_entries(
    bdmv: &ScanEntryPath,
    canonical_root: &Path,
) -> Result<Option<Vec<ScanEntryPath>>, String> {
    let canonical_bdmv = canonicalize_within_library_root(&bdmv.filesystem, canonical_root)?;
    if !canonical_bdmv.is_dir() {
        return Err(format!(
            "BDMV 路径不是目录，已跳过：{}",
            bdmv.logical.display()
        ));
    }
    let bdmv_entries = fs::read_dir(&canonical_bdmv).map_err(|error| {
        format!(
            "无法读取 BDMV 目录 {}，已跳过：{error}",
            bdmv.logical.display()
        )
    })?;
    let mut stream = None;
    for entry_result in bdmv_entries {
        let entry = entry_result
            .map_err(|error| format!("读取 BDMV 目录项失败 {}：{error}", bdmv.logical.display()))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取 BDMV 文件类型失败 {}：{error}",
                bdmv.logical.join(entry.file_name()).display()
            )
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let entry_path = entry.path();
        if file_type.is_dir() && file_name_eq(&entry_path, "STREAM") {
            let logical = bdmv.logical.join(entry.file_name());
            let filesystem = canonicalize_within_library_root(&entry_path, canonical_root)?;
            if !filesystem.is_dir() {
                return Err(format!(
                    "BDMV STREAM 路径不是目录，已跳过：{}",
                    logical.display()
                ));
            }
            stream = Some(ScanEntryPath {
                logical,
                filesystem,
            });
            break;
        }
    }
    let Some(stream) = stream else {
        return Ok(None);
    };
    let entries = fs::read_dir(&stream.filesystem).map_err(|error| {
        format!(
            "无法读取 BDMV STREAM 目录 {}，已跳过：{error}",
            stream.logical.display()
        )
    })?;
    let mut videos = Vec::new();
    for entry_result in entries {
        let entry = entry_result.map_err(|error| {
            format!(
                "读取 BDMV STREAM 目录项失败 {}：{error}",
                stream.logical.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取 BDMV STREAM 文件类型失败 {}：{error}",
                stream.logical.join(entry.file_name()).display()
            )
        })?;
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let filesystem = entry.path();
        if has_extension(&filesystem, "m2ts") {
            videos.push(ScanEntryPath {
                logical: stream.logical.join(entry.file_name()),
                filesystem,
            });
        }
    }
    Ok(Some(videos))
}

#[allow(clippy::too_many_arguments)]
fn scan_directory(
    app: Option<&AppHandle>,
    connection: &Connection,
    root_id: i64,
    path: &Path,
    filesystem_path: &Path,
    canonical_root: &Path,
    parent_node_id: Option<i64>,
    control: &ScanControl,
    extensions: &HashSet<String>,
    token: &str,
    visited: &mut HashSet<PathBuf>,
) -> Result<i64, ScanAbort> {
    check_cancel(control)?;
    let canonical = match canonicalize_within_library_root(filesystem_path, canonical_root) {
        Ok(canonical) if canonical.is_dir() => canonical,
        Ok(_) => {
            update_progress(app, control, path, |progress| {
                progress.errors += 1;
                progress.message = Some(format!("扫描路径不是目录，已跳过：{}", path.display()));
            });
            return Ok(0);
        }
        Err(message) => {
            update_progress(app, control, path, |progress| {
                progress.errors += 1;
                progress.message = Some(message);
            });
            return Ok(0);
        }
    };
    if !visited.insert(canonical.clone()) {
        return Ok(0);
    }

    update_progress(app, control, path, |progress| progress.folders_scanned += 1);
    let folder_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_str().unwrap_or("资源库"));
    let absolute_path = path.to_string_lossy().into_owned();
    let node_id = upsert_node(
        connection,
        root_id,
        parent_node_id,
        &absolute_path,
        folder_name,
        token,
    )?;

    let existing = connection
        .query_row(
            "SELECT node_type,manual_type_override,total_video_count,direct_video_count,
             child_media_branch_count FROM nodes WHERE id=?1",
            [node_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    if existing.1 && existing.0 == "IGNORED" {
        // Keep the branch's index and user metadata so "restore automatic detection" remains
        // reversible. Ignored branches are excluded from their parent's media counts.
        return Ok(0);
    }

    let errors_before_read = control.progress().errors;
    let read_dir = match fs::read_dir(&canonical) {
        Ok(entries) => entries,
        Err(error) => {
            update_progress(app, control, path, |progress| {
                progress.errors += 1;
                progress.message = Some(format!("无法读取 {}：{error}", path.display()));
            });
            return Ok(existing.2.max(0));
        }
    };

    let mut directories = Vec::new();
    let mut video_files = Vec::new();
    let mut resource_files = Vec::new();
    for entry_result in read_dir {
        check_cancel(control)?;
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                update_progress(app, control, path, |progress| {
                    progress.errors += 1;
                    progress.message = Some(format!("读取目录项失败：{error}"));
                });
                continue;
            }
        };
        let entry_path = entry.path();
        let logical_entry_path = path.join(entry.file_name());
        let file_type = match entry.file_type() {
            Ok(value) => value,
            Err(error) => {
                update_progress(app, control, &logical_entry_path, |progress| {
                    progress.errors += 1;
                    progress.message = Some(format!("读取文件类型失败：{error}"));
                });
                continue;
            }
        };
        // Never follow symlinks/junction-like links: this avoids cycles and out-of-root traversal.
        if file_type.is_symlink() {
            continue;
        }
        let scan_path = ScanEntryPath {
            logical: logical_entry_path,
            filesystem: entry_path,
        };
        if file_type.is_dir() {
            directories.push(scan_path);
        } else if file_type.is_file() {
            if is_video(&scan_path.logical, extensions) {
                video_files.push(scan_path);
            } else {
                resource_files.push(scan_path);
            }
        }
    }
    let entries_complete = control.progress().errors == errors_before_read;
    let mut files_complete = entries_complete;

    let bdmv_directory = directories
        .iter()
        .find(|child| file_name_eq(&child.logical, "BDMV"))
        .cloned();
    let has_bdmv = if let Some(bdmv) = &bdmv_directory {
        match bdmv_stream_entries(bdmv, canonical_root) {
            Ok(Some(mut stream_videos)) => {
                video_files.append(&mut stream_videos);
                true
            }
            Ok(None) => false,
            Err(message) => {
                files_complete = false;
                update_progress(app, control, &bdmv.logical, |progress| {
                    progress.errors += 1;
                    progress.message = Some(message);
                });
                false
            }
        }
    } else {
        false
    };
    if let Some(bdmv) = &bdmv_directory {
        directories.retain(|child| child != bdmv);
    }

    video_files.sort_by(|a, b| {
        crate::db::natural_cmp(
            a.logical
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default(),
            b.logical
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default(),
        )
    });
    for video in &video_files {
        check_cancel(control)?;
        let canonical_file =
            match canonical_file_within_library_root(&video.filesystem, canonical_root) {
                Ok(path) => path,
                Err(message) => {
                    files_complete = false;
                    update_progress(app, control, &video.logical, |progress| {
                        progress.errors += 1;
                        progress.message = Some(message);
                    });
                    continue;
                }
            };
        match index_media_file(connection, node_id, &video.logical, &canonical_file, token) {
            Ok(()) => update_progress(app, control, &video.logical, |progress| {
                progress.videos_found += 1
            }),
            Err(error) => {
                files_complete = false;
                update_progress(app, control, &video.logical, |progress| {
                    progress.errors += 1;
                    progress.message = Some(error);
                });
            }
        }
    }

    resource_files.sort_by(|left, right| {
        crate::db::natural_cmp(
            left.logical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
            right
                .logical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
        )
    });
    for resource in &resource_files {
        check_cancel(control)?;
        let canonical_file =
            match canonical_file_within_library_root(&resource.filesystem, canonical_root) {
                Ok(path) => path,
                Err(message) => {
                    files_complete = false;
                    update_progress(app, control, &resource.logical, |progress| {
                        progress.errors += 1;
                        progress.message = Some(message);
                    });
                    continue;
                }
            };
        if let Err(error) = index_resource_file(
            connection,
            node_id,
            &resource.logical,
            &canonical_file,
            token,
        ) {
            files_complete = false;
            update_progress(app, control, &resource.logical, |progress| {
                progress.errors += 1;
                progress.message = Some(error);
            });
        }
    }
    if let Some(bdmv) = bdmv_directory.as_ref() {
        if !index_transparent_bdmv_resources(
            app,
            connection,
            node_id,
            bdmv,
            canonical_root,
            control,
            extensions,
            token,
        )? {
            files_complete = false;
        }
    }

    directories.sort_by(|a, b| {
        crate::db::natural_cmp(
            a.logical
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default(),
            b.logical
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default(),
        )
    });
    let mut children_complete = files_complete;
    for child in directories {
        let errors_before = control.progress().errors;
        scan_directory(
            app,
            connection,
            root_id,
            &child.logical,
            &child.filesystem,
            canonical_root,
            Some(node_id),
            control,
            extensions,
            token,
            visited,
        )?;
        if control.progress().errors > errors_before {
            children_complete = false;
        }
    }
    check_cancel(control)?;

    // Stale records are index-only cleanup. Source files are never changed.
    if children_complete {
        connection
            .execute(
                "DELETE FROM media_files WHERE node_id=?1 AND last_seen_at<>?2",
                params![node_id, token],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM resource_files WHERE node_id=?1 AND last_seen_at<>?2",
                params![node_id, token],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM nodes WHERE parent_node_id=?1 AND last_seen_at<>?2",
                params![node_id, token],
            )
            .map_err(|error| error.to_string())?;
    }

    let child_summary = indexed_child_media_summary(connection, node_id)?;
    let (direct_video_count, child_media_branch_count, total_video_count) = if children_complete {
        let direct = video_files.len() as i64;
        (
            direct,
            child_summary.branch_count,
            direct + child_summary.total_videos,
        )
    } else {
        // A permission/transient read failure must not turn a previously indexed work into an
        // empty container or prune data that could not be observed in this pass.
        (existing.3, existing.4, existing.2)
    };
    let automatic_type = classify_directory(
        direct_video_count,
        child_media_branch_count,
        child_summary.supplementary_branch_count,
        has_bdmv,
        false,
    );
    connection
        .execute(
            "UPDATE nodes SET
                node_type=CASE WHEN manual_type_override=1 THEN node_type ELSE ?1 END,
                direct_video_count=?2,child_media_branch_count=?3,total_video_count=?4,
                last_seen_at=?5,updated_at=CURRENT_TIMESTAMP
             WHERE id=?6",
            params![
                automatic_type.as_db(),
                direct_video_count,
                child_media_branch_count,
                total_video_count,
                token,
                node_id
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(total_video_count.max(0))
}

fn upsert_node(
    connection: &Connection,
    root_id: i64,
    parent_node_id: Option<i64>,
    absolute_path: &str,
    folder_name: &str,
    token: &str,
) -> Result<i64, ScanAbort> {
    connection
        .execute(
            "INSERT INTO nodes(
                library_root_id,parent_node_id,absolute_path,folder_name,display_name,node_type,last_seen_at
             ) VALUES (?1,?2,?3,?4,?4,'CONTAINER',?5)
             ON CONFLICT(absolute_path) DO UPDATE SET
                library_root_id=excluded.library_root_id,
                parent_node_id=excluded.parent_node_id,
                folder_name=excluded.folder_name,
                last_seen_at=excluded.last_seen_at,
                updated_at=CURRENT_TIMESTAMP",
            params![root_id, parent_node_id, absolute_path, folder_name, token],
        )
        .map_err(|error| ScanAbort::Failed(format!("索引目录失败：{error}")))?;
    connection
        .query_row(
            "SELECT id FROM nodes WHERE absolute_path=?1 COLLATE NOCASE",
            [absolute_path],
            |row| row.get(0),
        )
        .map_err(|error| ScanAbort::Failed(format!("读取目录索引失败：{error}")))
}

fn refresh_ancestors(connection: &Connection, mut node_id: Option<i64>) -> Result<(), ScanAbort> {
    while let Some(id) = node_id {
        let (path, direct_videos, parent_id) = connection
            .query_row(
                "SELECT absolute_path,direct_video_count,parent_node_id FROM nodes WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        let child_summary = indexed_child_media_summary(connection, id)?;
        let automatic_type = classify_directory(
            direct_videos,
            child_summary.branch_count,
            child_summary.supplementary_branch_count,
            has_typical_bdmv(Path::new(&path)),
            false,
        );
        connection
            .execute(
                "UPDATE nodes SET
                    node_type=CASE WHEN manual_type_override=1 THEN node_type ELSE ?1 END,
                    child_media_branch_count=?2,total_video_count=?3,updated_at=CURRENT_TIMESTAMP
                 WHERE id=?4",
                params![
                    automatic_type.as_db(),
                    child_summary.branch_count,
                    direct_videos + child_summary.total_videos,
                    id
                ],
            )
            .map_err(|error| error.to_string())?;
        node_id = parent_id;
    }
    Ok(())
}

fn index_media_file(
    connection: &Connection,
    node_id: i64,
    path: &Path,
    filesystem_path: &Path,
    token: &str,
) -> AppResult<()> {
    let metadata = fs::metadata(filesystem_path)
        .map_err(|error| format!("无法读取视频元数据 {}：{error}", path.to_string_lossy()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let modified_at = metadata
        .modified()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default();
    connection
        .execute(
            "INSERT INTO media_files(
                node_id,absolute_path,file_name,extension,file_size,modified_at,last_seen_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(absolute_path) DO UPDATE SET
                node_id=excluded.node_id,file_name=excluded.file_name,extension=excluded.extension,
                file_size=excluded.file_size,modified_at=excluded.modified_at,last_seen_at=excluded.last_seen_at",
            params![
                node_id,
                path.to_string_lossy(),
                file_name,
                extension,
                metadata.len() as i64,
                modified_at,
                token
            ],
        )
        .map_err(|error| format!("写入视频索引失败：{error}"))?;
    Ok(())
}

fn index_resource_file(
    connection: &Connection,
    node_id: i64,
    path: &Path,
    filesystem_path: &Path,
    token: &str,
) -> AppResult<()> {
    let metadata = fs::metadata(filesystem_path)
        .map_err(|error| format!("无法读取附属资源元数据 {}：{error}", path.to_string_lossy()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let modified_at = metadata
        .modified()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default();
    let resource_type = resource_type_for_extension(&extension);
    connection
        .execute(
            "INSERT INTO resource_files(
                node_id,absolute_path,file_name,extension,file_size,modified_at,resource_type,last_seen_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(absolute_path) DO UPDATE SET
                node_id=excluded.node_id,file_name=excluded.file_name,extension=excluded.extension,
                file_size=excluded.file_size,modified_at=excluded.modified_at,
                resource_type=excluded.resource_type,last_seen_at=excluded.last_seen_at",
            params![
                node_id,
                path.to_string_lossy(),
                file_name,
                extension,
                metadata.len() as i64,
                modified_at,
                resource_type.as_db(),
                token
            ],
        )
        .map_err(|error| format!("写入附属资源索引失败：{error}"))?;
    Ok(())
}

/// A BDMV folder is transparent in the library tree: its STREAM/*.m2ts files belong to the
/// containing work and it must not create its own card. Walk the rest of that tree here so its
/// non-video files remain available as resources on the same parent node.
#[allow(clippy::too_many_arguments)]
fn index_transparent_bdmv_resources(
    app: Option<&AppHandle>,
    connection: &Connection,
    node_id: i64,
    bdmv: &ScanEntryPath,
    canonical_root: &Path,
    control: &ScanControl,
    extensions: &HashSet<String>,
    token: &str,
) -> Result<bool, ScanAbort> {
    let mut complete = true;
    let mut pending = vec![bdmv.clone()];
    let mut visited = HashSet::new();
    while let Some(directory) = pending.pop() {
        check_cancel(control)?;
        let canonical_directory =
            match canonicalize_within_library_root(&directory.filesystem, canonical_root) {
                Ok(path) if path.is_dir() => path,
                Ok(_) => {
                    complete = false;
                    update_progress(app, control, &directory.logical, |progress| {
                        progress.errors += 1;
                        progress.message = Some(format!(
                            "BDMV 附属资源路径不是目录，已跳过：{}",
                            directory.logical.display()
                        ));
                    });
                    continue;
                }
                Err(message) => {
                    complete = false;
                    update_progress(app, control, &directory.logical, |progress| {
                        progress.errors += 1;
                        progress.message = Some(message);
                    });
                    continue;
                }
            };
        if !visited.insert(canonical_directory.clone()) {
            continue;
        }
        let entries = match fs::read_dir(&canonical_directory) {
            Ok(entries) => entries,
            Err(error) => {
                complete = false;
                update_progress(app, control, &directory.logical, |progress| {
                    progress.errors += 1;
                    progress.message = Some(format!(
                        "无法读取 BDMV 附属资源目录 {}：{error}",
                        directory.logical.display()
                    ));
                });
                continue;
            }
        };
        for entry_result in entries {
            check_cancel(control)?;
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(error) => {
                    complete = false;
                    update_progress(app, control, &directory.logical, |progress| {
                        progress.errors += 1;
                        progress.message = Some(format!("读取 BDMV 目录项失败：{error}"));
                    });
                    continue;
                }
            };
            let filesystem_path = entry.path();
            let logical_path = directory.logical.join(entry.file_name());
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    complete = false;
                    update_progress(app, control, &logical_path, |progress| {
                        progress.errors += 1;
                        progress.message = Some(format!("读取 BDMV 文件类型失败：{error}"));
                    });
                    continue;
                }
            };
            // Keep the transparent walk inside the source tree and avoid link cycles.
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(ScanEntryPath {
                    logical: logical_path,
                    filesystem: filesystem_path,
                });
                continue;
            }
            if !file_type.is_file()
                || is_video(&logical_path, extensions)
                || has_extension(&logical_path, "m2ts")
            {
                continue;
            }
            let canonical_file =
                match canonical_file_within_library_root(&filesystem_path, canonical_root) {
                    Ok(path) => path,
                    Err(message) => {
                        complete = false;
                        update_progress(app, control, &logical_path, |progress| {
                            progress.errors += 1;
                            progress.message = Some(message);
                        });
                        continue;
                    }
                };
            if let Err(error) =
                index_resource_file(connection, node_id, &logical_path, &canonical_file, token)
            {
                complete = false;
                update_progress(app, control, &logical_path, |progress| {
                    progress.errors += 1;
                    progress.message = Some(error);
                });
            }
        }
    }
    Ok(complete)
}

pub fn resource_type_for_extension(extension: &str) -> ResourceType {
    match extension
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "ass" | "ssa" | "srt" | "sup" | "vtt" | "sub" | "idx" | "lrc" => ResourceType::Subtitle,
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => {
            ResourceType::Image
        }
        "flac" | "wav" | "mp3" | "m4a" | "aac" | "ogg" | "opus" | "ape" | "wv" | "dts" | "ac3"
        | "eac3" | "truehd" => ResourceType::Audio,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => ResourceType::Archive,
        "ttf" | "otf" | "ttc" | "woff" | "woff2" => ResourceType::Font,
        "m3u" | "m3u8" | "pls" | "cue" | "mpls" => ResourceType::Playlist,
        "pdf" | "txt" | "log" | "nfo" | "xml" | "json" | "md" | "html" | "htm" | "doc" | "docx"
        | "rtf" | "csv" | "yaml" | "yml" => ResourceType::Document,
        _ => ResourceType::Other,
    }
}

fn update_progress(
    app: Option<&AppHandle>,
    control: &ScanControl,
    path: &Path,
    update: impl FnOnce(&mut ScanProgress),
) {
    let mut progress = control
        .progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    progress.current_path = path.to_string_lossy().into_owned();
    update(&mut progress);
    let snapshot = progress.clone();
    drop(progress);
    if let Some(app) = app {
        let _ = app.emit("scan-progress", snapshot);
    }
}

fn check_cancel(control: &ScanControl) -> Result<(), ScanAbort> {
    if control.cancel.load(Ordering::Relaxed) {
        Err(ScanAbort::Cancelled)
    } else {
        Ok(())
    }
}

fn is_video(path: &Path, extensions: &HashSet<String>) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase()))
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn find_stream_directory(bdmv: &Path) -> Option<PathBuf> {
    fs::read_dir(bdmv).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && file_name_eq(&path, "STREAM") {
            Some(path)
        } else {
            None
        }
    })
}

pub fn has_typical_bdmv(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|entries| {
            entries.flatten().find_map(|entry| {
                let candidate = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && file_name_eq(&candidate, "BDMV")
                {
                    find_stream_directory(&candidate)
                } else {
                    None
                }
            })
        })
        .is_some()
}

pub(crate) fn indexed_child_media_summary(
    connection: &Connection,
    node_id: i64,
) -> AppResult<ChildMediaSummary> {
    let mut statement = connection
        .prepare(
            "SELECT folder_name,total_video_count,node_type,manual_type_override
             FROM nodes WHERE parent_node_id=?1",
        )
        .map_err(|error| error.to_string())?;
    let children = statement
        .query_map([node_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut summary = ChildMediaSummary::default();
    for child in children {
        let (folder_name, total_videos, node_type, manually_classified) =
            child.map_err(|error| error.to_string())?;
        if total_videos <= 0 || node_type == NodeType::Ignored.as_db() {
            continue;
        }
        summary.branch_count += 1;
        summary.total_videos += total_videos;
        if !manually_classified && is_supplementary_directory_name(&folder_name) {
            summary.supplementary_branch_count += 1;
        }
    }
    Ok(summary)
}

/// A deliberately small allow-list for folders that normally belong to the work whose
/// main episodes are stored directly in the parent. It only influences automatic detection
/// when the parent already has direct videos; it never turns an otherwise empty parent into a
/// work, and an explicit child classification always wins over this hint.
pub fn is_supplementary_directory_name(folder_name: &str) -> bool {
    let compact = folder_name
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect::<String>();
    if compact.is_empty() {
        return false;
    }

    const EXACT_NAMES: &[&str] = &[
        "sp",
        "sps",
        "special",
        "specials",
        "extra",
        "extras",
        "bonus",
        "bonuses",
        "bonusdisc",
        "bonusdiscs",
        "bonusfeature",
        "bonusfeatures",
        "ova",
        "ovas",
        "oad",
        "oads",
        "ona",
        "onas",
        "omake",
        "ncop",
        "nced",
        "ncoped",
        "ncedop",
        "ncopnced",
        "ncedncop",
        "pv",
        "pvs",
        "cm",
        "cms",
        "trailer",
        "trailers",
        "menu",
        "menus",
        "creditless",
        "creditlessop",
        "creditlessed",
        "promotionalvideo",
        "promotionalvideos",
        "preview",
        "previews",
        "sponsor",
        "sponsors",
    ];
    if EXACT_NAMES.contains(&compact.as_str()) {
        return true;
    }

    const NUMBERABLE_NAMES: &[&str] = &[
        "sp", "sps", "special", "specials", "extra", "extras", "ova", "ovas", "oad", "oads", "ona",
        "onas", "ncop", "nced", "pv", "pvs", "cm", "cms",
    ];
    if NUMBERABLE_NAMES.iter().any(|prefix| {
        compact
            .strip_prefix(prefix)
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
    }) {
        return true;
    }

    const CJK_NAMES: &[&str] = &[
        "特典",
        "特典映像",
        "映像特典",
        "影像特典",
        "特别篇",
        "特別篇",
        "番外篇",
        "番外",
        "附加内容",
        "附加內容",
        "花絮",
    ];
    CJK_NAMES.iter().any(|name| {
        compact == *name
            || compact.strip_prefix(name).is_some_and(|suffix| {
                suffix.chars().all(|c| c.is_ascii_digit())
                    || suffix.strip_prefix("vol").is_some_and(|number| {
                        !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
                    })
            })
    })
}

/// Returns whether an automatically classified child is structural supplementary content of a
/// parent work that already stores its main episodes directly. Matching candidate selection uses
/// this scanner-owned name rule so SP/OVA/Extras allow-lists cannot drift between scanning and
/// Bangumi matching. An explicit child classification always wins.
pub(crate) fn is_automatic_supplementary_child(
    folder_name: &str,
    manually_classified: bool,
    parent_direct_video_count: i64,
) -> bool {
    !manually_classified
        && parent_direct_video_count > 0
        && is_supplementary_directory_name(folder_name)
}

pub fn classify_directory(
    direct_video_count: i64,
    child_media_branch_count: i64,
    supplementary_branch_count: i64,
    has_bdmv: bool,
    ignored: bool,
) -> NodeType {
    if ignored {
        NodeType::Ignored
    } else if has_bdmv
        || (direct_video_count > 0 && child_media_branch_count == supplementary_branch_count)
    {
        NodeType::AutoWork
    } else if direct_video_count == 0 && child_media_branch_count > 0 {
        NodeType::Container
    } else if direct_video_count > 0 && child_media_branch_count > 0 {
        NodeType::Mixed
    } else {
        NodeType::Container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    #[test]
    fn conservative_classification_matches_product_rules() {
        assert_eq!(
            classify_directory(12, 0, 0, false, false),
            NodeType::AutoWork
        );
        assert_eq!(
            classify_directory(0, 3, 0, false, false),
            NodeType::Container
        );
        assert_eq!(classify_directory(1, 2, 0, false, false), NodeType::Mixed);
        assert_eq!(classify_directory(0, 0, 0, true, false), NodeType::AutoWork);
        assert_eq!(classify_directory(5, 1, 1, false, true), NodeType::Ignored);
    }

    #[test]
    fn supplementary_names_are_conservative_and_case_insensitive() {
        for name in [
            "SP",
            "sp 02",
            "Specials",
            "Specials 02",
            "EXTRAS",
            "Extras 2",
            "OVA",
            "OAD_1",
            "NCOP",
            "NCED 02",
            "NCOP & NCED",
            "Menu",
            "Creditless OP",
            "Promotional Videos",
            "Previews",
            "映像特典",
            "影像特典 Vol. 2",
            "番外篇",
            "花絮",
        ] {
            assert!(is_supplementary_directory_name(name), "{name}");
        }
        for name in [
            "Season 01",
            "Disc 1",
            "作品 SPY×FAMILY",
            "OVA Collection 2024",
            "特典作品全集",
            "Another Show",
            "Movie",
        ] {
            assert!(!is_supplementary_directory_name(name), "{name}");
        }

        assert!(is_automatic_supplementary_child("SP 01", false, 12));
        assert!(!is_automatic_supplementary_child("SP 01", true, 12));
        assert!(!is_automatic_supplementary_child("SP 01", false, 0));
        assert!(!is_automatic_supplementary_child(
            "OVA Collection 2024",
            false,
            12
        ));

        assert_eq!(
            classify_directory(12, 4, 4, false, false),
            NodeType::AutoWork
        );
        assert_eq!(classify_directory(12, 4, 3, false, false), NodeType::Mixed);
        // Folder-name hints alone cannot create a work without direct main episodes.
        assert_eq!(
            classify_directory(0, 4, 4, false, false),
            NodeType::Container
        );
    }

    #[test]
    fn scan_treats_direct_episodes_plus_supplements_as_one_work() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("媒体");
        let work = root_path.join("本篇作品");
        let sp = work.join("sp 01");
        let ova = work.join("OVA");
        let extras = work.join("影像特典 Vol. 2");
        let unrelated = work.join("另一部作品");
        for directory in [&sp, &ova, &extras, &unrelated] {
            fs::create_dir_all(directory).unwrap();
        }
        for episode in 1..=12 {
            fs::write(work.join(format!("EP{episode:02}.mkv")), b"episode").unwrap();
        }
        fs::write(sp.join("SP01.mkv"), b"sp").unwrap();
        fs::write(ova.join("OVA01.mp4"), b"ova").unwrap();
        fs::write(extras.join("NCOP.webm"), b"extra").unwrap();
        fs::write(unrelated.join("01.mkv"), b"other").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        let run = |scan_id: &str| {
            database.start_scan_run(scan_id, root.id).unwrap();
            let control = ScanControl {
                scan_id: scan_id.to_string(),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Mutex::new(ScanProgress {
                    scan_id: scan_id.to_string(),
                    root_id: root.id,
                    current_path: root.path.clone(),
                    folders_scanned: 0,
                    videos_found: 0,
                    status: ScanStatus::Running,
                    errors: 0,
                    message: None,
                    phase: ScanPhase::Scanning,
                    auto_match_current: 0,
                    auto_match_total: 0,
                    auto_match_matched: 0,
                    auto_match_pending: 0,
                    auto_match_unmatched: 0,
                    auto_match_errors: 0,
                })),
            };
            run_scan(
                None,
                &database,
                vec![ScanTarget {
                    root: root.clone(),
                    path: root_path.clone(),
                    parent_node_id: None,
                }],
                &control,
                &crate::db::default_video_extensions(),
            );
            assert_eq!(control.progress().status, ScanStatus::Completed);
        };

        run("with-real-child");
        let connection = database.connect().unwrap();
        let work_id: i64 = connection
            .query_row(
                "SELECT id FROM nodes WHERE folder_name='本篇作品'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let kind: String = connection
            .query_row(
                "SELECT node_type FROM nodes WHERE id=?1",
                [work_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "MIXED");
        connection
            .execute(
                "DELETE FROM nodes WHERE parent_node_id=?1 AND folder_name='另一部作品'",
                [work_id],
            )
            .unwrap();
        drop(connection);
        fs::remove_dir_all(&unrelated).unwrap();

        run("supplements-only");
        let connection = database.connect().unwrap();
        let (kind, branches, total): (String, i64, i64) = connection
            .query_row(
                "SELECT node_type,child_media_branch_count,total_video_count FROM nodes WHERE id=?1",
                [work_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "AUTO_WORK");
        assert_eq!(branches, 3);
        assert_eq!(total, 15);
    }

    #[test]
    fn partial_supplement_scan_refreshes_parent_as_auto_work() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("媒体");
        let work = root_path.join("局部扫描作品");
        let sp = work.join("SP");
        fs::create_dir_all(&sp).unwrap();
        fs::write(work.join("01.mkv"), b"episode").unwrap();
        fs::write(sp.join("SP01.mkv"), b"special").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        database.start_scan_run("full", root.id).unwrap();
        let full_scan = scan_control("full", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: root_path.clone(),
                parent_node_id: None,
            }],
            &full_scan,
            &crate::db::default_video_extensions(),
        );

        let connection = database.connect().unwrap();
        let (work_id, sp_id): (i64, i64) = (
            connection
                .query_row(
                    "SELECT id FROM nodes WHERE folder_name='局部扫描作品'",
                    [],
                    |row| row.get(0),
                )
                .unwrap(),
            connection
                .query_row("SELECT id FROM nodes WHERE folder_name='SP'", [], |row| {
                    row.get(0)
                })
                .unwrap(),
        );
        connection
            .execute("UPDATE nodes SET node_type='MIXED' WHERE id=?1", [work_id])
            .unwrap();
        drop(connection);

        database.start_scan_run("partial", root.id).unwrap();
        let partial_scan = scan_control("partial", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: sp.clone(),
                parent_node_id: Some(work_id),
            }],
            &partial_scan,
            &crate::db::default_video_extensions(),
        );

        assert_eq!(partial_scan.progress().status, ScanStatus::Completed);
        assert_eq!(
            database.get_node(work_id).unwrap().node_type,
            NodeType::AutoWork
        );
        assert_eq!(
            database.get_node(sp_id).unwrap().node_type,
            NodeType::AutoWork
        );
    }

    #[test]
    fn reset_old_mixed_uses_supplement_rule_and_manual_parent_survives_scan() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("root");
        let work = root_path.join("work");
        let extras = work.join("Extras");
        fs::create_dir_all(&extras).unwrap();
        fs::write(work.join("01.mkv"), b"episode").unwrap();
        fs::write(extras.join("NCOP.mkv"), b"extra").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        database.start_scan_run("initial", root.id).unwrap();
        let initial_scan = scan_control("initial", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: root_path.clone(),
                parent_node_id: None,
            }],
            &initial_scan,
            &crate::db::default_video_extensions(),
        );

        let connection = database.connect().unwrap();
        let work_id: i64 = connection
            .query_row("SELECT id FROM nodes WHERE folder_name='work'", [], |row| {
                row.get(0)
            })
            .unwrap();
        connection
            .execute(
                "UPDATE nodes SET node_type='MIXED',manual_type_override=0 WHERE id=?1",
                [work_id],
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            database.reset_node_type(work_id).unwrap().node_type,
            NodeType::AutoWork
        );

        let manually_set = database
            .set_node_type(work_id, NodeType::Container)
            .unwrap();
        assert!(manually_set.manual_type_override);
        database.start_scan_run("after-manual", root.id).unwrap();
        let rescan = scan_control("after-manual", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: root_path,
                parent_node_id: None,
            }],
            &rescan,
            &crate::db::default_video_extensions(),
        );
        let preserved = database.get_node(work_id).unwrap();
        assert_eq!(preserved.node_type, NodeType::Container);
        assert!(preserved.manual_type_override);
    }

    fn scan_control(scan_id: &str, root: &LibraryRoot) -> ScanControl {
        ScanControl {
            scan_id: scan_id.to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(ScanProgress {
                scan_id: scan_id.to_string(),
                root_id: root.id,
                current_path: root.path.clone(),
                folders_scanned: 0,
                videos_found: 0,
                status: ScanStatus::Running,
                errors: 0,
                message: None,
                phase: ScanPhase::Scanning,
                auto_match_current: 0,
                auto_match_total: 0,
                auto_match_matched: 0,
                auto_match_pending: 0,
                auto_match_unmatched: 0,
                auto_match_errors: 0,
            })),
        }
    }

    #[test]
    fn canonical_boundary_rejects_a_sibling_with_the_same_text_prefix() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("Media");
        let child = root_path.join("Work");
        let prefix_sibling = temp.path().join("Media-Outside");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&prefix_sibling).unwrap();

        let canonical_root = fs::canonicalize(&root_path).unwrap();
        assert!(canonicalize_within_library_root(&child, &canonical_root).is_ok());
        let error = canonicalize_within_library_root(&prefix_sibling, &canonical_root).unwrap_err();
        assert!(error.contains("超出资源库根目录"), "{error}");
    }

    #[test]
    fn partial_scan_rejects_a_target_outside_the_canonical_library_root() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("Library");
        let outside = temp.path().join("Library-Outside");
        fs::create_dir_all(&root_path).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_video = outside.join("private.mkv");
        fs::write(&outside_video, b"outside").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        database.start_scan_run("outside-partial", root.id).unwrap();
        let control = scan_control("outside-partial", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: outside,
                parent_node_id: Some(9_999),
            }],
            &control,
            &crate::db::default_video_extensions(),
        );

        let progress = control.progress();
        assert_eq!(progress.status, ScanStatus::Failed);
        assert_eq!(progress.videos_found, 0);
        assert!(
            progress
                .message
                .as_deref()
                .is_some_and(|message| message.contains("超出资源库根目录")),
            "{:?}",
            progress.message
        );
        let connection = database.connect().unwrap();
        let node_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE library_root_id=?1",
                [root.id],
                |row| row.get(0),
            )
            .unwrap();
        let media_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM media_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(node_count, 0);
        assert_eq!(media_count, 0);
        assert_eq!(fs::read(&outside_video).unwrap(), b"outside");
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn directory_symlinks_remain_skipped_and_cannot_be_partial_scan_escape_hatches() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("Library");
        let inside = root_path.join("Inside");
        let outside = temp.path().join("Outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(inside.join("01.mkv"), b"inside").unwrap();
        let outside_video = outside.join("private.mkv");
        fs::write(&outside_video, b"outside").unwrap();
        let link = root_path.join("LinkedOutside");
        if let Err(error) = create_directory_symlink(&outside, &link) {
            // Creating symlinks may require Developer Mode or SeCreateSymbolicLinkPrivilege on
            // Windows. The platform-independent outside-target test above still covers the
            // canonical boundary when that privilege is unavailable.
            #[cfg(windows)]
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("failed to create test directory symlink: {error}");
        }

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();

        database.start_scan_run("linked-partial", root.id).unwrap();
        let partial = scan_control("linked-partial", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: link.clone(),
                parent_node_id: Some(9_999),
            }],
            &partial,
            &crate::db::default_video_extensions(),
        );
        assert_eq!(partial.progress().status, ScanStatus::Failed);
        assert_eq!(partial.progress().videos_found, 0);

        database.start_scan_run("linked-full", root.id).unwrap();
        let full = scan_control("linked-full", &root);
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: root_path,
                parent_node_id: None,
            }],
            &full,
            &crate::db::default_video_extensions(),
        );
        assert_eq!(full.progress().status, ScanStatus::Completed);
        assert_eq!(full.progress().videos_found, 1);
        let connection = database.connect().unwrap();
        let escaped_nodes: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE absolute_path=?1 COLLATE NOCASE",
                [link.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        let escaped_media: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM media_files WHERE absolute_path=?1 COLLATE NOCASE",
                [outside_video.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(escaped_nodes, 0);
        assert_eq!(escaped_media, 0);
        assert_eq!(fs::read(outside_video).unwrap(), b"outside");
    }

    #[test]
    fn recursive_scan_indexes_unicode_mixed_and_bdmv_without_touching_sources() {
        let temp = TempDir::new().unwrap();
        let media_root = temp.path().join("媒体 [收藏]");
        let series = media_root.join("动画合集");
        let work_a = series.join("作品 A");
        let work_b = series.join("日本語 作品");
        let mixed = media_root.join("混合");
        let nested = mixed.join("下级").join("三层");
        let bdmv_stream = media_root.join("蓝光作品").join("BDMV").join("STREAM");
        for directory in [&work_a, &work_b, &nested, &bdmv_stream] {
            fs::create_dir_all(directory).unwrap();
        }
        for episode in 1..=12 {
            fs::write(work_a.join(format!("[{episode:02}].mkv")), b"test").unwrap();
        }
        fs::write(work_b.join("01.mp4"), b"test").unwrap();
        fs::write(mixed.join("本目录.webm"), b"test").unwrap();
        fs::write(nested.join("EP2.m2ts"), b"test").unwrap();
        fs::write(bdmv_stream.join("00000.m2ts"), b"test").unwrap();
        fs::write(media_root.join("not-media.txt"), b"keep me").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&media_root, None).unwrap();
        let scan_id = "test-scan".to_string();
        database.start_scan_run(&scan_id, root.id).unwrap();
        let control = ScanControl {
            scan_id: scan_id.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(ScanProgress {
                scan_id,
                root_id: root.id,
                current_path: root.path.clone(),
                folders_scanned: 0,
                videos_found: 0,
                status: ScanStatus::Running,
                errors: 0,
                message: None,
                phase: ScanPhase::Scanning,
                auto_match_current: 0,
                auto_match_total: 0,
                auto_match_matched: 0,
                auto_match_pending: 0,
                auto_match_unmatched: 0,
                auto_match_errors: 0,
            })),
        };
        run_scan(
            None,
            &database,
            vec![ScanTarget {
                root: root.clone(),
                path: media_root.clone(),
                parent_node_id: None,
            }],
            &control,
            &crate::db::default_video_extensions(),
        );
        assert_eq!(control.progress().status, ScanStatus::Completed);
        assert_eq!(control.progress().videos_found, 16);
        assert_eq!(
            fs::read_to_string(media_root.join("not-media.txt")).unwrap(),
            "keep me"
        );

        let connection = database.connect().unwrap();
        let kind: String = connection
            .query_row(
                "SELECT node_type FROM nodes WHERE folder_name='混合'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "MIXED");
        let bdmv_kind: String = connection
            .query_row(
                "SELECT node_type FROM nodes WHERE folder_name='蓝光作品'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bdmv_kind, "AUTO_WORK");
        let bdmv_nodes: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE folder_name='BDMV'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bdmv_nodes, 0);
    }

    #[test]
    fn bdmv_resources_attach_to_parent_without_nodes_or_duplicate_videos() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("蓝光库");
        let work = root_path.join("蓝光作品");
        let bdmv = work.join("BDMV");
        let stream = bdmv.join("STREAM");
        let clipinf = bdmv.join("CLIPINF");
        let playlist = bdmv.join("PLAYLIST");
        let artwork = bdmv.join("META").join("DL");
        for directory in [&stream, &clipinf, &playlist, &artwork] {
            fs::create_dir_all(directory).unwrap();
        }

        let movie = stream.join("00000.m2ts");
        let index = bdmv.join("index.bdmv");
        let clip = clipinf.join("00000.clpi");
        let play_list = playlist.join("00000.mpls");
        let cover = artwork.join("封面.jpg");
        let extensionless = bdmv.join("README");
        fs::write(&movie, b"video").unwrap();
        fs::write(&index, b"index").unwrap();
        fs::write(&clip, b"clip info").unwrap();
        fs::write(&play_list, b"playlist").unwrap();
        fs::write(&cover, b"cover").unwrap();
        fs::write(&extensionless, b"notes").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        let run = |scan_id: &str| {
            database.start_scan_run(scan_id, root.id).unwrap();
            let control = scan_control(scan_id, &root);
            run_scan(
                None,
                &database,
                vec![ScanTarget {
                    root: root.clone(),
                    path: root_path.clone(),
                    parent_node_id: None,
                }],
                &control,
                &crate::db::default_video_extensions(),
            );
            assert_eq!(control.progress().status, ScanStatus::Completed);
            assert_eq!(control.progress().videos_found, 1);
        };
        run("bdmv-resources-first");

        let connection = database.connect().unwrap();
        let work_id: i64 = connection
            .query_row(
                "SELECT id FROM nodes WHERE absolute_path=?1 COLLATE NOCASE",
                [work.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        let transparent_node_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE absolute_path IN (?1,?2,?3,?4,?5)",
                params![
                    bdmv.to_string_lossy(),
                    stream.to_string_lossy(),
                    clipinf.to_string_lossy(),
                    playlist.to_string_lossy(),
                    artwork.to_string_lossy()
                ],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        assert_eq!(transparent_node_count, 0);

        let node = database.get_node(work_id).unwrap();
        assert_eq!(node.node_type, NodeType::AutoWork);
        assert_eq!(node.direct_video_count, 1);
        assert_eq!(node.child_media_branch_count, 0);
        assert_eq!(node.total_video_count, 1);

        let media = database.list_media(work_id).unwrap();
        assert_eq!(media.len(), 1);
        assert_eq!(Path::new(&media[0].absolute_path), movie);

        let resources = database.list_resources(work_id).unwrap();
        assert_eq!(resources.len(), 5);
        assert!(resources.iter().all(|file| file.node_id == work_id));
        assert!(resources
            .iter()
            .all(|file| !file.extension.eq_ignore_ascii_case("m2ts")));
        assert!(resources
            .iter()
            .any(|file| Path::new(&file.absolute_path) == index));
        assert!(resources.iter().any(|file| {
            Path::new(&file.absolute_path) == play_list
                && file.resource_type == ResourceType::Playlist
        }));
        assert!(resources
            .iter()
            .any(|file| Path::new(&file.absolute_path) == cover));
        assert!(resources
            .iter()
            .any(|file| Path::new(&file.absolute_path) == extensionless));

        fs::remove_file(&clip).unwrap();
        run("bdmv-resources-second");
        let resources = database.list_resources(work_id).unwrap();
        assert_eq!(resources.len(), 4);
        assert!(!resources
            .iter()
            .any(|file| Path::new(&file.absolute_path) == clip));
        assert!(movie.is_file());
        assert_eq!(fs::read(&index).unwrap(), b"index");
        let node = database.get_node(work_id).unwrap();
        assert_eq!(node.direct_video_count, 1);
        assert_eq!(node.child_media_branch_count, 0);
        assert_eq!(node.total_video_count, 1);
    }

    #[test]
    fn rescan_preserves_manual_type_and_only_removes_index_rows() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("root");
        let work = root_path.join("work");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("01.mkv");
        fs::write(&source, b"immutable media").unwrap();
        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();

        let run_once = |id: &str| {
            database.start_scan_run(id, root.id).unwrap();
            let control = ScanControl {
                scan_id: id.to_string(),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Mutex::new(ScanProgress {
                    scan_id: id.to_string(),
                    root_id: root.id,
                    current_path: root.path.clone(),
                    folders_scanned: 0,
                    videos_found: 0,
                    status: ScanStatus::Running,
                    errors: 0,
                    message: None,
                    phase: ScanPhase::Scanning,
                    auto_match_current: 0,
                    auto_match_total: 0,
                    auto_match_matched: 0,
                    auto_match_pending: 0,
                    auto_match_unmatched: 0,
                    auto_match_errors: 0,
                })),
            };
            run_scan(
                None,
                &database,
                vec![ScanTarget {
                    root: root.clone(),
                    path: root_path.clone(),
                    parent_node_id: None,
                }],
                &control,
                &crate::db::default_video_extensions(),
            );
        };
        run_once("first");
        let connection = database.connect().unwrap();
        let node_id: i64 = connection
            .query_row("SELECT id FROM nodes WHERE folder_name='work'", [], |row| {
                row.get(0)
            })
            .unwrap();
        drop(connection);
        database
            .set_node_type(node_id, NodeType::Container)
            .unwrap();
        run_once("second");
        assert_eq!(
            database.get_node(node_id).unwrap().node_type,
            NodeType::Container
        );
        assert_eq!(fs::read(&source).unwrap(), b"immutable media");
        database.remove_root(root.id).unwrap();
        assert!(source.exists(), "deleting a library removes only its index");
    }

    #[test]
    fn scan_indexes_non_video_resources_without_changing_project_counts() {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("媒体库");
        let work = root_path.join("作品");
        let fonts = work.join("Fonts");
        let specials = work.join("SP");
        fs::create_dir_all(&fonts).unwrap();
        fs::create_dir_all(&specials).unwrap();

        fs::write(work.join("Movie.mkv"), b"video").unwrap();
        fs::write(work.join("Movie.ass"), b"subtitle").unwrap();
        fs::write(work.join("Movie.sup"), b"subtitle").unwrap();
        fs::write(work.join("cover.jpg"), b"image fixture").unwrap();
        fs::write(work.join("booklet.pdf"), b"document").unwrap();
        fs::write(work.join("data.xyzabc"), b"unknown").unwrap();
        fs::write(fonts.join("font1.ttf"), b"font").unwrap();
        fs::write(fonts.join("readme.txt"), b"readme").unwrap();
        fs::write(specials.join("SP01.mkv"), b"special video").unwrap();
        fs::write(specials.join("SP01.ass"), b"special subtitle").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        let run = |scan_id: &str| {
            database.start_scan_run(scan_id, root.id).unwrap();
            let control = scan_control(scan_id, &root);
            run_scan(
                None,
                &database,
                vec![ScanTarget {
                    root: root.clone(),
                    path: root_path.clone(),
                    parent_node_id: None,
                }],
                &control,
                &crate::db::default_video_extensions(),
            );
            assert_eq!(control.progress().status, ScanStatus::Completed);
        };
        run("resources-first");

        let connection = database.connect().unwrap();
        let work_id: i64 = connection
            .query_row(
                "SELECT id FROM nodes WHERE folder_name='作品'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let fonts_id: i64 = connection
            .query_row(
                "SELECT id FROM nodes WHERE folder_name='Fonts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let specials_id: i64 = connection
            .query_row("SELECT id FROM nodes WHERE folder_name='SP'", [], |row| {
                row.get(0)
            })
            .unwrap();
        drop(connection);

        let direct = database.list_resources(work_id).unwrap();
        assert_eq!(direct.len(), 5);
        assert!(direct.iter().any(|file| {
            file.file_name == "Movie.ass" && file.resource_type == ResourceType::Subtitle
        }));
        assert!(direct.iter().any(|file| {
            file.file_name == "cover.jpg" && file.resource_type == ResourceType::Image
        }));
        assert!(direct.iter().any(|file| {
            file.file_name == "booklet.pdf" && file.resource_type == ResourceType::Document
        }));
        assert!(direct.iter().any(|file| {
            file.file_name == "data.xyzabc" && file.resource_type == ResourceType::Other
        }));
        assert_eq!(database.list_resources(fonts_id).unwrap().len(), 2);
        assert_eq!(database.list_resources(specials_id).unwrap().len(), 1);

        let refreshed_root = database.get_root(root.id).unwrap();
        assert_eq!(refreshed_root.node_count, Some(1));
        let all = database.list_all_resources().unwrap();
        assert_eq!(all.total_count, 1);
        assert_eq!(all.nodes[0].id, work_id);
        assert_eq!(all.nodes[0].library_root_id, root.id);

        fs::remove_file(work.join("data.xyzabc")).unwrap();
        run("resources-second");
        assert!(!database
            .list_resources(work_id)
            .unwrap()
            .iter()
            .any(|file| file.file_name == "data.xyzabc"));
        assert!(work.join("booklet.pdf").is_file());
    }

    #[test]
    fn resource_type_mapping_keeps_unknown_and_extensionless_files() {
        assert_eq!(resource_type_for_extension("ass"), ResourceType::Subtitle);
        assert_eq!(resource_type_for_extension("FLAC"), ResourceType::Audio);
        assert_eq!(resource_type_for_extension("7z"), ResourceType::Archive);
        assert_eq!(resource_type_for_extension("xyzabc"), ResourceType::Other);
        assert_eq!(resource_type_for_extension(""), ResourceType::Other);
    }

    #[test]
    fn all_resources_unifies_two_roots_without_merging_same_named_projects() {
        let temp = TempDir::new().unwrap();
        let root_a_path = temp.path().join("动画库");
        let root_b_path = temp.path().join("电影库");
        let work_a = root_a_path.join("同名作品");
        let work_b = root_b_path.join("同名作品");
        fs::create_dir_all(&work_a).unwrap();
        fs::create_dir_all(&work_b).unwrap();
        fs::write(work_a.join("01.mkv"), b"a").unwrap();
        fs::write(work_b.join("01.mkv"), b"b").unwrap();

        let database = Database::new(temp.path().join("test.db"));
        database.migrate().unwrap();
        let root_a = database.add_root(&root_a_path, None).unwrap();
        let root_b = database.add_root(&root_b_path, None).unwrap();
        database.start_scan_run("two-roots", root_a.id).unwrap();
        database.start_scan_run("two-roots", root_b.id).unwrap();
        let control = scan_control("two-roots", &root_a);
        run_scan(
            None,
            &database,
            vec![
                ScanTarget {
                    root: root_a.clone(),
                    path: root_a_path.clone(),
                    parent_node_id: None,
                },
                ScanTarget {
                    root: root_b.clone(),
                    path: root_b_path.clone(),
                    parent_node_id: None,
                },
            ],
            &control,
            &crate::db::default_video_extensions(),
        );
        assert_eq!(control.progress().status, ScanStatus::Completed);

        let all = database.list_all_resources().unwrap();
        assert_eq!(all.total_count, 2);
        assert_eq!(
            all.nodes
                .iter()
                .map(|node| node.library_root_id)
                .collect::<HashSet<_>>(),
            HashSet::from([root_a.id, root_b.id])
        );
        assert!(all.nodes.iter().all(|node| node.folder_name == "同名作品"));
        assert_eq!(database.get_root(root_a.id).unwrap().node_count, Some(1));
        assert_eq!(database.get_root(root_b.id).unwrap().node_count, Some(1));

        let renamed = database
            .update_root_display_name(root_a.id, "动画收藏")
            .unwrap();
        assert_eq!(renamed.display_name, "动画收藏");
        assert_eq!(renamed.path, root_a.path);
        assert!(root_a_path.is_dir());
    }
}
