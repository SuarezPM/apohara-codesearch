// SPDX-License-Identifier: MIT OR Apache-2.0

//! Code-aware tokenizer shared by the index-time and query-time paths.
//!
//! This function is load-bearing: the FTS index stores `code_tokens(body)` and
//! the search path tokenizes the query through the SAME function, so any change
//! here changes both sides at once and they must stay in lockstep.
//!
//! Pipeline (split first, lowercase last so case boundaries survive splitting):
//!   1. Split on every non-alphanumeric character (including `_`).
//!   2. Within each segment, split camelCase at a lower→upper boundary.
//!   3. Acronym-aware: in a run of consecutive uppercase letters followed by a
//!      lowercase letter, split before the LAST uppercase letter
//!      (`HTTPServer` -> `http`, `server`; `parseHTTPRequest` -> `parse`, `http`, `request`).
//!   4. Digits stay attached to adjacent alphabetic characters
//!      (`utf8Decode` -> `utf8`, `decode`; `sha256sum` stays one token).
//!   5. Empty tokens (from leading/trailing/doubled separators) are dropped.

/// Tokenize `s` into lowercased code tokens. See module docs for the exact spec.
pub fn code_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Split on separators first, keeping each alphanumeric segment intact.
    for segment in s.split(|c: char| !c.is_alphanumeric()) {
        if segment.is_empty() {
            continue;
        }
        split_segment(segment, &mut out);
    }
    out
}

/// Split a single alphanumeric segment on camelCase and acronym boundaries,
/// pushing each lowercased sub-token onto `out`.
fn split_segment(segment: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = segment.chars().collect();
    let n = chars.len();
    let mut start = 0usize;

    for i in 1..n {
        if is_boundary(&chars, i) {
            out.push(chars[start..i].iter().collect::<String>().to_lowercase());
            start = i;
        }
    }
    if start < n {
        out.push(chars[start..n].iter().collect::<String>().to_lowercase());
    }
}

/// Decide whether a token boundary falls BEFORE `chars[i]` (i >= 1).
///
/// A boundary marks the start of a new capitalized word. Digits never start a
/// new word and never end one with a hard cut: `alpha->digit`, `digit->lower`
/// and `digit->digit` all stay attached (`sha256sum` is one token). The only
/// digit transition that splits is `digit->upper`, which is just camelCase with
/// a digit on the left (`utf8Decode` -> `utf8`, `decode`).
fn is_boundary(chars: &[char], i: usize) -> bool {
    let prev = chars[i - 1];
    let cur = chars[i];

    // A new word can only begin at an uppercase letter.
    if !cur.is_uppercase() {
        return false;
    }

    // camelCase (and digitCase): lower/digit -> upper starts a new word.
    if prev.is_lowercase() || prev.is_ascii_digit() {
        return true;
    }

    // Acronym-aware: in a run of consecutive uppercase letters followed by a
    // lowercase letter, split before the LAST cap. Here prev=upper, cur=upper,
    // and the char AFTER cur is lowercase, so `cur` is that last cap and begins
    // the next word (e.g. `HTTPServer` splits before `S`).
    if prev.is_uppercase() && chars.get(i + 1).map(|c| c.is_lowercase()).unwrap_or(false) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        code_tokens(s)
    }

    #[test]
    fn pinned_cases() {
        assert_eq!(t("parseString"), vec!["parse", "string"]);
        assert_eq!(t("HTTPServer"), vec!["http", "server"]);
        assert_eq!(t("parseHTTPRequest"), vec!["parse", "http", "request"]);
        assert_eq!(t("utf8Decode"), vec!["utf8", "decode"]);
        assert_eq!(t("MAX_SIZE"), vec!["max", "size"]);
        assert_eq!(t("sha256sum"), vec!["sha256sum"]);
        assert_eq!(t("__init__"), vec!["init"]);
        assert_eq!(t("_foo"), vec!["foo"]);
    }
}
