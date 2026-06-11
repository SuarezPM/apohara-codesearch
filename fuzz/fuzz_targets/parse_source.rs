// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the tree-sitter structural parser on untrusted source bytes. A crafted
//! file must never panic the parser — it must recover (ERROR nodes) or return a
//! typed error. Backs the input_validation claim in docs/ASSURANCE.md § 4.
#![no_main]

use apohara_indexer::{parse_source, Language};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The parser consumes &str; non-UTF-8 is rejected upstream by the walker.
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    // Exercise every supported grammar so error-recovery is fuzzed for all five.
    for language in [
        Language::Rust,
        Language::TypeScript,
        Language::Python,
        Language::Go,
        Language::Bash,
        Language::Java,
    ] {
        let _ = parse_source(source, language);
    }
});
