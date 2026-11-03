//! Author/contributor name normalization for search.
//!
//! This module intentionally does not mutate or rewrite the original metadata.
//! Keep the provider's original author string for display/storage and use these
//! helpers only to derive searchable names.
//!
//! Supported separators: `/`, `,`, `;`, `|`, CR/LF.
//! Whitespace is never treated as a separator because personal names may
//! contain spaces.

/// Split a provider-supplied author/contributor field into searchable names.
///
/// Examples:
/// - `"허버트 조지 웰즈 저/이태영 그림/채희석 역"`
///   -> `["허버트 조지 웰즈", "이태영", "채희석"]`
/// - `"홍길동, 김철수 | 이영희"`
///   -> `["홍길동", "김철수", "이영희"]`
/// - `"홍길동 (글쓴이)"` -> `["홍길동"]`
///
/// The input is not modified. Role labels are removed only from the returned
/// search values; callers should retain the original string separately.
pub fn normalize_author_names(value: &str) -> Vec<String> {
    let mut names = Vec::new();

    for part in split_contributors(value) {
        let cleaned = strip_role_annotations(part);
        let normalized = normalize_whitespace(&cleaned);

        if normalized.is_empty() {
            continue;
        }

        let key = normalized.to_lowercase();
        if !names
            .iter()
            .any(|existing: &String| existing.to_lowercase() == key)
        {
            names.push(normalized);
        }
    }

    names
}

/// Normalize a free-text author query to a searchable key.
///
/// Only the first contributor (main author) is retained.
/// Role annotations are removed and whitespace/case are normalized.
pub fn normalize_author_query(value: &str) -> String {
    normalize_author_names(value)
        .into_iter()
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Return true when any normalized contributor name contains the normalized
/// query. This provides OR semantics for a field containing multiple names.
///
/// For example, a query for `"채희석"` matches
/// `"허버트 조지 웰즈 저/이태영 그림/채희석 역"`.
pub fn author_names_match(value: &str, query: &str) -> bool {
    let query = normalize_author_query(query);
    if query.is_empty() {
        return true;
    }

    normalize_author_names(value)
        .iter()
        .any(|name| name.to_lowercase().contains(&query))
}

fn split_contributors(value: &str) -> Vec<&str> {
    value.split(['/', ',', ';', '|', '\n', '\r']).collect()
}

fn strip_role_annotations(value: &str) -> String {
    let mut result = value.trim().to_string();

    // Remove parenthesized/bracketed role annotations, e.g. "(글쓴이)",
    // "[번역]", "（그림）". Iterate because a value may contain more than one.
    for (open, close) in [('(', ')'), ('[', ']'), ('（', '）'), ('［', '］')] {
        loop {
            let Some(start) = result.find(open) else {
                break;
            };
            let Some(relative_end) = result[start + open.len_utf8()..].find(close) else {
                break;
            };
            let end = start + open.len_utf8() + relative_end + close.len_utf8();
            result.replace_range(start..end, " ");
        }
    }

    // Remove role labels at the end only. Match complete labels so that
    // ordinary names containing similar syllables are not truncated.
    const SUFFIXES: &[&str] = &[
        "글쓴이",
        "지은이",
        "지음",
        "저자",
        "저",
        "작가",
        "옮긴이",
        "옮김",
        "번역자",
        "번역",
        "역자",
        "역",
        "그린이",
        "그림",
        "삽화",
        "일러스트",
        "엮은이",
        "엮음",
        "편저",
        "편역",
        "편",
        "감수",
        "해설",
        "사진",
        "촬영",
        "기획",
        "구성",
        "각색",
        "원작",
        "공저",
        "공역",
        "글",
        "그림작가",
        "편집",
    ];

    loop {
        let trimmed = result.trim_end();
        let matched = SUFFIXES.iter().find(|suffix| {
            trimmed.ends_with(**suffix)
                && trimmed[..trimmed.len() - suffix.len()]
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace)
        });

        match matched {
            Some(suffix) => {
                let new_len = trimmed.len() - suffix.len();
                result.truncate(new_len);
            }
            None => break,
        }
    }

    result
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
