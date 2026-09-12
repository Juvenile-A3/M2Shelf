//! A read-only catalogue over the existing path-owned index. Grouping never merges database
//! rows, so rescans, manual bindings, favorites and playback retain their original identities.
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use crate::{
    db::{AppResult, Database},
    models::{MediaNode, NodeDetail, NodeType, WorkGroup},
    title_extractor::{extract_title_signals, is_generic_title, normalize_title_for_match},
};

fn is_file_node(node: &MediaNode) -> bool {
    let path = Path::new(&node.absolute_path);
    path.file_stem().and_then(|name| name.to_str()) == Some(node.folder_name.as_str())
        && path.file_name().and_then(|name| name.to_str()) != Some(node.folder_name.as_str())
}

/// Strip explicit episode syntax only. A bare numeric sequel title remains distinct.
fn episode_title(value: &str) -> String {
    let normalized = value
        .split_whitespace()
        .map(|token| {
            let lower = token.to_ascii_lowercase();
            if let Some((season, episode)) = lower
                .strip_prefix('s')
                .and_then(|rest| rest.split_once('e'))
            {
                if !season.is_empty()
                    && !episode.is_empty()
                    && season.chars().all(|c| c.is_ascii_digit())
                    && episode.chars().all(|c| c.is_ascii_digit())
                {
                    return format!("S{season}");
                }
            }
            token.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((title, episode)) = normalized.rsplit_once(" - ") {
        if !title.is_empty()
            && (1..=3).contains(&episode.len())
            && episode.chars().all(|c| c.is_ascii_digit())
        {
            return title.to_string();
        }
    }
    normalized
}

fn title_signals(node: &MediaNode) -> crate::title_extractor::TitleSignals {
    extract_title_signals(&if is_file_node(node) {
        episode_title(&node.display_name)
    } else {
        node.display_name.clone()
    })
}

fn local_title(node: &MediaNode) -> String {
    let signals = title_signals(node);
    if signals.cleaned_title.is_empty() || is_generic_title(&signals.cleaned_title) {
        node.display_name.clone()
    } else {
        signals.cleaned_title
    }
}

fn group_key(node: &MediaNode) -> String {
    if let Some(binding) = &node.binding {
        return format!(
            "bangumi:{}:{}",
            binding.provider_subject_type, binding.provider_subject_id
        );
    }
    // Only flatten unbound episode file nodes within the same physical directory. Folder
    // releases and cross-library aliases need an explicit/shared Bangumi identity to merge.
    let path = Path::new(&node.absolute_path);
    if !is_file_node(node) {
        return format!("node:{}", node.id);
    }
    let signals = title_signals(node);
    let title = normalize_title_for_match(&signals.cleaned_title);
    if title.chars().count() < 3
        || title.chars().all(|c| c.is_numeric())
        || is_generic_title(&signals.cleaned_title)
    {
        return format!("node:{}", node.id);
    }
    format!(
        "local:{}:{:?}:{}:{:?}:{:?}:{:?}",
        node.library_root_id,
        path.parent(),
        title,
        signals.season_number,
        signals.year,
        signals.edition_kind
    )
}

pub fn group_works(mut nodes: Vec<MediaNode>) -> Vec<WorkGroup> {
    nodes.sort_by_key(|node| node.id);
    let mut groups: BTreeMap<String, Vec<MediaNode>> = BTreeMap::new();
    for node in nodes {
        groups.entry(group_key(&node)).or_default().push(node);
    }
    groups
        .into_values()
        .map(|mut sources| {
            // Prefer a source with artwork; ties retain deterministic source identity.
            sources.sort_by_key(|source| (source.cover_cache_path.is_none(), source.id));
            let mut node = sources[0].clone();
            node.node_type = NodeType::Work;
            if node.binding.is_none() {
                node.display_name = local_title(&node);
            }
            node.direct_video_count = sources.iter().map(|source| source.direct_video_count).sum();
            node.total_video_count = sources
                .iter()
                .map(|source| {
                    if source.direct_video_count > 0 {
                        source.direct_video_count
                    } else {
                        source.total_video_count
                    }
                })
                .sum();
            node.child_media_branch_count = 0;
            node.created_at = sources.iter().map(|n| &n.created_at).min().unwrap().clone();
            node.last_watched_at = sources
                .iter()
                .filter_map(|source| source.last_watched_at.as_ref())
                .max()
                .cloned();
            node.latest_file_modified_at = sources
                .iter()
                .filter_map(|source| source.latest_file_modified_at.as_ref())
                .max()
                .cloned();
            let mut tags = BTreeMap::new();
            for source in &sources {
                for tag in &source.user_tags {
                    tags.insert(tag.id, tag.clone());
                }
            }
            node.user_tags = tags.into_values().collect();
            WorkGroup { node, sources }
        })
        .collect()
}

pub fn work_detail(database: &Database, node_id: i64) -> AppResult<NodeDetail> {
    let group = group_works(database.list_work_sources()?)
        .into_iter()
        .find(|group| group.sources.iter().any(|node| node.id == node_id))
        .ok_or_else(|| "作品已不在当前索引中，请刷新资源库。".to_string())?;
    let source_ids: HashSet<i64> = group.sources.iter().map(|node| node.id).collect();
    let mut detail = NodeDetail {
        binding: group.node.binding.clone(),
        breadcrumbs: database.breadcrumbs(group.node.id, true)?,
        node: group.node,
        children: Vec::new(),
        media_files: Vec::new(),
        resource_files: Vec::new(),
        work_sources: None,
    };
    for source in &group.sources {
        detail.media_files.extend(database.list_media(source.id)?);
        detail
            .resource_files
            .extend(database.list_resources(source.id)?);
        detail.children.extend(
            database
                .list_children(source.id)?
                .into_iter()
                .filter(|node| !source_ids.contains(&node.id)),
        );
    }
    detail.children.sort_by_key(|node| node.id);
    detail.children.dedup_by_key(|node| node.id);
    detail.work_sources = Some(group.sources);
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_titles_keep_seasons_and_numeric_sequels() {
        assert_eq!(episode_title("Steins;Gate - 01"), "Steins;Gate");
        assert_eq!(episode_title("Steins;Gate s02e03"), "Steins;Gate S02");
        assert_eq!(episode_title("Movie 2"), "Movie 2");
        assert_eq!(episode_title("Movie - 2026"), "Movie - 2026");
        assert_eq!(episode_title("Steins;Gate 0"), "Steins;Gate 0");
    }
}
