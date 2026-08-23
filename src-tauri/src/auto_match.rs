use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    bangumi, cache,
    db::{AppResult, ConditionalBindingSave, Database},
    models::{BangumiSubject, CoverSource, MediaNode, NodeType},
    scanner::ScanTarget,
    title_extractor::{self, EditionKind, MatchEvidence},
};

const AUTO_SEARCH_LIMIT: usize = 20;
const MAX_QUERIES_PER_NODE: usize = 3;
const MAX_CANDIDATES_PER_NODE: usize = 30;
const MAX_DETAIL_ENRICHMENTS: usize = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AutoMatchReport {
    pub examined: usize,
    pub matched: usize,
    pub pending: usize,
    pub unmatched: usize,
    pub errors: usize,
}

/// `IfAbsent` is the non-destructive scan policy. `ExplicitRematch` is reserved for a user
/// initiated rematch command and may replace the binding that was present when that run began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchWriteMode {
    IfAbsent,
    ExplicitRematch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchWeights {
    pub exact_primary: i32,
    pub exact_alternate: i32,
    pub similarity_max: i32,
    pub year_match: i32,
    pub year_conflict: i32,
    pub season_match: i32,
    pub season_conflict: i32,
    pub edition_match: i32,
    pub edition_conflict: i32,
    pub hierarchy_match: i32,
    pub generic_penalty: i32,
    pub automatic_threshold: i32,
    pub pending_threshold: i32,
    pub minimum_margin: i32,
    pub container_minimum_margin: i32,
}

impl Default for MatchWeights {
    fn default() -> Self {
        Self {
            exact_primary: 55,
            // Folder/display/file-derived alternates are structured title evidence, not provider
            // rank. With 42 points, even an exact alternate plus perfect similarity and first
            // provider rank topped out at 77 and could never cross the 82 automatic gate unless
            // unrelated optional metadata happened to exist. Keep the absolute/margin/conflict
            // gates, but allow an unambiguous exact alternate to qualify.
            exact_alternate: 50,
            similarity_max: 30,
            year_match: 8,
            year_conflict: -12,
            season_match: 12,
            season_conflict: -25,
            edition_match: 10,
            edition_conflict: -20,
            hierarchy_match: 8,
            generic_penalty: -20,
            automatic_threshold: 82,
            pending_threshold: 60,
            minimum_margin: 15,
            container_minimum_margin: 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrongConflict {
    NonAnime,
    Year,
    Season,
    MissingSeason,
    Edition,
}

#[derive(Debug, Clone)]
pub struct CandidateScore {
    pub subject: BangumiSubject,
    pub score: i32,
    pub official_rank: usize,
    pub primary_exact: bool,
    pub alternate_exact: bool,
    pub similarity_score: i32,
    pub strong_conflicts: Vec<StrongConflict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchConfidence {
    High,
    Pending,
    Low,
}

#[derive(Debug, Clone)]
pub struct MatchDecision {
    pub confidence: MatchConfidence,
    pub best: Option<CandidateScore>,
    pub second_score: Option<i32>,
    pub score_margin: i32,
}

#[derive(Debug, Clone)]
struct RecalledCandidate {
    subject: BangumiSubject,
    official_rank: usize,
}

#[derive(Default)]
struct MatchRunCache {
    searches: HashMap<String, Result<Vec<BangumiSubject>, String>>,
    details: HashMap<i64, Result<BangumiSubject, String>>,
}

/// Runs the confidence matcher for the unbound, eligible Nodes in a completed scan scope.
/// Online failures are counted rather than returned so local scanning remains successful.
pub fn run_auto_match<F, C>(
    database: &Database,
    targets: &[ScanTarget],
    cache_root: Result<&Path, &str>,
    on_progress: F,
    is_cancelled: C,
) -> AutoMatchReport
where
    F: FnMut(usize, usize, &MediaNode),
    C: Fn() -> bool,
{
    let candidates = match candidates_in_targets(database, targets) {
        Ok(candidates) => candidates,
        Err(_) => {
            return AutoMatchReport {
                errors: 1,
                ..AutoMatchReport::default()
            };
        }
    };
    run_match_nodes(
        database,
        &candidates,
        cache_root,
        MatchWriteMode::IfAbsent,
        on_progress,
        is_cancelled,
    )
}

/// Synchronous reusable runner for an explicit existing-content match command.
///
/// The caller remains responsible for selecting Nodes in an authorized Library Root. This
/// function still rejects ineligible/self-ignored Nodes, preserves manual covers, keeps all
/// network and cache writes application-owned, and applies scan-style cancellation/breakers.
pub fn run_match_nodes<F, C>(
    database: &Database,
    nodes: &[MediaNode],
    cache_root: Result<&Path, &str>,
    write_mode: MatchWriteMode,
    mut on_progress: F,
    is_cancelled: C,
) -> AutoMatchReport
where
    F: FnMut(usize, usize, &MediaNode),
    C: Fn() -> bool,
{
    let total = nodes.len();
    let mut report = AutoMatchReport::default();
    let mut consecutive_errors = 0usize;
    let mut run_cache = MatchRunCache::default();

    for (index, node) in nodes.iter().enumerate() {
        if is_cancelled() {
            break;
        }
        report.examined += 1;
        on_progress(index + 1, total, node);

        match auto_match_node(
            database,
            node,
            cache_root,
            write_mode,
            &mut run_cache,
            &is_cancelled,
        ) {
            Ok(AutoMatchNodeResult::Matched) => {
                report.matched += 1;
                consecutive_errors = 0;
            }
            Ok(AutoMatchNodeResult::MatchedWithCoverError) => {
                report.matched += 1;
                report.errors += 1;
                consecutive_errors = 0;
            }
            Ok(AutoMatchNodeResult::Pending) => {
                report.pending += 1;
                consecutive_errors = 0;
            }
            Ok(AutoMatchNodeResult::Unmatched) => {
                report.unmatched += 1;
                consecutive_errors = 0;
            }
            Ok(AutoMatchNodeResult::AlreadyBound) => consecutive_errors = 0,
            Ok(AutoMatchNodeResult::Cancelled) => break,
            Err(error) => {
                report.errors += 1;
                consecutive_errors += 1;
                // One provider-wide search failure is enough to stop this run's online phase;
                // retrying every remaining Node would multiply the same offline/429 delay.
                if is_provider_search_failure(&error) || consecutive_errors >= 3 {
                    break;
                }
            }
        }
    }
    report
}

fn is_provider_search_failure(error: &str) -> bool {
    error.starts_with("搜索 Bangumi 失败：") || error.starts_with("无法初始化 Bangumi 网络客户端：")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoMatchNodeResult {
    Matched,
    MatchedWithCoverError,
    Pending,
    Unmatched,
    AlreadyBound,
    Cancelled,
}

fn candidates_in_targets(database: &Database, targets: &[ScanTarget]) -> AppResult<Vec<MediaNode>> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for target in targets {
        for node in database.list_unbound_bangumi_candidates(target.root.id)? {
            if seen.insert(node.id)
                && cache::is_equal_or_within(Path::new(&node.absolute_path), target.path.as_path())
            {
                candidates.push(node);
            }
        }
    }
    candidates.sort_by_key(|node| node.id);
    Ok(candidates)
}

fn auto_match_node<C>(
    database: &Database,
    node: &MediaNode,
    cache_root: Result<&Path, &str>,
    write_mode: MatchWriteMode,
    run_cache: &mut MatchRunCache,
    is_cancelled: &C,
) -> AppResult<AutoMatchNodeResult>
where
    C: Fn() -> bool,
{
    if !node.can_bind_bangumi() || node.node_type == NodeType::Ignored {
        return Ok(AutoMatchNodeResult::Unmatched);
    }
    let initial_binding = database.get_binding(node.id)?;
    if write_mode == MatchWriteMode::IfAbsent && initial_binding.is_some() {
        return Ok(AutoMatchNodeResult::AlreadyBound);
    }

    let media_file_names = database
        .list_media(node.id)?
        .into_iter()
        .map(|file| file.file_name)
        .collect::<Vec<_>>();
    let parent_name = node
        .parent_node_id
        .and_then(|parent_id| database.get_node(parent_id).ok())
        .map(|parent| parent.display_name);
    let evidence = title_extractor::build_match_evidence(
        &node.folder_name,
        &node.display_name,
        parent_name.as_deref(),
        &media_file_names,
    );
    let decision = match assess_evidence_online(
        &evidence,
        node.node_type == NodeType::Container,
        run_cache,
        is_cancelled,
    )? {
        OnlineAssessment::Decision(decision) => *decision,
        OnlineAssessment::Cancelled => return Ok(AutoMatchNodeResult::Cancelled),
    };
    // Keep the ambiguity diagnostics available to the forthcoming review UI even though this
    // runner currently persists only the confidence class.
    let _decision_diagnostics = (decision.second_score, decision.score_margin);

    let subject = match decision.confidence {
        MatchConfidence::High => decision
            .best
            .map(|candidate| candidate.subject)
            .ok_or_else(|| "高置信度匹配缺少 Bangumi 候选。".to_string())?,
        MatchConfidence::Pending => return Ok(AutoMatchNodeResult::Pending),
        MatchConfidence::Low => return Ok(AutoMatchNodeResult::Unmatched),
    };
    if is_cancelled() {
        return Ok(AutoMatchNodeResult::Cancelled);
    }

    // Re-read after network I/O. In explicit-rematch mode a binding changed after this run began
    // is a newer user decision; the DB compares that expected Subject and replaces it in the same
    // immediate transaction so there is no check/write race.
    let current_binding = database.get_binding(node.id)?;
    match write_mode {
        MatchWriteMode::IfAbsent => {
            if current_binding.is_some() || !database.save_binding_if_absent(node.id, &subject)? {
                return Ok(AutoMatchNodeResult::AlreadyBound);
            }
        }
        MatchWriteMode::ExplicitRematch => {
            let initial_subject = initial_binding
                .as_ref()
                .map(|binding| binding.provider_subject_id);
            let old_path = match database.save_rematched_binding_if_unchanged(
                node.id,
                initial_subject,
                &subject,
            )? {
                ConditionalBindingSave::Applied(path) => path,
                ConditionalBindingSave::Stale => return Ok(AutoMatchNodeResult::AlreadyBound),
            };
            if let (Some(old_path), Ok(active_cache)) = (old_path.as_deref(), cache_root) {
                remove_cached_file_if_unreferenced(database, old_path, active_cache);
            }
        }
    }
    if is_cancelled() {
        return Ok(AutoMatchNodeResult::Matched);
    }

    // A manual cover selected before or during matching is explicit curation and is never
    // replaced. The new binding may still improve the locale-aware title.
    if database.get_node(node.id)?.cover_source == CoverSource::Manual {
        return Ok(AutoMatchNodeResult::Matched);
    }

    let cover_result = match cache_root {
        Ok(cache_root) => bangumi::download_cover(cache_root, &subject),
        Err(error) => Err(error.to_string()),
    };
    if is_cancelled() {
        return Ok(AutoMatchNodeResult::Matched);
    }
    let cover_error = match cover_result {
        Ok(Some(path)) => {
            if database.set_bangumi_cover_for_subject_unless_manual(
                node.id,
                subject.subject_id,
                &path,
            )? {
                database.set_binding_cover_error_if_subject(node.id, subject.subject_id, None)?;
            }
            false
        }
        Ok(None) => {
            database.set_binding_cover_error_if_subject(
                node.id,
                subject.subject_id,
                Some("该 Bangumi 条目没有可用封面。"),
            )?;
            true
        }
        Err(error) => {
            database.set_binding_cover_error_if_subject(
                node.id,
                subject.subject_id,
                Some(&error),
            )?;
            true
        }
    };

    Ok(if cover_error {
        AutoMatchNodeResult::MatchedWithCoverError
    } else {
        AutoMatchNodeResult::Matched
    })
}

fn remove_cached_file_if_unreferenced(database: &Database, path: &Path, cache_root: &Path) {
    if cache::is_equal_or_within(path, cache_root)
        && database.cover_path_reference_count(path).ok() == Some(0)
    {
        let _ = cache::remove_cached_file(path, cache_root);
    }
}

enum OnlineAssessment {
    Decision(Box<MatchDecision>),
    Cancelled,
}

fn assess_evidence_online<C>(
    evidence: &MatchEvidence,
    is_container: bool,
    run_cache: &mut MatchRunCache,
    is_cancelled: &C,
) -> AppResult<OnlineAssessment>
where
    C: Fn() -> bool,
{
    let queries = match_queries(evidence);
    if queries.is_empty() {
        return Ok(OnlineAssessment::Decision(Box::new(MatchDecision {
            confidence: MatchConfidence::Low,
            best: None,
            second_score: None,
            score_margin: 0,
        })));
    }

    let mut search_results = Vec::<Vec<BangumiSubject>>::with_capacity(queries.len());
    for query in queries {
        if is_cancelled() {
            return Ok(OnlineAssessment::Cancelled);
        }
        let cache_key = title_extractor::normalize_title_for_match(&query);
        let results = run_cache
            .searches
            .entry(cache_key)
            .or_insert_with(|| bangumi::search(&query, AUTO_SEARCH_LIMIT))
            .clone()?;
        if is_cancelled() {
            return Ok(OnlineAssessment::Cancelled);
        }
        search_results.push(results.into_iter().take(AUTO_SEARCH_LIMIT).collect());
    }
    let mut recalled = merge_search_results_fair(&search_results);

    if recalled.is_empty() {
        return Ok(OnlineAssessment::Decision(Box::new(MatchDecision {
            confidence: MatchConfidence::Low,
            best: None,
            second_score: None,
            score_margin: 0,
        })));
    }

    // Stage one uses search response metadata only. Only the most plausible five Subjects incur
    // a detail request; detail errors remain best-effort and do not discard search evidence.
    let weights = MatchWeights::default();
    let mut preliminary = recalled
        .iter()
        .map(|candidate| {
            score_candidate(
                evidence,
                &candidate.subject,
                candidate.official_rank,
                &weights,
            )
        })
        .collect::<Vec<_>>();
    sort_scores(&mut preliminary);
    let detail_ids = preliminary
        .iter()
        .take(MAX_DETAIL_ENRICHMENTS)
        .map(|score| score.subject.subject_id)
        .collect::<Vec<_>>();

    for subject_id in detail_ids {
        if is_cancelled() {
            return Ok(OnlineAssessment::Cancelled);
        }
        let Some(candidate) = recalled
            .iter_mut()
            .find(|candidate| candidate.subject.subject_id == subject_id)
        else {
            continue;
        };
        let enriched = run_cache
            .details
            .entry(subject_id)
            .or_insert_with(|| bangumi::enrich_subject(&candidate.subject))
            .clone();
        if let Ok(subject) = enriched {
            candidate.subject = subject;
        }
        if is_cancelled() {
            return Ok(OnlineAssessment::Cancelled);
        }
    }

    let scored = recalled
        .into_iter()
        .map(|candidate| {
            score_candidate(
                evidence,
                &candidate.subject,
                candidate.official_rank,
                &weights,
            )
        })
        .collect::<Vec<_>>();
    Ok(OnlineAssessment::Decision(Box::new(decide_scores(
        scored,
        is_container,
        &weights,
    ))))
}

/// Merges provider results by rank round instead of allowing the earliest query to fill the
/// bounded candidate pool first. Query order remains the deterministic tie-breaker within a
/// rank, while each query gets an equal opportunity to contribute candidates.
fn merge_search_results_fair(search_results: &[Vec<BangumiSubject>]) -> Vec<RecalledCandidate> {
    let max_rank = search_results
        .iter()
        .map(|results| results.len().min(AUTO_SEARCH_LIMIT))
        .max()
        .unwrap_or(0);
    let mut recalled = Vec::<RecalledCandidate>::new();
    let mut subject_indexes = HashMap::<i64, usize>::new();

    'rank_rounds: for rank in 0..max_rank {
        for results in search_results {
            let Some(subject) = results.get(rank) else {
                continue;
            };
            if subject.subject_type != 2 {
                continue;
            }
            if let Some(existing_index) = subject_indexes.get(&subject.subject_id).copied() {
                recalled[existing_index].official_rank =
                    recalled[existing_index].official_rank.min(rank);
                continue;
            }
            subject_indexes.insert(subject.subject_id, recalled.len());
            recalled.push(RecalledCandidate {
                subject: subject.clone(),
                official_rank: rank,
            });
            if recalled.len() >= MAX_CANDIDATES_PER_NODE {
                break 'rank_rounds;
            }
        }
    }

    recalled
}

/// Produces up to three distinct, safe provider queries in evidence-priority order.
pub fn match_queries(evidence: &MatchEvidence) -> Vec<String> {
    let mut queries = Vec::new();
    push_query(&mut queries, &evidence.primary_title);
    // A folder may use a fan-created English translation that the official provider has never
    // indexed, while episode files or the parent still carry a romanized/original title. Give
    // those independent local sources first access to the two remaining bounded searches.
    for candidate in [
        evidence.frequent_file_title.as_deref(),
        evidence.parent_title.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if queries.len() >= MAX_QUERIES_PER_NODE {
            break;
        }
        push_query(&mut queries, candidate);
    }
    for candidate in &evidence.alternate_titles {
        if queries.len() >= MAX_QUERIES_PER_NODE {
            break;
        }
        push_query(&mut queries, candidate);
    }
    queries.truncate(MAX_QUERIES_PER_NODE);
    queries
}

fn push_query(queries: &mut Vec<String>, candidate: &str) {
    if !title_extractor::is_safe_match_query(candidate) {
        return;
    }
    let normalized = title_extractor::normalize_title_for_match(candidate);
    if !queries
        .iter()
        .any(|query| title_extractor::normalize_title_for_match(query) == normalized)
    {
        queries.push(candidate.trim().to_string());
    }
}

/// Pure first/final-stage scorer. Provider order contributes at most five points and can never
/// turn an otherwise unrelated title into an automatic match.
pub fn score_candidate(
    evidence: &MatchEvidence,
    subject: &BangumiSubject,
    official_rank: usize,
    weights: &MatchWeights,
) -> CandidateScore {
    let titles = subject_titles(subject);
    let provider_season = detect_provider_season(&titles, evidence.season_number);
    let normalized_primary = title_extractor::normalize_title_for_match(&evidence.primary_title);
    let normalized_alternates = evidence
        .alternate_titles
        .iter()
        .map(|title| title_extractor::normalize_title_for_match(title))
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();
    let normalized_titles = titles
        .iter()
        .map(|title| title_extractor::normalize_title_for_match(title))
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();
    let normalized_provider_series = titles
        .iter()
        .map(|title| provider_series_title(title, provider_season))
        .map(|title| title_extractor::normalize_title_for_match(&title))
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();

    let primary_exact = !normalized_primary.is_empty()
        && normalized_titles
            .iter()
            .any(|title| title == &normalized_primary);
    let alternate_exact = normalized_alternates.iter().any(|alternate| {
        normalized_titles.iter().any(|title| title == alternate)
            || normalized_provider_series
                .iter()
                .any(|title| title == alternate)
    });
    let mut score = if primary_exact {
        weights.exact_primary
    } else if alternate_exact {
        weights.exact_alternate
    } else {
        0
    };

    let local_titles = std::iter::once(evidence.primary_title.as_str())
        .chain(evidence.alternate_titles.iter().map(String::as_str));
    let similarity = local_titles
        .flat_map(|local| {
            titles
                .iter()
                .map(move |official| title_similarity(local, official))
        })
        .fold(0.0_f64, f64::max);
    let similarity_score = (similarity * f64::from(weights.similarity_max)).round() as i32;
    score += similarity_score;

    let mut strong_conflicts = Vec::new();
    if subject.subject_type != 2 {
        score += weights.edition_conflict;
        strong_conflicts.push(StrongConflict::NonAnime);
    }

    let subject_year = subject.date.as_deref().and_then(parse_subject_year);
    if let (Some(local_year), Some(provider_year)) = (evidence.year, subject_year) {
        if local_year == provider_year {
            score += weights.year_match;
        } else if (local_year - provider_year).abs() > 1 {
            score += weights.year_conflict;
            strong_conflicts.push(StrongConflict::Year);
        }
    }

    match (evidence.season_number, provider_season) {
        (Some(local), Some(provider)) if local == provider => score += weights.season_match,
        (Some(_), Some(_)) => {
            score += weights.season_conflict;
            strong_conflicts.push(StrongConflict::Season);
        }
        (Some(local), None) if local > 1 => {
            score += weights.season_conflict;
            strong_conflicts.push(StrongConflict::MissingSeason);
        }
        (None, Some(provider)) if provider > 1 => {
            score += weights.season_conflict;
            strong_conflicts.push(StrongConflict::Season);
        }
        _ => {}
    }

    let provider_edition = titles.iter().find_map(|title| {
        let edition = title_extractor::extract_title_signals(title).edition_kind;
        (edition != EditionKind::Unknown).then_some(edition)
    });
    if let Some(provider_edition) = provider_edition {
        if evidence.edition_kind != EditionKind::Unknown {
            if editions_compatible(evidence.edition_kind, provider_edition) {
                score += weights.edition_match;
            } else {
                score += weights.edition_conflict;
                strong_conflicts.push(StrongConflict::Edition);
            }
        }
    }

    if evidence.parent_title.as_deref().is_some_and(|parent| {
        let parent = title_extractor::normalize_title_for_match(parent);
        !parent.is_empty() && normalized_titles.iter().any(|title| title == &parent)
    }) {
        score += weights.hierarchy_match;
    }
    if evidence.evidence_quality < 40 || title_extractor::is_generic_title(&evidence.primary_title)
    {
        score += weights.generic_penalty;
    }
    score += rank_prior(official_rank);

    CandidateScore {
        subject: subject.clone(),
        score: score.clamp(0, 100),
        official_rank,
        primary_exact,
        alternate_exact,
        similarity_score,
        strong_conflicts,
    }
}

fn detect_provider_season(titles: &[&str], expected_local: Option<u16>) -> Option<u16> {
    titles
        .iter()
        .find_map(|title| title_extractor::extract_title_signals(title).season_number)
        .or_else(|| {
            let expected = expected_local.filter(|season| *season > 1)?;
            titles
                .iter()
                .any(|title| trailing_sequel_number(title) == Some(expected))
                .then_some(expected)
        })
}

fn provider_series_title(title: &str, provider_season: Option<u16>) -> String {
    let signals = title_extractor::extract_title_signals(title);
    if signals.season_number.is_some() {
        return signals.series_title;
    }
    let Some(season) = provider_season else {
        return title.to_string();
    };
    if trailing_sequel_number(title) != Some(season) {
        return title.to_string();
    }
    let mut parts = title.split_whitespace().collect::<Vec<_>>();
    parts.pop();
    parts
        .join(" ")
        .trim_end_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '_' | ':' | '·' | '|')
        })
        .to_string()
}

fn trailing_sequel_number(value: &str) -> Option<u16> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let number = parts
        .last()?
        .trim_matches(|character: char| !character.is_ascii_digit());
    if number.is_empty() || !number.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let number = number.parse::<u16>().ok()?;
    (number > 1 && number <= 99).then_some(number)
}

/// Applies absolute score, score-margin, and strong-conflict gates. Containers additionally need
/// an exact primary title and a wider margin because a series folder often spans many Subjects.
pub fn decide_scores(
    mut candidates: Vec<CandidateScore>,
    is_container: bool,
    weights: &MatchWeights,
) -> MatchDecision {
    sort_scores(&mut candidates);
    let best = candidates.first().cloned();
    let second_score = candidates.get(1).map(|candidate| candidate.score);
    let score_margin = best
        .as_ref()
        .map_or(0, |best| best.score - second_score.unwrap_or(0));
    let confidence = match best.as_ref() {
        Some(best)
            if best.score >= weights.automatic_threshold
                && score_margin
                    >= if is_container {
                        weights.container_minimum_margin
                    } else {
                        weights.minimum_margin
                    }
                && best.strong_conflicts.is_empty()
                && (!is_container || best.primary_exact) =>
        {
            MatchConfidence::High
        }
        Some(best) if best.score >= weights.pending_threshold => MatchConfidence::Pending,
        _ => MatchConfidence::Low,
    };
    MatchDecision {
        confidence,
        best,
        second_score,
        score_margin,
    }
}

fn sort_scores(scores: &mut [CandidateScore]) {
    scores.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| right.primary_exact.cmp(&left.primary_exact))
            .then_with(|| right.alternate_exact.cmp(&left.alternate_exact))
            .then_with(|| right.similarity_score.cmp(&left.similarity_score))
            .then_with(|| left.official_rank.cmp(&right.official_rank))
            .then_with(|| left.subject.subject_id.cmp(&right.subject.subject_id))
    });
}

fn subject_titles(subject: &BangumiSubject) -> Vec<&str> {
    let mut titles: Vec<&str> = Vec::new();
    for title in [
        Some(subject.title.as_str()),
        subject.title_cn.as_deref(),
        subject.title_en.as_deref(),
        subject.title_ja.as_deref(),
        subject.title_ko.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(subject.match_aliases.iter().map(String::as_str))
    {
        if !title.trim().is_empty()
            && !titles.iter().any(|existing| {
                title_extractor::normalize_title_for_match(existing)
                    == title_extractor::normalize_title_for_match(title)
            })
        {
            titles.push(title);
        }
    }
    titles
}

fn parse_subject_year(value: &str) -> Option<i32> {
    value
        .get(0..4)?
        .parse::<i32>()
        .ok()
        .filter(|year| (1900..=2099).contains(year))
}

fn editions_compatible(left: EditionKind, right: EditionKind) -> bool {
    left == right
        || matches!(
            (left, right),
            (EditionKind::Sp, EditionKind::Special) | (EditionKind::Special, EditionKind::Sp)
        )
}

fn rank_prior(rank: usize) -> i32 {
    match rank {
        0 => 5,
        1 => 4,
        2 => 3,
        3 => 2,
        4 => 1,
        _ => 0,
    }
}

fn title_similarity(left: &str, right: &str) -> f64 {
    let left = title_extractor::normalize_title_for_match(left);
    let right = title_extractor::normalize_title_for_match(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let length_ratio = left_chars.len().min(right_chars.len()) as f64
        / left_chars.len().max(right_chars.len()) as f64;
    let containment = if left.contains(&right) || right.contains(&left) {
        length_ratio
    } else {
        0.0
    };
    let left_bigrams = bigrams(&left_chars);
    let right_bigrams = bigrams(&right_chars);
    if left_bigrams.is_empty() || right_bigrams.is_empty() {
        return containment;
    }
    let mut right_counts = HashMap::<(char, char), usize>::new();
    for bigram in &right_bigrams {
        *right_counts.entry(*bigram).or_default() += 1;
    }
    let mut intersection = 0usize;
    for bigram in &left_bigrams {
        if let Some(count) = right_counts.get_mut(bigram) {
            if *count > 0 {
                intersection += 1;
                *count -= 1;
            }
        }
    }
    let dice = (2 * intersection) as f64 / (left_bigrams.len() + right_bigrams.len()) as f64;
    containment.max(dice * length_ratio.sqrt())
}

fn bigrams(value: &[char]) -> Vec<(char, char)> {
    if value.len() == 1 {
        return vec![(value[0], value[0])];
    }
    value.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(id: i64, title: &str, title_cn: Option<&str>) -> BangumiSubject {
        BangumiSubject {
            subject_id: id,
            title: title.into(),
            title_cn: title_cn.map(str::to_string),
            title_en: None,
            title_ja: None,
            title_ko: None,
            match_aliases: Vec::new(),
            date: None,
            image_url: None,
            summary: None,
            subject_type: 2,
        }
    }

    fn evidence(title: &str) -> MatchEvidence {
        title_extractor::build_match_evidence(title, title, None, &[])
    }

    #[test]
    fn exact_title_beats_provider_rank_alone() {
        let evidence = evidence("葬送的芙莉莲");
        let weights = MatchWeights::default();
        let ranked_first = score_candidate(
            &evidence,
            &subject(1, "Unrelated provider first result", None),
            0,
            &weights,
        );
        let exact = score_candidate(
            &evidence,
            &subject(2, "葬送のフリーレン", Some("葬送的芙莉莲")),
            4,
            &weights,
        );
        assert!(exact.score > ranked_first.score);
        assert!(exact.primary_exact);
        assert!(ranked_first.score < weights.pending_threshold);
    }

    #[test]
    fn clear_alternate_exact_can_auto_bind_work_but_not_container() {
        let mut evidence = evidence("Local release name");
        evidence.alternate_titles = vec!["Official alternate title".into()];
        let weights = MatchWeights::default();
        let alternate = score_candidate(
            &evidence,
            &subject(1, "Official alternate title", None),
            0,
            &weights,
        );
        let unrelated = score_candidate(
            &evidence,
            &subject(2, "Unrelated provider result", None),
            1,
            &weights,
        );

        assert!(alternate.alternate_exact);
        assert!(!alternate.primary_exact);
        assert_eq!(
            decide_scores(vec![alternate.clone(), unrelated.clone()], false, &weights).confidence,
            MatchConfidence::High
        );
        assert_ne!(
            decide_scores(vec![alternate, unrelated], true, &weights).confidence,
            MatchConfidence::High
        );
    }

    #[test]
    fn official_match_alias_participates_in_exact_title_scoring() {
        let evidence = evidence("Official Romanized Alias");
        let weights = MatchWeights::default();
        let mut candidate = subject(1, "公式タイトル", None);
        candidate.match_aliases = vec!["Official Romanized Alias".into()];
        let score = score_candidate(&evidence, &candidate, 0, &weights);

        assert!(score.primary_exact);
        assert!(score.score >= weights.automatic_threshold);
        assert_eq!(
            decide_scores(vec![score], false, &weights).confidence,
            MatchConfidence::High
        );
    }

    #[test]
    #[ignore = "requires external network access"]
    fn live_structured_match_accepts_an_official_romanized_alias() {
        let evidence = evidence("Code Geass: Hangyaku no Lelouch");
        let mut cache = MatchRunCache::default();
        let decision = match assess_evidence_online(&evidence, false, &mut cache, &|| false)
            .expect("Bangumi live assessment should succeed")
        {
            OnlineAssessment::Decision(decision) => decision,
            OnlineAssessment::Cancelled => panic!("live assessment was unexpectedly cancelled"),
        };
        assert_eq!(decision.confidence, MatchConfidence::High);
        assert!(decision.best.is_some());
    }

    #[test]
    fn second_season_beats_first_and_conflict_blocks_auto_binding() {
        let evidence = evidence("Made in Abyss Season 2");
        let weights = MatchWeights::default();
        let first = score_candidate(
            &evidence,
            &subject(1, "Made in Abyss Season 1", None),
            0,
            &weights,
        );
        let second = score_candidate(
            &evidence,
            &subject(2, "Made in Abyss Season 2", None),
            1,
            &weights,
        );
        assert!(second.score > first.score);
        assert!(first.strong_conflicts.contains(&StrongConflict::Season));
        let decision = decide_scores(vec![first], false, &weights);
        assert_ne!(decision.confidence, MatchConfidence::High);
    }

    #[test]
    fn translated_folder_uses_file_and_parent_titles_before_generic_fallbacks() {
        let evidence = title_extractor::build_match_evidence(
            "The Disastrous Life of Saiki K. S2",
            "The Disastrous Life of Saiki K. S2",
            Some("斉木楠雄のΨ難"),
            &[
                "[BeanSub] Saiki Kusuo no Psi-nan S02 [01][1080p].mkv".into(),
                "[BeanSub] Saiki Kusuo no Psi-nan S02 [02][1080p].mkv".into(),
            ],
        );
        let queries = match_queries(&evidence);
        assert_eq!(queries.len(), MAX_QUERIES_PER_NODE);
        assert_eq!(queries[0], "The Disastrous Life of Saiki K. S2");
        assert_eq!(queries[1], "Saiki Kusuo no Psi-nan S2");
        assert_eq!(queries[2], "斉木楠雄のΨ難");
    }

    #[test]
    fn explicit_s2_matches_official_bare_two_but_first_season_remains_conflicted() {
        let mut local = evidence("Unofficial English Translation S02");
        local.alternate_titles = vec!["Saiki Kusuo no Psi-nan".into()];
        let weights = MatchWeights::default();
        let second = score_candidate(
            &local,
            &subject(2, "Saiki Kusuo no Psi-nan 2", None),
            0,
            &weights,
        );
        let first = score_candidate(
            &local,
            &subject(1, "Saiki Kusuo no Psi-nan", None),
            1,
            &weights,
        );

        assert!(second.alternate_exact);
        assert!(second.strong_conflicts.is_empty());
        assert!(first
            .strong_conflicts
            .contains(&StrongConflict::MissingSeason));
        assert_eq!(
            decide_scores(vec![second.clone(), first.clone()], false, &weights).confidence,
            MatchConfidence::High
        );
        assert_ne!(
            decide_scores(vec![second, first], true, &weights).confidence,
            MatchConfidence::High
        );
    }

    #[test]
    fn movie_candidate_beats_tv_candidate() {
        let evidence = evidence("Violet Evergarden Movie");
        let weights = MatchWeights::default();
        let movie = score_candidate(
            &evidence,
            &subject(1, "Violet Evergarden Movie", None),
            1,
            &weights,
        );
        let tv = score_candidate(
            &evidence,
            &subject(2, "Violet Evergarden TV", None),
            0,
            &weights,
        );
        assert!(movie.score > tv.score);
        assert!(tv.strong_conflicts.contains(&StrongConflict::Edition));
    }

    #[test]
    fn year_conflict_is_strong_and_reduces_score() {
        let evidence = evidence("Legendary Work (2018)");
        let weights = MatchWeights::default();
        let mut same_year = subject(1, "Legendary Work", None);
        same_year.date = Some("2018-01-01".into());
        let mut wrong_year = subject(2, "Legendary Work", None);
        wrong_year.date = Some("2024-01-01".into());
        let same = score_candidate(&evidence, &same_year, 1, &weights);
        let wrong = score_candidate(&evidence, &wrong_year, 0, &weights);
        assert!(same.score > wrong.score);
        assert!(wrong.strong_conflicts.contains(&StrongConflict::Year));
    }

    #[test]
    fn close_top_two_candidates_are_pending_even_when_both_score_high() {
        let evidence = evidence("同名作品");
        let weights = MatchWeights::default();
        let first = score_candidate(&evidence, &subject(1, "同名作品", None), 0, &weights);
        let second = score_candidate(&evidence, &subject(2, "同名作品", None), 1, &weights);
        let decision = decide_scores(vec![first, second], false, &weights);
        assert_eq!(decision.confidence, MatchConfidence::Pending);
        assert!(decision.score_margin < weights.minimum_margin);
    }

    #[test]
    fn low_similarity_candidate_stays_unmatched() {
        let evidence = evidence("Cowboy Bebop");
        let weights = MatchWeights::default();
        let unrelated = score_candidate(
            &evidence,
            &subject(1, "K-On!", Some("轻音少女")),
            0,
            &weights,
        );
        let decision = decide_scores(vec![unrelated], false, &weights);
        assert_eq!(decision.confidence, MatchConfidence::Low);
    }

    #[test]
    fn rank_prior_is_small_and_locked_to_five_points() {
        assert_eq!(rank_prior(0), 5);
        assert_eq!(rank_prior(1), 4);
        assert_eq!(rank_prior(4), 1);
        assert_eq!(rank_prior(5), 0);
        let evidence = evidence("Exact Work");
        let weights = MatchWeights::default();
        let first = score_candidate(&evidence, &subject(1, "Different", None), 0, &weights);
        let exact = score_candidate(&evidence, &subject(2, "Exact Work", None), 19, &weights);
        assert!(exact.score > first.score);
    }

    #[test]
    fn container_requires_primary_exact_and_wider_margin() {
        let weights = MatchWeights::default();
        let alternate_exact = CandidateScore {
            subject: subject(1, "Fate", None),
            score: 95,
            official_rank: 0,
            primary_exact: false,
            alternate_exact: true,
            similarity_score: 30,
            strong_conflicts: Vec::new(),
        };
        assert_eq!(
            decide_scores(vec![alternate_exact], true, &weights).confidence,
            MatchConfidence::Pending
        );
    }

    #[test]
    fn query_generation_is_safe_unique_and_bounded() {
        let mut evidence = evidence("ＳＴＥＩＮＳ；ＧＡＴＥ 0");
        evidence.alternate_titles = vec![
            "steins;gate 0".into(),
            "Steins Gate".into(),
            "123".into(),
            "Parent Title".into(),
        ];
        let queries = match_queries(&evidence);
        assert_eq!(queries.len(), 3);
        assert_eq!(queries[0], "STEINS;GATE 0");
        assert!(!queries.iter().any(|query| query == "123"));
    }

    #[test]
    fn candidate_pool_round_robins_across_all_three_queries() {
        let search_results = (0..3)
            .map(|query_index| {
                (0..AUTO_SEARCH_LIMIT)
                    .map(|rank| {
                        subject(
                            (query_index * 100 + rank + 1) as i64,
                            &format!("query {query_index} rank {rank}"),
                            None,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let recalled = merge_search_results_fair(&search_results);
        assert_eq!(recalled.len(), MAX_CANDIDATES_PER_NODE);
        assert_eq!(
            recalled
                .iter()
                .take(6)
                .map(|candidate| candidate.subject.subject_id)
                .collect::<Vec<_>>(),
            vec![1, 101, 201, 2, 102, 202]
        );
        for query_index in 0..3 {
            let lower = (query_index * 100 + 1) as i64;
            let upper = lower + AUTO_SEARCH_LIMIT as i64;
            assert_eq!(
                recalled
                    .iter()
                    .filter(|candidate| { (lower..upper).contains(&candidate.subject.subject_id) })
                    .count(),
                MAX_CANDIDATES_PER_NODE / 3
            );
        }
    }

    #[test]
    fn candidate_pool_deduplicates_and_keeps_best_provider_rank() {
        let duplicate = subject(99, "duplicate", None);
        let mut first_query = (0..6)
            .map(|rank| subject(rank + 1, &format!("first {rank}"), None))
            .collect::<Vec<_>>();
        first_query.push(duplicate.clone());
        let second_query = vec![subject(50, "second first", None), duplicate];

        let recalled = merge_search_results_fair(&[first_query, second_query]);
        let duplicates = recalled
            .iter()
            .filter(|candidate| candidate.subject.subject_id == 99)
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].official_rank, 1);
    }

    #[test]
    fn provider_search_failures_trip_the_run_level_circuit_breaker() {
        assert!(is_provider_search_failure(
            "搜索 Bangumi 失败：connection refused"
        ));
        assert!(is_provider_search_failure(
            "无法初始化 Bangumi 网络客户端：TLS error"
        ));
        assert!(!is_provider_search_failure("数据库操作失败：busy"));
    }

    #[test]
    fn score_sort_is_deterministic() {
        let evidence = evidence("Work");
        let weights = MatchWeights::default();
        let mut scores = vec![
            score_candidate(&evidence, &subject(2, "Work", None), 1, &weights),
            score_candidate(&evidence, &subject(1, "Work", None), 1, &weights),
        ];
        sort_scores(&mut scores);
        assert_eq!(
            scores[0]
                .subject
                .subject_id
                .cmp(&scores[1].subject.subject_id),
            std::cmp::Ordering::Less
        );
    }
}
