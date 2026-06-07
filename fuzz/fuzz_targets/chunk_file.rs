// SPDX-License-Identifier: MIT OR Apache-2.0
//
//! Fuzz the chunker on untrusted file content. Per-symbol + module/window
//! chunking over a crafted file must never panic, hang on a pathological input,
//! or produce malformed spans. Backs docs/ASSURANCE.md § 4 (input validation)
//! and the bounded-chunk hardening in chunker.rs.
#![no_main]

use apohara_indexer::{chunk_file, Language};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(content) = std::str::from_utf8(data) else {
        return;
    };
    // Parsed-language paths (tree-sitter) and the language-agnostic text-window
    // fallback (None) all chunk the same untrusted content.
    let _ = chunk_file("fuzz.rs", content, Some(Language::Rust));
    let _ = chunk_file("fuzz.py", content, Some(Language::Python));
    let _ = chunk_file("fuzz.ts", content, Some(Language::TypeScript));
    let _ = chunk_file("fuzz.go", content, Some(Language::Go));
    let _ = chunk_file("fuzz.unknown", content, None);
});
