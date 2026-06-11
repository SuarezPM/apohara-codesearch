// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the Java tree-sitter extractor on untrusted source bytes.
//!
//! F1-JAVA story. Mirrors parse_bash_source: exercises the four public
//! entry points on adversarial input so error-recovery is fuzzed for
//! class/interface/enum/record/method extraction independently of the
//! bundled parse_source target.
#![no_main]

use apohara_indexer::{
    parse_source, parse_source_imports_exports, parse_source_spans, Language,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_source(source, Language::Java);
    let _ = parse_source_spans(source, Language::Java);
    let _ = parse_source_imports_exports(source, Language::Java);
});
