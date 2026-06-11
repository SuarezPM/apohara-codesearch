// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the Bash tree-sitter extractor on untrusted source bytes.
//!
//! This is a dedicated target for the Bash grammar (F1-BASH story) so
//! ClusterFuzzLite can exercise the Bash-specific extractors
//! (`extract_bash_functions`, `extract_bash_function_spans`,
//! `extract_bash_imports`, `extract_bash_exports`) on adversarial input
//! independently from the main `parse_source` target (which also includes Bash
//! but is bundled with the other grammars).
//!
//! Run locally: `cargo +nightly fuzz run parse_bash_source`
//! CI runs it via ClusterFuzzLite on PRs (not on each push for cost reasons).
#![no_main]

use apohara_indexer::{
    parse_source, parse_source_imports_exports, parse_source_spans, Language,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    // Exercise the four Bash-specific public entry points. None must panic.
    let _ = parse_source(source, Language::Bash);
    let _ = parse_source_spans(source, Language::Bash);
    let _ = parse_source_imports_exports(source, Language::Bash);
});
