use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use crate::models::BangumiSearchPrefill;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditionKind {
    Tv,
    Movie,
    Ova,
    Oad,
    Sp,
    Special,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageHint {
    Chinese,
    Japanese,
    Korean,
    Latin,
}

/// Structured, read-only evidence used by the automatic Bangumi matcher.
///
/// Every field is derived in memory. None of these values are written back to the Node's
/// `display_name`, `folder_name`, path, or source filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchEvidence {
    pub original_name: String,
    pub primary_title: String,
    pub alternate_titles: Vec<String>,
    pub parent_title: Option<String>,
    pub frequent_file_title: Option<String>,
    pub year: Option<i32>,
    pub season_number: Option<u16>,
    pub edition_kind: EditionKind,
    pub language_hints: Vec<LanguageHint>,
    pub removed_noise: Vec<String>,
    /// 0-100 estimate of how specific and useful the local evidence is.
    pub evidence_quality: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleSignals {
    pub cleaned_title: String,
    pub series_title: String,
    pub year: Option<i32>,
    pub season_number: Option<u16>,
    pub edition_kind: EditionKind,
}

/// Produces temporary Bangumi search text only. It never writes names back to the database or
/// the source filesystem.
pub fn extract_search_keyword(raw_name: &str) -> String {
    extract_keyword_with_mode(raw_name, false).0
}

/// Builds the evidence consumed by the confidence matcher. `parent_name` is optional because a
/// Library Root or a stale parent can legitimately have no displayable parent Node.
pub fn build_match_evidence(
    folder_name: &str,
    display_name: &str,
    parent_name: Option<&str>,
    media_file_names: &[String],
) -> MatchEvidence {
    let original_name =
        if !display_name.trim().is_empty() && display_name.trim() != folder_name.trim() {
            display_name.trim().to_string()
        } else {
            folder_name.trim().to_string()
        };
    let primary_signals = extract_title_signals(&original_name);
    let folder_signals = extract_title_signals(folder_name);
    let display_signals = extract_title_signals(display_name);
    let parent_title = parent_name
        .map(extract_search_keyword)
        .filter(|value| is_useful_candidate(value));
    let ranked_file_titles = ranked_file_candidates(media_file_names);
    let frequent_file_title = ranked_file_titles.first().cloned();
    let primary_title = if is_safe_match_query(&primary_signals.cleaned_title) {
        primary_signals.cleaned_title.clone()
    } else {
        frequent_file_title
            .clone()
            .filter(|value| is_safe_match_query(value))
            .unwrap_or_else(|| primary_signals.cleaned_title.clone())
    };

    let mut alternate_titles = Vec::new();
    for candidate in [
        primary_signals.series_title.clone(),
        folder_signals.cleaned_title,
        folder_signals.series_title,
        display_signals.cleaned_title,
        display_signals.series_title,
    ] {
        if is_useful_candidate(&candidate) {
            push_unique(&mut alternate_titles, candidate);
        }
    }
    for value in ranked_file_titles.into_iter().take(3) {
        if is_useful_candidate(&value) {
            push_unique(&mut alternate_titles, value);
        }
    }
    if let Some(value) = parent_title.as_ref() {
        push_unique(&mut alternate_titles, value.clone());
    }
    for candidate in extract_embedded_title_candidates(folder_name)
        .into_iter()
        .chain(extract_embedded_title_candidates(display_name))
    {
        if is_useful_candidate(&candidate) {
            push_unique(&mut alternate_titles, candidate);
        }
    }
    alternate_titles.retain(|value| {
        normalize_title_for_match(value) != normalize_title_for_match(&primary_title)
    });

    let (_, mut removed_noise) = extract_keyword_with_mode(&original_name, true);
    deduplicate_strings(&mut removed_noise);
    let language_hints = detect_language_hints(&original_name);
    let evidence_quality = evidence_quality(
        &primary_title,
        frequent_file_title.as_deref(),
        parent_title.as_deref(),
    );

    MatchEvidence {
        original_name,
        primary_title,
        alternate_titles,
        parent_title,
        frequent_file_title,
        year: primary_signals.year.or(folder_signals.year),
        season_number: primary_signals
            .season_number
            .or(folder_signals.season_number),
        edition_kind: if primary_signals.edition_kind != EditionKind::Unknown {
            primary_signals.edition_kind
        } else {
            folder_signals.edition_kind
        },
        language_hints,
        removed_noise,
        evidence_quality,
    }
}

/// Extracts comparable title, year, season, and edition signals from either a local name or an
/// official provider title. This is intentionally public so the scorer can use exactly the same
/// semantics on both sides.
pub fn extract_title_signals(value: &str) -> TitleSignals {
    let nfkc = value.nfkc().collect::<String>();
    let year = detect_year(&nfkc);
    let season_number = detect_season(&nfkc);
    let edition_kind = detect_edition_kind(&nfkc);
    let (semantic_title, _) = extract_keyword_with_mode(&nfkc, true);
    let cleaned_title = remove_year_marker(&semantic_title, year);
    let series_title = remove_semantic_markers(&cleaned_title, season_number, edition_kind);
    TitleSignals {
        cleaned_title: if cleaned_title.is_empty() {
            extract_search_keyword(&nfkc)
        } else {
            cleaned_title
        },
        series_title,
        year,
        season_number,
        edition_kind,
    }
}

/// NFKC title normalization shared by query caches, exact matching, and similarity scoring.
pub fn normalize_title_for_match(value: &str) -> String {
    let nfkc = value.nfkc().collect::<String>().to_lowercase();
    let words = nfkc
        .split(|character: char| !character.is_alphanumeric() && character != '&')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    words
        .into_iter()
        .map(|word| if word == "&" { "and" } else { word })
        .collect::<String>()
}

pub fn build_search_prefill(
    folder_name: &str,
    display_name: &str,
    media_file_names: &[String],
) -> BangumiSearchPrefill {
    let folder_candidate = extract_search_keyword(folder_name);
    let display_candidate = extract_search_keyword(display_name);
    let mut candidates = Vec::new();
    if is_useful_candidate(&folder_candidate) {
        push_unique(&mut candidates, folder_candidate.clone());
    }
    if display_name.trim() != folder_name.trim() && is_useful_candidate(&display_candidate) {
        push_unique(&mut candidates, display_candidate.clone());
    }
    for candidate in ranked_file_candidates(media_file_names).into_iter().take(3) {
        push_unique(&mut candidates, candidate);
    }
    for candidate in extract_embedded_title_candidates(folder_name)
        .into_iter()
        .chain(extract_embedded_title_candidates(display_name))
    {
        if is_useful_candidate(&candidate) {
            push_unique(&mut candidates, candidate);
        }
    }
    if candidates.is_empty() {
        let fallback = folder_name.trim().to_string();
        if !fallback.is_empty() {
            candidates.push(fallback);
        }
    }

    BangumiSearchPrefill {
        original_name: folder_name.to_string(),
        extracted_name: candidates
            .first()
            .cloned()
            .unwrap_or_else(|| folder_name.trim().to_string()),
        candidates,
    }
}

fn ranked_file_candidates(media_file_names: &[String]) -> Vec<String> {
    let mut frequency: HashMap<String, (String, usize)> = HashMap::new();
    for file_name in media_file_names {
        let candidate = extract_media_file_keyword(file_name);
        if is_useful_candidate(&candidate) {
            let key = normalize_title_for_match(&candidate);
            frequency
                .entry(key)
                .and_modify(|entry| entry.1 += 1)
                .or_insert((candidate, 1));
        }
    }
    let mut candidates = frequency.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.0.chars().count().cmp(&left.0.chars().count()))
            .then_with(|| left.0.cmp(&right.0))
    });
    candidates.into_iter().map(|entry| entry.0).collect()
}

/// File names frequently contain a useful romanized/original title even when the folder uses a
/// fan-created English translation that Bangumi does not index. Keep an explicit season marker
/// from a file name after the ordinary keyword cleaner removes episode/encode noise, so this
/// independent local source can become one of the three bounded official queries.
fn extract_media_file_keyword(file_name: &str) -> String {
    let mut candidate = extract_search_keyword(file_name);
    let source_season = detect_season(file_name);
    if let Some(season) = source_season.filter(|season| *season > 1) {
        if detect_season(&candidate).is_none() && is_useful_candidate(&candidate) {
            candidate.push_str(&format!(" S{season}"));
        }
    }
    candidate
}

fn extract_keyword_with_mode(raw_name: &str, preserve_semantics: bool) -> (String, Vec<String>) {
    // Keep the advisory/manual prefill byte semantics stable (for example Japanese `！`). The
    // structured matcher requests NFKC through `extract_title_signals` before entering this path.
    let owned;
    let source = if preserve_semantics {
        owned = raw_name.nfkc().collect::<String>();
        owned.as_str()
    } else {
        raw_name
    };
    let without_extension = strip_known_extension(source.trim());
    let normalized_separators = normalize_release_separators(without_extension);
    let mut output = String::with_capacity(normalized_separators.len());
    let mut removed_noise = Vec::new();
    let characters = normalized_separators.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut visible_text_seen = false;

    while index < characters.len() {
        if characters[index] == '[' {
            if let Some(relative_end) = characters[index + 1..]
                .iter()
                .position(|character| *character == ']')
            {
                let end = index + relative_end + 1;
                let group = characters[index + 1..end].iter().collect::<String>();
                let group = group.trim();
                let release_prefix = !visible_text_seen
                    && (looks_like_release_group(group)
                        || (looks_like_contextual_release_identity(group)
                            && has_following_title_candidate(&characters, end + 1)));
                let semantic = preserve_semantics && is_semantic_group(group);
                if !release_prefix && (!is_technical_group(group) || semantic) {
                    push_separated(&mut output, group);
                    visible_text_seen |= !group.is_empty();
                } else if !group.is_empty() {
                    removed_noise.push(group.to_string());
                }
                index = end + 1;
                continue;
            }
        }

        let character = characters[index];
        output.push(if character == '_' { ' ' } else { character });
        visible_text_seen |= !character.is_whitespace();
        index += 1;
    }

    let (cleaned, word_noise) = remove_technical_words(&output, preserve_semantics);
    removed_noise.extend(word_noise);
    (cleaned, removed_noise)
}

/// Scene-style movie and television releases commonly use periods as word separators, for
/// example `The.Sword.of.Doom.1966.1080p`. Scoring ignores punctuation, but sending the whole
/// dotted value as one token prevents the year and encode noise from being removed first. Only
/// normalize a multi-period name when one segment is recognizable release metadata, preserving
/// ordinary dotted titles that do not look like a release name.
fn normalize_release_separators(value: &str) -> String {
    let parts = value.split('.').collect::<Vec<_>>();
    let normalize_periods = parts.len() >= 3
        && parts.iter().any(|part| {
            let part = part.trim();
            detect_year(part).is_some()
                || (is_technical_atom(part)
                    && !part.chars().all(|character| character.is_ascii_digit()))
        });
    value
        .chars()
        .map(|character| {
            if character == '_' || (normalize_periods && character == '.') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn strip_known_extension(value: &str) -> &str {
    let Some((stem, extension)) = value.rsplit_once('.') else {
        return value;
    };
    let extension = extension.to_ascii_lowercase();
    const FILE_EXTENSIONS: &[&str] = &[
        "mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "ass", "ssa", "srt", "sup", "vtt",
        "flac", "wav", "mp3", "aac", "jpg", "jpeg", "png", "webp",
    ];
    if FILE_EXTENSIONS.contains(&extension.as_str()) {
        stem.trim_end()
    } else {
        value
    }
}

fn push_separated(target: &mut String, value: &str) {
    if value.is_empty() {
        return;
    }
    if !target.is_empty() && !target.ends_with(char::is_whitespace) {
        target.push(' ');
    }
    target.push_str(&value.replace('_', " "));
    target.push(' ');
}

fn looks_like_release_group(value: &str) -> bool {
    let lower = value.to_lowercase();
    let compact = lower
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect::<String>();
    lower.contains("studio")
        || lower.contains("raws")
        || lower.contains("fansub")
        || lower.contains("subgroup")
        || lower.contains("字幕")
        || lower.contains("压制")
        || lower.contains("壓制")
        || (lower.contains('&')
            && lower
                .chars()
                .any(|character| character.is_ascii_alphabetic()))
        || matches!(
            compact.as_str(),
            "airota"
                | "vcb"
                | "dbd"
                | "caso"
                | "ktxp"
                | "dmhy"
                | "ani"
                | "beansub"
                | "fzsd"
                | "lolihouse"
                | "reinforce"
                | "moozzi2"
                | "nekomoe"
                | "nekomoekissaten"
                | "nanoalchemist"
                | "uhawings"
                | "lilithraws"
                | "beatriceraws"
        )
}

/// Release folders often use an unregistered short team identity in the first bracket followed
/// by a real title in the next bracket or in plain text. Limit the heuristic to strong identity
/// shapes and require a following title candidate, so a bracketed title such as
/// `[STEINS;GATE][1080p]` remains intact.
fn looks_like_contextual_release_identity(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.is_ascii() {
        return false;
    }
    let alphanumeric = trimmed
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    if !(2..=24).contains(&alphanumeric.len())
        || !alphanumeric
            .chars()
            .any(|character| character.is_ascii_alphabetic())
    {
        return false;
    }
    let uppercase_identity = trimmed
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .all(|character| character.is_ascii_uppercase())
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    let camel_humps = trimmed
        .chars()
        .filter(|character| character.is_ascii_uppercase())
        .count();
    let compact_camel_identity = !trimmed.contains(char::is_whitespace)
        && !trimmed.contains([':', ';', '!', '?', '/'])
        && camel_humps >= 2;
    uppercase_identity || compact_camel_identity
}

fn has_following_title_candidate(characters: &[char], mut index: usize) -> bool {
    while index < characters.len() {
        while index < characters.len() && characters[index].is_whitespace() {
            index += 1;
        }
        if index >= characters.len() {
            return false;
        }
        if characters[index] == '[' {
            let Some(relative_end) = characters[index + 1..]
                .iter()
                .position(|character| *character == ']')
            else {
                return false;
            };
            let end = index + relative_end + 1;
            let group = characters[index + 1..end].iter().collect::<String>();
            if !group.trim().is_empty()
                && !looks_like_release_group(&group)
                && !is_technical_group(&group)
                && !is_semantic_group(&group)
            {
                return true;
            }
            index = end + 1;
            continue;
        }

        let end = characters[index..]
            .iter()
            .position(|character| *character == '[')
            .map_or(characters.len(), |relative| index + relative);
        let plain = characters[index..end].iter().collect::<String>();
        let (cleaned, _) = remove_technical_words(&plain.replace('_', " "), false);
        if is_useful_candidate(&cleaned) {
            return true;
        }
        index = end;
    }
    false
}

fn extract_embedded_title_candidates(raw_name: &str) -> Vec<String> {
    let source = strip_known_extension(raw_name.trim());
    let characters = source.chars().collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut index = 0;
    let mut visible_text_seen = false;
    while index < characters.len() {
        if characters[index] == '[' {
            let Some(relative_end) = characters[index + 1..]
                .iter()
                .position(|character| *character == ']')
            else {
                break;
            };
            let end = index + relative_end + 1;
            let group = characters[index + 1..end].iter().collect::<String>();
            let contextual_release = !visible_text_seen
                && (looks_like_release_group(&group)
                    || (looks_like_contextual_release_identity(&group)
                        && has_following_title_candidate(&characters, end + 1)));
            if !contextual_release && !is_technical_group(&group) && !is_semantic_group(&group) {
                let (candidate, _) = remove_technical_words(&group.replace('_', " "), false);
                if is_useful_candidate(&candidate) {
                    push_unique(&mut candidates, candidate);
                }
            }
            index = end + 1;
            continue;
        }
        visible_text_seen |= !characters[index].is_whitespace();
        index += 1;
    }
    candidates
}

fn is_technical_group(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || is_crc(trimmed) {
        return true;
    }
    let atoms = trimmed
        .split(|character: char| {
            character.is_whitespace()
                || matches!(character, '_' | '-' | '+' | ',' | '/' | '\\' | '&')
        })
        .filter(|atom| !atom.is_empty())
        .collect::<Vec<_>>();
    !atoms.is_empty() && atoms.iter().all(|atom| is_technical_atom(atom))
}

fn is_semantic_group(value: &str) -> bool {
    detect_season(value).is_some() || detect_edition_kind(value) != EditionKind::Unknown
}

fn is_technical_atom(value: &str) -> bool {
    let lower = value
        .trim_matches(|character: char| !character.is_alphanumeric() && character != '#')
        .to_ascii_lowercase();
    if lower.is_empty() || lower.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    if is_crc(&lower) {
        return true;
    }
    if matches!(
        lower.as_str(),
        "bdrip"
            | "bluray"
            | "blu"
            | "ray"
            | "webrip"
            | "webdl"
            | "web"
            | "dl"
            | "hdtv"
            | "remux"
            | "bdmv"
            | "hevc"
            | "avc"
            | "h264"
            | "h265"
            | "x264"
            | "x265"
            | "av1"
            | "yuv"
            | "yuv420p"
            | "yuv420p10"
            | "hdr"
            | "hdr10"
            | "hdr10plus"
            | "dolbyvision"
            | "dv"
            | "flac"
            | "aac"
            | "truehd"
            | "dts"
            | "ac3"
            | "eac3"
            | "opus"
            | "hi10p"
            | "ma10p"
            | "10bit"
            | "8bit"
            | "dual"
            | "audio"
            | "chs"
            | "cht"
            | "jpn"
            | "eng"
            | "gb"
            | "big5"
            | "batch"
            | "complete"
            | "proper"
            | "repack"
            | "sp"
            | "ova"
            | "oad"
            | "ona"
            | "ncop"
            | "nced"
            | "season"
            | "disc"
            | "disk"
    ) {
        return true;
    }
    if lower == "4k" || lower == "uhd" {
        return true;
    }
    if lower.split_once('x').is_some_and(|(width, height)| {
        matches!(width, "720" | "1280" | "1920" | "2560" | "3840" | "7680")
            && matches!(height, "480" | "720" | "1080" | "1440" | "2160" | "4320")
    }) {
        return true;
    }
    if lower
        .strip_suffix('p')
        .or_else(|| lower.strip_suffix('i'))
        .is_some_and(|number| {
            matches!(
                number,
                "480" | "576" | "720" | "1080" | "1440" | "2160" | "4320"
            )
        })
    {
        return true;
    }
    if lower
        .strip_suffix("bit")
        .is_some_and(|number| number.chars().all(|character| character.is_ascii_digit()))
    {
        return true;
    }
    looks_like_episode_token(&lower)
}

fn looks_like_episode_token(value: &str) -> bool {
    let without_version = value
        .strip_suffix("v2")
        .or_else(|| value.strip_suffix("v3"))
        .unwrap_or(value);
    for prefix in ["episode", "ep", "e", "#", "vol", "season", "disc", "disk"] {
        if without_version.strip_prefix(prefix).is_some_and(|number| {
            !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
        }) {
            return true;
        }
    }
    if let Some(rest) = without_version.strip_prefix('s') {
        if is_ascii_number(rest) {
            return true;
        }
        let mut parts = rest.split('e');
        return parts.next().is_some_and(is_ascii_number)
            && parts.next().is_some_and(is_ascii_number)
            && parts.next().is_none();
    }
    false
}

fn is_ascii_number(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn is_crc(value: &str) -> bool {
    let value = value.trim();
    value.len() == 8 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn remove_technical_words(value: &str, preserve_semantics: bool) -> (String, Vec<String>) {
    let normalized = value
        .replace(['(', ')', '{', '}', '【', '】'], " ")
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    let mut index = 0;
    while index < normalized.len() {
        let token = normalized[index]
            .trim_matches(|character: char| matches!(character, ',' | ';' | '|' | '_'));
        let lower = token.to_ascii_lowercase();
        if matches!(lower.as_str(), "disc" | "disk") {
            removed.push(token.to_string());
            index += 1;
            if index < normalized.len() && is_ascii_number(trim_numeric(&normalized[index])) {
                removed.push(normalized[index].clone());
                index += 1;
            }
            continue;
        }
        if lower == "season" {
            if preserve_semantics {
                kept.push(token.to_string());
                index += 1;
                if index < normalized.len() && is_ascii_number(trim_numeric(&normalized[index])) {
                    kept.push(normalized[index].clone());
                    index += 1;
                }
            } else {
                removed.push(token.to_string());
                index += 1;
                if index < normalized.len() && is_ascii_number(trim_numeric(&normalized[index])) {
                    removed.push(normalized[index].clone());
                    index += 1;
                }
            }
            continue;
        }
        let semantic = preserve_semantics
            && (detect_season(token).is_some()
                || detect_edition_kind(token) != EditionKind::Unknown);
        if is_technical_atom(token)
            && !token.chars().all(|character| character.is_ascii_digit())
            && !semantic
        {
            removed.push(token.to_string());
            index += 1;
            continue;
        }
        if !token.is_empty() {
            kept.push(token.to_string());
        }
        index += 1;
    }
    while kept.len() > 1
        && kept.last().is_some_and(|token| {
            let trimmed = token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '第' && character != '話'
            });
            (!preserve_semantics && trimmed.chars().all(|character| character.is_ascii_digit()))
                || (looks_like_episode_token(&trimmed.to_ascii_lowercase())
                    && !(preserve_semantics && detect_season(trimmed).is_some()))
        })
    {
        if let Some(value) = kept.pop() {
            removed.push(value);
        }
    }
    (clean_join(kept), removed)
}

fn clean_join(values: Vec<String>) -> String {
    values
        .join(" ")
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '_' | '.' | '·' | '|')
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_numeric(value: &str) -> &str {
    value.trim_matches(|character: char| !character.is_ascii_digit())
}

fn detect_year(value: &str) -> Option<i32> {
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index + 3 < chars.len() {
        if chars[index..index + 4]
            .iter()
            .all(|character| character.is_ascii_digit())
        {
            let left_clear = index == 0 || !chars[index - 1].is_ascii_digit();
            let right_clear = index + 4 == chars.len() || !chars[index + 4].is_ascii_digit();
            if left_clear && right_clear {
                let year = chars[index..index + 4]
                    .iter()
                    .collect::<String>()
                    .parse::<i32>()
                    .ok()?;
                if (1900..=2099).contains(&year) {
                    return Some(year);
                }
            }
            index += 4;
        } else {
            index += 1;
        }
    }
    None
}

fn detect_season(value: &str) -> Option<u16> {
    let lower = value.nfkc().collect::<String>().to_lowercase();
    let tokens = lower
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        if matches!(*token, "season" | "シーズン" | "시즌") {
            if let Some(number) = tokens.get(index + 1).and_then(|value| parse_ordinal(value)) {
                return Some(number);
            }
        }
        if let Some(number) = parse_ordinal(token) {
            if tokens
                .get(index + 1)
                .is_some_and(|next| matches!(*next, "season" | "シーズン" | "시즌"))
            {
                return Some(number);
            }
        }
        for prefix in ["season", "シーズン", "시즌"] {
            if let Some(number) = token.strip_prefix(prefix).and_then(parse_ordinal) {
                if number > 0 {
                    return Some(number);
                }
            }
        }
        if let Some(number_text) = token.strip_suffix("season") {
            if let Some(number) = parse_ordinal(number_text) {
                if number > 0 {
                    return Some(number);
                }
            }
        }
        if let Some(number) = token.strip_prefix('s').and_then(parse_small_u16) {
            if number > 0 && !token.contains('e') {
                return Some(number);
            }
        }
    }
    detect_cjk_season(&lower)
}

fn detect_cjk_season(value: &str) -> Option<u16> {
    let chars = value.chars().collect::<Vec<_>>();
    for start in 0..chars.len() {
        if chars[start] != '第' {
            continue;
        }
        for end in start + 1..chars.len().min(start + 6) {
            if matches!(chars[end], '季' | '期') {
                let number = chars[start + 1..end].iter().collect::<String>();
                return parse_cjk_number(&number);
            }
        }
    }
    for (marker_index, marker) in chars.iter().enumerate() {
        if !matches!(*marker, '季' | '期' | '기') || marker_index == 0 {
            continue;
        }
        let mut start = marker_index;
        while start > 0 && marker_index - start < 3 && is_cjk_number_character(chars[start - 1]) {
            start -= 1;
        }
        if start < marker_index {
            let number = chars[start..marker_index].iter().collect::<String>();
            // Chinese ordinal seasons normally use `第N季`; accepting any bare Han numeral
            // before `季` would misread ordinary words such as `四季`. ASCII `2季`, Japanese
            // `二期`, and Korean `2기` remain supported here.
            if *marker == '季' && !number.chars().all(|character| character.is_ascii_digit()) {
                continue;
            }
            if let Some(number) = parse_cjk_number(&number) {
                if number > 0 {
                    return Some(number);
                }
            }
        }
    }
    None
}

fn is_cjk_number_character(character: char) -> bool {
    character.is_ascii_digit()
        || matches!(
            character,
            '一' | '二' | '两' | '兩' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十'
        )
}

fn parse_cjk_number(value: &str) -> Option<u16> {
    if let Some(number) = parse_small_u16(value) {
        return Some(number);
    }
    match value {
        "一" => Some(1),
        "二" | "两" | "兩" => Some(2),
        "三" => Some(3),
        "四" => Some(4),
        "五" => Some(5),
        "六" => Some(6),
        "七" => Some(7),
        "八" => Some(8),
        "九" => Some(9),
        "十" => Some(10),
        _ if value.starts_with('十') => parse_cjk_number(&value["十".len()..]).map(|n| 10 + n),
        _ if value.ends_with('十') => {
            parse_cjk_number(&value[..value.len() - "十".len()]).map(|n| n * 10)
        }
        _ => None,
    }
}

fn parse_ordinal(value: &str) -> Option<u16> {
    let digits = value
        .strip_suffix("st")
        .or_else(|| value.strip_suffix("nd"))
        .or_else(|| value.strip_suffix("rd"))
        .or_else(|| value.strip_suffix("th"))
        .unwrap_or(value);
    parse_small_u16(digits)
}

fn parse_small_u16(value: &str) -> Option<u16> {
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let parsed = value.parse::<u16>().ok()?;
    (parsed <= 99).then_some(parsed)
}

fn detect_edition_kind(value: &str) -> EditionKind {
    let lower = value.nfkc().collect::<String>().to_lowercase();
    if contains_word(&lower, "oad") {
        EditionKind::Oad
    } else if contains_word(&lower, "ova") || contains_word(&lower, "ona") {
        EditionKind::Ova
    } else if lower.contains("剧场版")
        || lower.contains("劇場版")
        || contains_word(&lower, "movie")
        || contains_word(&lower, "film")
    {
        EditionKind::Movie
    } else if contains_word(&lower, "sp") {
        EditionKind::Sp
    } else if contains_word(&lower, "special")
        || lower.contains("特别篇")
        || lower.contains("特別編")
        || lower.contains("特典")
    {
        EditionKind::Special
    } else if contains_word(&lower, "tv") {
        EditionKind::Tv
    } else {
        EditionKind::Unknown
    }
}

fn contains_word(value: &str, needle: &str) -> bool {
    value
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == needle)
}

fn remove_year_marker(value: &str, year: Option<i32>) -> String {
    let Some(year) = year else {
        return value.to_string();
    };
    let year = year.to_string();
    clean_join(
        value
            .split_whitespace()
            .filter(|token| trim_numeric(token) != year)
            .map(str::to_string)
            .collect(),
    )
}

fn remove_semantic_markers(value: &str, season: Option<u16>, edition: EditionKind) -> String {
    let mut tokens = value
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(season) = season {
        let number = season.to_string();
        tokens.retain(|token| {
            let lower = token.to_lowercase();
            !(matches!(lower.as_str(), "season" | "シーズン" | "시즌")
                || lower == format!("s{season:02}")
                || lower == format!("s{season}")
                || lower == format!("season{season}")
                || lower == format!("シーズン{season}")
                || lower == format!("시즌{season}")
                || parse_ordinal(&lower) == Some(season)
                || (detect_season(&lower) == Some(season) && is_standalone_season_marker(&lower))
                || lower == number)
        });
    }
    if edition != EditionKind::Unknown {
        tokens.retain(|token| detect_edition_kind(token) == EditionKind::Unknown);
    }
    let mut result = clean_join(tokens);
    if let Some(season) = season {
        result = strip_trailing_season_marker(&result, season);
    }
    if result.is_empty() {
        value.to_string()
    } else {
        result
    }
}

fn is_standalone_season_marker(value: &str) -> bool {
    let lower = value.nfkc().collect::<String>().to_lowercase();
    if lower == "season" || lower == "シーズン" || lower == "시즌" {
        return true;
    }
    if lower.strip_prefix('s').and_then(parse_small_u16).is_some()
        || lower
            .strip_prefix("season")
            .and_then(parse_ordinal)
            .is_some()
        || lower
            .strip_prefix("シーズン")
            .and_then(parse_small_u16)
            .is_some()
        || lower
            .strip_prefix("시즌")
            .and_then(parse_small_u16)
            .is_some()
    {
        return true;
    }
    let chars = lower.chars().collect::<Vec<_>>();
    chars
        .last()
        .is_some_and(|marker| matches!(marker, '季' | '期' | '기'))
        && chars[..chars.len().saturating_sub(1)]
            .iter()
            .copied()
            .filter(|character| *character != '第')
            .all(is_cjk_number_character)
}

fn strip_trailing_season_marker(value: &str, season: u16) -> String {
    let cjk_number = match season {
        1 => Some("一"),
        2 => Some("二"),
        3 => Some("三"),
        4 => Some("四"),
        5 => Some("五"),
        6 => Some("六"),
        7 => Some("七"),
        8 => Some("八"),
        9 => Some("九"),
        10 => Some("十"),
        _ => None,
    };
    let mut suffixes = vec![
        format!("season {season}"),
        format!("season{season}"),
        format!("s{season:02}"),
        format!("s{season}"),
        format!("第{season}季"),
        format!("第{season}期"),
        format!("{season}季"),
        format!("{season}期"),
        format!("シーズン{season}"),
        format!("시즌 {season}"),
        format!("시즌{season}"),
        format!("{season}기"),
    ];
    if let Some(cjk_number) = cjk_number {
        suffixes.extend([
            format!("第{cjk_number}季"),
            format!("第{cjk_number}期"),
            format!("{cjk_number}季"),
            format!("{cjk_number}期"),
        ]);
    }
    suffixes.sort_by_key(|suffix| std::cmp::Reverse(suffix.chars().count()));
    let lower = value.to_lowercase();
    for suffix in suffixes {
        if lower.ends_with(&suffix) {
            let prefix_len = value.len().saturating_sub(suffix.len());
            if value.is_char_boundary(prefix_len) {
                let prefix = value[..prefix_len].trim_end_matches(|character: char| {
                    character.is_whitespace() || matches!(character, '-' | '_' | ':' | '·' | '|')
                });
                if is_useful_candidate(prefix) {
                    return prefix.to_string();
                }
            }
        }
    }
    value.to_string()
}

fn detect_language_hints(value: &str) -> Vec<LanguageHint> {
    let mut hints = Vec::new();
    for character in value.chars() {
        let code = character as u32;
        if (0x4e00..=0x9fff).contains(&code) && !hints.contains(&LanguageHint::Chinese) {
            hints.push(LanguageHint::Chinese);
        }
        if ((0x3040..=0x30ff).contains(&code) || (0x31f0..=0x31ff).contains(&code))
            && !hints.contains(&LanguageHint::Japanese)
        {
            hints.push(LanguageHint::Japanese);
        }
        if (0xac00..=0xd7af).contains(&code) && !hints.contains(&LanguageHint::Korean) {
            hints.push(LanguageHint::Korean);
        }
        if character.is_ascii_alphabetic() && !hints.contains(&LanguageHint::Latin) {
            hints.push(LanguageHint::Latin);
        }
    }
    hints
}

fn evidence_quality(primary: &str, file: Option<&str>, parent: Option<&str>) -> u8 {
    let normalized = normalize_title_for_match(primary);
    if !is_useful_candidate(primary) || is_generic_title(primary) {
        return 25;
    }
    let mut quality: u8 = if normalized.chars().count() >= 6 {
        70
    } else {
        55
    };
    if file.is_some_and(|value| normalize_title_for_match(value) == normalized) {
        quality = quality.saturating_add(20);
    }
    if parent.is_some_and(|value| normalize_title_for_match(value) == normalized) {
        quality = quality.saturating_add(10);
    }
    quality.min(100)
}

pub fn is_generic_title(value: &str) -> bool {
    matches!(
        normalize_title_for_match(value).as_str(),
        "bd" | "bdmv"
            | "sp"
            | "ova"
            | "oad"
            | "extra"
            | "extras"
            | "special"
            | "specials"
            | "season"
            | "season2"
            | "合集"
            | "合辑"
            | "動畫"
            | "动画"
            | "anime"
            | "animation"
            | "media"
            | "video"
            | "videos"
    )
}

pub fn is_safe_match_query(value: &str) -> bool {
    let trimmed = value.trim();
    let normalized = normalize_title_for_match(trimmed);
    let numeric_slash_title = trimmed.contains('/')
        && trimmed.chars().all(|character| {
            character.is_ascii_digit() || character == '/' || character.is_whitespace()
        });
    normalized.chars().count() >= 3
        && (!normalized.chars().all(|character| character.is_numeric()) || numeric_slash_title)
        && !is_generic_title(trimmed)
}

fn is_useful_candidate(value: &str) -> bool {
    let compact = value.trim();
    compact.chars().count() >= 2 && !is_generic_title(compact)
}

fn push_unique(values: &mut Vec<String>, candidate: String) {
    let normalized = normalize_title_for_match(&candidate);
    if !values
        .iter()
        .any(|value| normalize_title_for_match(value) == normalized)
    {
        values.push(candidate);
    }
}

fn deduplicate_strings(values: &mut Vec<String>) {
    let mut unique = Vec::new();
    for value in std::mem::take(values) {
        if !value.trim().is_empty() {
            push_unique(&mut unique, value);
        }
    }
    *values = unique;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_complex_latin_release_name() {
        assert_eq!(
            extract_search_keyword(
                "[Airota&VCB-Studio] Sousou no Frieren [Ma10p_1080p][x265_flac]"
            ),
            "Sousou no Frieren"
        );
    }

    #[test]
    fn preserves_japanese_title_and_removes_episode_parameters() {
        assert_eq!(
            extract_search_keyword("[VCB-Studio] ぼっち・ざ・ろっく！ [01][Ma10p_1080p]"),
            "ぼっち・ざ・ろっく！"
        );
    }

    #[test]
    fn preserves_title_inside_brackets_after_release_group() {
        assert_eq!(
            extract_search_keyword("[DBD-Raws][STEINS;GATE][1080P][BDRip][HEVC-10bit]"),
            "STEINS;GATE"
        );
    }

    #[test]
    fn removes_multiple_release_groups_and_season_markers() {
        assert_eq!(
            extract_search_keyword("[DBD-Raws][VCB-Studio] Title [S01][1080p]"),
            "Title"
        );
    }

    #[test]
    fn strips_real_world_leading_release_groups_without_dropping_bracketed_titles() {
        let cases = [
            (
                "[BeanSub&FZSD][Saiki_Kusuo_no_Psi-nan][S02][1080p]",
                "Saiki Kusuo no Psi-nan",
            ),
            ("[CASO][Rozen_Maiden][01-12][1080P]", "Rozen Maiden"),
            (
                "[NanoAlchemist] Rozen Maiden S2 [01][1080p]",
                "Rozen Maiden",
            ),
            (
                "[Nekomoe kissaten] Tonari no Kyuuketsuki-san [1080p]",
                "Tonari no Kyuuketsuki-san",
            ),
            ("[UHA-WINGS][Fate_stay_night][1080p]", "Fate stay night"),
        ];
        for (raw, expected) in cases {
            assert_eq!(extract_search_keyword(raw), expected, "raw={raw}");
        }
        assert_eq!(extract_search_keyword("[NANA][1080p]"), "NANA");
        assert_eq!(
            extract_search_keyword("[STEINS;GATE][1080p]"),
            "STEINS;GATE"
        );
    }

    #[test]
    fn media_file_frequency_provides_candidate_without_mutating_original() {
        let original = "BD";
        let result = build_search_prefill(
            original,
            original,
            &[
                "[VCB-Studio] Frieren [01][1080p].mkv".into(),
                "[VCB-Studio] Frieren [02][1080p].mkv".into(),
                "[VCB-Studio] Frieren [03][1080p].mkv".into(),
            ],
        );
        assert_eq!(result.original_name, "BD");
        assert!(result.candidates.iter().any(|value| value == "Frieren"));
    }

    #[test]
    fn structured_evidence_extracts_season_year_movie_and_noise() {
        let evidence = build_match_evidence(
            "[VCB-Studio] Made in Abyss Season 2 Movie (2022) [1080p][HEVC]",
            "[VCB-Studio] Made in Abyss Season 2 Movie (2022) [1080p][HEVC]",
            Some("Made in Abyss"),
            &[],
        );
        assert_eq!(evidence.season_number, Some(2));
        assert_eq!(evidence.year, Some(2022));
        assert_eq!(evidence.edition_kind, EditionKind::Movie);
        assert!(evidence.primary_title.contains("Made in Abyss"));
        assert!(evidence.removed_noise.iter().any(|value| value == "1080p"));
        assert_eq!(evidence.parent_title.as_deref(), Some("Made in Abyss"));
    }

    #[test]
    fn structured_evidence_extracts_a_common_live_action_movie_release_name() {
        let evidence = build_match_evidence(
            "[YTS] Oppenheimer (2023) [1080p][BluRay][x264]",
            "[YTS] Oppenheimer (2023) [1080p][BluRay][x264]",
            None,
            &["Oppenheimer.2023.1080p.BluRay.x264.mkv".into()],
        );
        assert_eq!(evidence.primary_title, "Oppenheimer");
        assert_eq!(evidence.year, Some(2023));
        assert!(is_safe_match_query(&evidence.primary_title));
    }

    #[test]
    fn dotted_scene_movie_names_become_clean_structured_queries() {
        let cases = [
            (
                "The.Sword.of.Doom.1966.1080p.BluRay.x264",
                "The Sword of Doom",
                1966,
            ),
            ("WolfWalkers.2020.1080p.BluRay.x265", "WolfWalkers", 2020),
            (
                "The.Empire.of.Corpses.2015.1080p.BDRip.HEVC",
                "The Empire of Corpses",
                2015,
            ),
        ];
        for (raw, expected_title, expected_year) in cases {
            let evidence = build_match_evidence(raw, raw, None, &[]);
            assert_eq!(evidence.primary_title, expected_title, "raw={raw}");
            assert_eq!(evidence.year, Some(expected_year), "raw={raw}");
            assert_eq!(extract_search_keyword(raw), expected_title, "raw={raw}");
            assert!(is_safe_match_query(&evidence.primary_title), "raw={raw}");
        }
    }

    #[test]
    fn meaningful_dotted_title_without_release_metadata_is_preserved() {
        assert_eq!(extract_search_keyword("K.O.2"), "K.O.2");
    }

    #[test]
    fn structured_evidence_understands_cjk_seasons() {
        let simplified = extract_title_signals("葬送的芙莉莲 第二季 [2024]");
        let japanese = extract_title_signals("作品名 第3期");
        assert_eq!(simplified.season_number, Some(2));
        assert_eq!(simplified.year, Some(2024));
        assert_eq!(japanese.season_number, Some(3));
    }

    #[test]
    fn season_signals_cover_compact_and_cjk_second_season_forms() {
        let cases = [
            ("Example S2", "Example"),
            ("Example S02", "Example"),
            ("Example Season2", "Example"),
            ("Example Season 2", "Example"),
            ("Example 2nd Season", "Example"),
            ("作品名第二季", "作品名"),
            ("作品名 第2季", "作品名"),
            ("作品名 2期", "作品名"),
            ("作品名 二期", "作品名"),
            ("作品名 シーズン2", "作品名"),
            ("작품명 시즌 2", "작품명"),
            ("작품명 2기", "작품명"),
        ];
        for (raw, expected_series) in cases {
            let signals = extract_title_signals(raw);
            assert_eq!(signals.season_number, Some(2), "raw={raw}");
            assert_eq!(signals.series_title, expected_series, "raw={raw}");
        }
        assert_eq!(extract_title_signals("春夏秋冬四季").season_number, None);
    }

    #[test]
    fn file_and_embedded_aliases_survive_as_independent_match_evidence() {
        let evidence = build_match_evidence(
            "[CASO][A Fan English Translation][Saiki Kusuo no Psi-nan][S02]",
            "[CASO][A Fan English Translation][Saiki Kusuo no Psi-nan][S02]",
            None,
            &[
                "[BeanSub] Saiki Kusuo no Psi-nan S02 [01][1080p].mkv".into(),
                "[BeanSub] Saiki Kusuo no Psi-nan S02 [02][1080p].mkv".into(),
            ],
        );
        assert_eq!(evidence.season_number, Some(2));
        assert_eq!(
            evidence.frequent_file_title.as_deref(),
            Some("Saiki Kusuo no Psi-nan S2")
        );
        assert!(evidence
            .alternate_titles
            .iter()
            .any(|title| title == "Saiki Kusuo no Psi-nan"));
        assert!(evidence
            .alternate_titles
            .iter()
            .any(|title| title == "A Fan English Translation"));
    }

    #[test]
    fn meaningful_title_numbers_are_preserved() {
        assert!(
            build_match_evidence("Steins;Gate 0", "Steins;Gate 0", None, &[])
                .primary_title
                .ends_with('0')
        );
        assert!(
            build_match_evidence("86 -Eighty Six-", "86 -Eighty Six-", None, &[])
                .primary_title
                .contains("86")
        );
        assert!(build_match_evidence("22/7", "22/7", None, &[])
            .primary_title
            .contains("22/7"));
        assert!(build_match_evidence("Fate/Zero", "Fate/Zero", None, &[])
            .primary_title
            .contains("Fate/Zero"));
        assert!(build_match_evidence("Re:Zero", "Re:Zero", None, &[])
            .primary_title
            .contains("Re:Zero"));
        assert!(is_safe_match_query("22/7"));
    }

    #[test]
    fn normalization_is_nfkc_case_and_separator_insensitive() {
        assert_eq!(
            normalize_title_for_match("ＳＴＥＩＮＳ；ＧＡＴＥ"),
            normalize_title_for_match("steins;gate")
        );
        assert_eq!(
            normalize_title_for_match("Tom & Jerry"),
            normalize_title_for_match("Tom and Jerry")
        );
    }

    #[test]
    fn high_frequency_file_title_becomes_structured_evidence() {
        let evidence = build_match_evidence(
            "BD",
            "BD",
            None,
            &[
                "[VCB-Studio] Frieren [01][1080p].mkv".into(),
                "[VCB-Studio] Frieren [02][1080p].mkv".into(),
                "[VCB-Studio] Frieren [03][1080p].mkv".into(),
            ],
        );
        assert_eq!(evidence.frequent_file_title.as_deref(), Some("Frieren"));
        assert_eq!(evidence.primary_title, "Frieren");
        assert!(evidence.evidence_quality >= 80);
    }

    #[test]
    fn generic_and_numeric_queries_are_rejected_but_specific_titles_are_safe() {
        assert!(!is_safe_match_query("Season 2"));
        assert!(!is_safe_match_query("123"));
        assert!(!is_safe_match_query("SP"));
        assert!(is_safe_match_query("86 Eighty-Six"));
    }
}
