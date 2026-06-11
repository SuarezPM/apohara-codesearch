// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the C++ tree-sitter extractor on untrusted source bytes.
//!
//! F1-KOTLIN-CPP story's C++ side. Mirrors parse_bash_source / parse_c_source.
//! Critical regression target: the `field_identifier` vs `identifier` node kind
//! distinction (member function names are `field_identifier`; the parser
//! extractor must accept both).
#![no_main]

use apohara_indexer::{
    parse_source, parse_source_imports_exports, parse_source_spans, Language,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_source(source, Language::Cpp);
    let _ = parse_source_spans(source, Language::Cpp);
    let _ = parse_source_imports_exports(source, Language::Cpp);
});
