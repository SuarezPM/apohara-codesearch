// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Small text-processing utilities: tokenizing, edit distance, and a naive
// substring scan. Distinct vocabulary so queries about "fuzzy matching" or
// "Levenshtein" can be labeled to the right symbol.

/// Split a string into lowercased whitespace-delimited words, dropping empties.
pub fn tokenize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Levenshtein edit distance between two strings: the minimum number of single
/// character insertions, deletions, or substitutions to turn one into the
/// other. Classic dynamic-programming implementation over a rolling row.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Return the byte offset of the first occurrence of `needle` in `haystack`, or
/// `None` when absent. A naive O(n*m) scan; fine for short inputs.
pub fn find_substring(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.chars().collect();
    let need: Vec<char> = needle.chars().collect();
    if need.len() > hay.len() {
        return None;
    }
    for start in 0..=(hay.len() - need.len()) {
        if hay[start..start + need.len()] == need[..] {
            return Some(start);
        }
    }
    None
}

/// Count how many words in `text` appear in the `stopwords` set, a cheap proxy
/// for how "noisy" a passage is before indexing it.
pub fn count_stopwords(text: &str, stopwords: &[&str]) -> usize {
    tokenize_words(text)
        .iter()
        .filter(|w| stopwords.contains(&w.as_str()))
        .count()
}
