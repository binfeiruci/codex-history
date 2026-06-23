use std::path::Path;

use crate::history::Conversation;

pub fn filter_and_rank<'a>(
    conversations: &'a [Conversation],
    query: &str,
    local_filter: bool,
    current_dir: &Path,
) -> Vec<&'a Conversation> {
    filter_and_rank_indices(conversations, query, local_filter, current_dir)
        .into_iter()
        .map(|idx| &conversations[idx])
        .collect()
}

pub fn filter_and_rank_indices(
    conversations: &[Conversation],
    query: &str,
    local_filter: bool,
    current_dir: &Path,
) -> Vec<usize> {
    filter_and_rank_candidate_indices(
        conversations,
        0..conversations.len(),
        query,
        local_filter,
        current_dir,
    )
}

pub fn filter_and_rank_candidate_indices(
    conversations: &[Conversation],
    candidates: impl IntoIterator<Item = usize>,
    query: &str,
    local_filter: bool,
    current_dir: &Path,
) -> Vec<usize> {
    let query = query.trim();
    let words = query_words(query);
    if words.is_empty() {
        return candidates
            .into_iter()
            .filter_map(|idx| {
                let conversation = &conversations[idx];
                (!local_filter || same_workspace(conversation, current_dir)).then_some(idx)
            })
            .collect();
    }

    let mut scored = candidates
        .into_iter()
        .filter_map(|idx| {
            let conversation = &conversations[idx];
            if local_filter && !same_workspace(conversation, current_dir) {
                return None;
            }
            score(conversation, &words).map(|score| (score, idx))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(score_a, idx_a), (score_b, idx_b)| {
        let a = &conversations[*idx_a];
        let b = &conversations[*idx_b];
        score_b
            .partial_cmp(score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    scored.into_iter().map(|(_, idx)| idx).collect()
}

pub fn same_workspace(conversation: &Conversation, current_dir: &Path) -> bool {
    let Some(cwd) = &conversation.cwd else {
        return false;
    };
    cwd == current_dir || cwd.starts_with(current_dir) || current_dir.starts_with(cwd)
}

fn score(conversation: &Conversation, words: &[String]) -> Option<f64> {
    if !words
        .iter()
        .all(|word| conversation_matches(conversation, word))
    {
        return None;
    }

    let mut total = 0.0;
    for word in words {
        if conversation.session_id_normalized.contains(word) {
            total += 80.0;
        }
        if conversation.title_normalized.contains(word) {
            total += 45.0;
        }
        if conversation.cwd_normalized.contains(word) {
            total += 25.0;
        }
        total += 5.0;
    }
    Some(total)
}

fn query_words(query: &str) -> Vec<String> {
    normalize(query)
        .split_whitespace()
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn normalize(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect()
}

fn conversation_matches(conversation: &Conversation, needle: &str) -> bool {
    word_matches(&conversation.search_text_normalized, needle)
}

fn word_matches(haystack: &str, needle: &str) -> bool {
    if needle
        .chars()
        .any(|ch| !ch.is_ascii() || ch == '_' || ch == '-')
        && haystack.contains(needle)
    {
        return true;
    }
    haystack.split_whitespace().any(|word| {
        word.starts_with(needle) || word.split('_').any(|part| part.starts_with(needle))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_matches_word_boundaries() {
        assert!(word_matches("authentication config", "auth"));
        assert!(!word_matches("fired", "red"));
    }

    #[test]
    fn prefix_matches_underscore_parts() {
        assert!(word_matches("auth_config", "auth"));
        assert!(word_matches("auth_config", "config"));
    }
}
