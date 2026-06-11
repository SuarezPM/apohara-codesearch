// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the Ruby tree-sitter extractor on untrusted source bytes.
//!
//! F1-RUBY story. Mirrors parse_bash_source / parse_java_source / parse_c_source.
//! Critical regression target: the R-1.3 anti-gotcha (do_block / block must
//! NOT be emitted as separate top-level methods; only `def` is).
#![no_main]

use apohara_indexer::{
    parse_source, parse_source_imports_exports, parse_source_spans, Language,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_source(source, Language::Ruby);
    let _ = parse_source_spans(source, Language::Ruby);
    let _ = parse_source_imports_exports(source, Language::Ruby);
});
