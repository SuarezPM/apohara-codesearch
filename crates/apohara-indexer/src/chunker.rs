// SPDX-License-Identifier: MIT OR Apache-2.0

//! Splits file contents into indexable chunks.
//!
//! Two strategies, picked by whether the file's language is known:
//!
//! - **Parsed** (`Some(Language)`): one [`ChunkKind::Symbol`] chunk per
//!   function/method span (from [`parse_source_spans`]), plus one or more
//!   [`ChunkKind::Module`] chunks holding the lines NOT covered by a symbol
//!   span. The Module chunks carry top-level code, `use`/`import` lines,
//!   struct/enum definitions, and the `impl`/`class` wrapper lines whose inner
//!   methods became Symbol chunks. The uncovered lines are split into bounded
//!   sub-chunks (see [`MAX_CHUNK_LINES`]/[`MAX_CHUNK_BYTES`]) so a large
//!   constants/types file does not produce one giant module chunk.
//! - **Unparsed** (`None`): overlapping fixed-size line [`ChunkKind::Window`]s.
//!
//! ## Overlapping-span rule
//!
//! [`parse_source_spans`] emits one span per *method*, not per enclosing
//! `impl`/`class` block. A line is considered "inside a symbol" iff it lies
//! within at least one emitted span's `[start, end]` (inclusive). The Module
//! chunk is the complement of that union over the file's lines. Consequence:
//! `impl Foo {` and its matching `}` (and any blank lines between methods) are
//! NOT inside a method span, so they land in the Module chunk. Method bodies do
//! not bleed into the Module chunk. Nested items (a fn defined inside another
//! fn) would be covered by the outer span and so do not produce a separate hole.

use crate::parser::{parse_source_spans, FunctionSignature, Language, SymbolKind};

/// Number of source lines per [`ChunkKind::Window`] for unparsed files.
pub const WINDOW_LINES: usize = 60;
/// Overlap (in lines) between consecutive windows. Stride = `WINDOW_LINES - WINDOW_OVERLAP`.
pub const WINDOW_OVERLAP: usize = 15;

/// Maximum number of source lines in a Module (or Window) chunk before it is
/// flushed and a new sub-chunk begins.
///
/// UNTUNED INITIAL DEFAULT — not a measured optimum. Revisited once a
/// recall/MRR benchmark over varied chunk sizes exists to provide the
/// tuning signal. Until then this is a conservative round number chosen so a
/// chunk stays roughly one screenful and its bag-of-tokens embedding is not
/// diluted by thousands of lines.
pub const MAX_CHUNK_LINES: usize = 200;
/// Maximum byte length of a Module (or Window) chunk body before it is flushed.
///
/// UNTUNED INITIAL DEFAULT — not a measured optimum. Revisited once a
/// recall/MRR benchmark exists. 8 KiB keeps a chunk's lexical content bounded so
/// its bag-of-tokens embedding is not diluted by a very long line block.
pub const MAX_CHUNK_BYTES: usize = 8 * 1024;

/// What a [`ChunkSpec`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    /// A single parsed function or method.
    Symbol,
    /// The non-symbol remainder of a parsed file.
    Module,
    /// A fixed-size overlapping window of an unparsed file.
    Window,
}

/// One chunk carved out of a file: a line range plus its body text and,
/// for symbol chunks, the originating [`FunctionSignature`] + its
/// [`SymbolKind`].
#[derive(Debug, Clone)]
pub struct ChunkSpec {
    pub kind: ChunkKind,
    pub start_line: usize,
    pub end_line: usize,
    pub body: String,
    pub symbol: Option<FunctionSignature>,
    /// The parsed symbol category for `Symbol` chunks; `None` for Module/Window.
    pub symbol_kind: Option<SymbolKind>,
}

impl ChunkSpec {
    /// Stable string tag for this chunk's kind.
    ///
    /// A `Function` symbol resolves to `"function"` or `"method"` (a signature
    /// whose first parameter is the receiver `self` is a method). A TYPE symbol
    /// (`struct`/`enum`/`trait`/`class`/`interface`/`type`) renders its own
    /// keyword from the threaded [`SymbolKind`]. Module and Window kinds map to
    /// `"module"` and `"window"`.
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            ChunkKind::Symbol => match self.symbol_kind {
                // A function may be a method (self-receiver heuristic).
                Some(SymbolKind::Function) | None => match &self.symbol {
                    Some(sig) if is_method(sig) => "method",
                    _ => "function",
                },
                Some(kind) => kind.keyword(),
            },
            ChunkKind::Module => "module",
            ChunkKind::Window => "window",
        }
    }
}

/// A signature is a method when its receiver parameter is `self` (Rust:
/// `self`/`&self`/`&mut self`; TS methods are extracted from class/interface
/// bodies and carry no `self`, so they read as "function" here, matching the
/// data we actually have).
fn is_method(sig: &FunctionSignature) -> bool {
    sig.parameters
        .first()
        .map(|p| p.name == "self")
        .unwrap_or(false)
}

/// Build the canonical chunk id: `"{rel_path}:{start_line}-{end_line}"`.
pub fn chunk_id(rel_path: &str, start_line: usize, end_line: usize) -> String {
    format!("{rel_path}:{start_line}-{end_line}")
}

/// Chunk a single file. `rel_path` is informational (the caller pairs it with
/// [`chunk_id`]); chunking decisions depend only on `content` and `language`.
pub fn chunk_file(rel_path: &str, content: &str, language: Option<Language>) -> Vec<ChunkSpec> {
    // `rel_path` is part of the public contract (callers pair it with
    // `chunk_id`) but chunking itself depends only on content + language.
    let _ = rel_path;
    match language {
        Some(lang) => chunk_parsed(content, lang),
        None => chunk_windows(content),
    }
}

/// Split `content` into lines, preserving them as owned strings without the
/// trailing newline. Line numbers are 1-based against this vector.
fn split_lines(content: &str) -> Vec<&str> {
    // `lines()` drops the final newline and does not yield a trailing empty
    // line, which matches tree-sitter's row counting for typical files.
    content.lines().collect()
}

/// Join a 1-based inclusive line range `[start, end]` from `lines` into a body.
fn join_range(lines: &[&str], start: usize, end: usize) -> String {
    let lo = start.saturating_sub(1);
    let hi = end.min(lines.len());
    if lo >= hi {
        return String::new();
    }
    lines[lo..hi].join("\n")
}

/// Parsed-file strategy: Symbol chunks per signature + one Module remainder.
fn chunk_parsed(content: &str, language: Language) -> Vec<ChunkSpec> {
    let lines = split_lines(content);
    let total = lines.len();

    let spans = parse_source_spans(content, language).unwrap_or_default();

    // Mark which 1-based lines are covered by at least one symbol span.
    let mut covered = vec![false; total + 1]; // index 0 unused
    let mut chunks: Vec<ChunkSpec> = Vec::new();
    // Guard against two emitted symbols sharing an identical clamped (start,end)
    // pair, which would collide on the `path:start-end` chunk id and drop the
    // whole file (see `chunk_ids_pairwise_distinct`). This happens for a TYPE
    // wrapper and an inner method declared on the SAME single line — e.g. a
    // one-line `trait Baz { fn x(&self); }` (both clamp to `L-L`). Type symbols
    // are emitted BEFORE their inner methods, so the enclosing type wins and the
    // same-line method is skipped (its lines are already covered by the type).
    let mut seen_ranges: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();

    for (sig, symbol_kind, start, end) in spans {
        // Clamp to the file's line count defensively.
        let start = start.max(1);
        let end = end.min(total).max(start);
        if !seen_ranges.insert((start, end)) {
            // A symbol with this exact range was already emitted; skip the
            // duplicate to preserve chunk-id distinctness. Its lines stay covered
            // (the first symbol already marked them), so the Module remainder is
            // unaffected.
            continue;
        }
        // `end` is already clamped to <= total above, so this slice is in-bounds.
        for covered_line in &mut covered[start..=end] {
            *covered_line = true;
        }
        chunks.push(ChunkSpec {
            kind: ChunkKind::Symbol,
            start_line: start,
            end_line: end,
            body: join_range(&lines, start, end),
            symbol: Some(sig),
            symbol_kind: Some(symbol_kind),
        });
    }

    // Module chunks: the lines not covered by a symbol span. If there are no
    // symbols at all, the whole file is uncovered. The uncovered lines form one
    // or more CONTIGUOUS runs (symbol spans punch holes between them); each run
    // is split into bounded Module sub-chunks. Because a run spans only
    // uncovered lines and Symbol chunks span only covered lines, every Module
    // sub-chunk range is disjoint from every Symbol range BY CONSTRUCTION — so
    // their `path:start-end` ids can never collide (pinned by the
    // `chunk_ids_pairwise_distinct` property test).
    let mut run_start = 0usize;
    // `covered[0]` is the unused sentinel; iterate the 1-based lines 1..=total.
    for (line, &is_covered) in covered.iter().enumerate().take(total + 1).skip(1) {
        let uncovered = !is_covered;
        if uncovered && run_start == 0 {
            run_start = line; // open a new contiguous run
        }
        // Close the run at a covered line, or at the file's last line.
        let run_ends_here = run_start != 0 && (!uncovered || line == total);
        if run_ends_here {
            let run_end = if uncovered { line } else { line - 1 };
            split_module_run(&lines, run_start, run_end, &mut chunks);
            run_start = 0;
        }
    }

    chunks
}

/// Split one contiguous run of uncovered lines `[run_start, run_end]` (1-based,
/// inclusive) into bounded [`ChunkKind::Module`] sub-chunks, appending them to
/// `chunks`.
///
/// A sub-chunk is flushed when adding the next line would exceed
/// [`MAX_CHUNK_LINES`] or [`MAX_CHUNK_BYTES`]. The flush is **boundary-
/// preferring**: it cuts at the NEAREST PRECEDING BLANK LINE within the current
/// sub-chunk window, falling back to a hard cut at the cap when the window holds
/// no blank line. Cuts are on LINE BOUNDARIES ONLY — never mid-line. (Rationale:
/// BM25 scores per-document, so splitting a struct/const block mid-construct
/// fragments lexical recall; blank lines usually separate top-level items.) Each
/// emitted sub-chunk carries its TRUE `[start_line, end_line]`.
fn split_module_run(lines: &[&str], run_start: usize, run_end: usize, chunks: &mut Vec<ChunkSpec>) {
    let mut seg_start = run_start; // 1-based start of the in-progress sub-chunk
    let mut line_count = 0usize;
    let mut byte_count = 0usize;
    // 1-based line number of the last blank line seen within the current
    // segment, or 0 if none. Used as the preferred flush boundary.
    let mut last_blank = 0usize;

    for line in run_start..=run_end {
        let text = lines[line - 1];
        // `+ 1` accounts for the `\n` join separator between lines.
        let added = text.len() + 1;

        // If adding this line would overflow either cap AND the segment already
        // holds at least one line, flush the accumulated segment first.
        let would_overflow = line_count > 0
            && (line_count + 1 > MAX_CHUNK_LINES || byte_count + added > MAX_CHUNK_BYTES);
        if would_overflow {
            // Prefer cutting at the last blank line inside the segment; the
            // blank line itself ends the flushed sub-chunk so the next segment
            // starts on the line after it. Fall back to a hard cap cut
            // (everything accumulated so far) when no blank line is available.
            let seg_end = if last_blank >= seg_start {
                last_blank
            } else {
                line - 1
            };
            push_module_chunk(lines, seg_start, seg_end, chunks);

            // Reset the accumulator to start at the line after the cut and
            // re-account every line from there up to (but excluding) `line`.
            seg_start = seg_end + 1;
            line_count = 0;
            byte_count = 0;
            last_blank = 0;
            for back in seg_start..line {
                let bt = lines[back - 1];
                line_count += 1;
                byte_count += bt.len() + 1;
                if bt.trim().is_empty() {
                    last_blank = back;
                }
            }
        }

        line_count += 1;
        byte_count += added;
        if text.trim().is_empty() {
            last_blank = line;
        }
    }

    // Flush the trailing segment.
    if line_count > 0 {
        push_module_chunk(lines, seg_start, run_end, chunks);
    }
}

/// Append a single [`ChunkKind::Module`] sub-chunk for the inclusive line range
/// `[start, end]`, with its body joined from those lines.
fn push_module_chunk(lines: &[&str], start: usize, end: usize, chunks: &mut Vec<ChunkSpec>) {
    chunks.push(ChunkSpec {
        kind: ChunkKind::Module,
        start_line: start,
        end_line: end,
        body: join_range(lines, start, end),
        symbol: None,
        symbol_kind: None,
    });
}

/// Unparsed-file strategy: overlapping fixed-size line windows.
fn chunk_windows(content: &str) -> Vec<ChunkSpec> {
    let lines = split_lines(content);
    let total = lines.len();
    let mut chunks = Vec::new();

    if total == 0 {
        return chunks;
    }

    let stride = WINDOW_LINES - WINDOW_OVERLAP;
    let mut start = 1usize; // 1-based
    loop {
        let end = (start + WINDOW_LINES - 1).min(total);
        chunks.push(ChunkSpec {
            kind: ChunkKind::Window,
            start_line: start,
            end_line: end,
            body: join_range(&lines, start, end),
            symbol: None,
            symbol_kind: None,
        });
        // The last window covers the tail.
        if end >= total {
            break;
        }
        start += stride;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_id_format() {
        assert_eq!(chunk_id("src/lib.rs", 10, 42), "src/lib.rs:10-42");
        assert_eq!(chunk_id("a/b.ts", 1, 1), "a/b.ts:1-1");
    }

    #[test]
    fn test_chunk_file_rust_two_fns_plus_module() {
        let content = r#"use std::fmt;

const VERSION: u32 = 1;

fn first() -> u32 {
    1
}

fn second() -> u32 {
    2
}

static TAIL: &str = "end";
"#;

        let chunks = chunk_file("src/x.rs", content, Some(Language::Rust));

        let symbols: Vec<&ChunkSpec> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Symbol)
            .collect();
        let modules: Vec<&ChunkSpec> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Module)
            .collect();

        assert_eq!(symbols.len(), 2, "expected 2 symbol chunks");
        // The uncovered remainder is now split into one Module chunk PER
        // contiguous uncovered run. The two functions punch holes at 5-7 and
        // 9-11, leaving three runs: [1,4], [8,8], [12,12] => 3 module chunks.
        assert_eq!(
            modules.len(),
            3,
            "expected one module chunk per uncovered run"
        );

        // Symbol spans: first() on lines 5-7, second() on lines 9-11.
        let first = symbols
            .iter()
            .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some("first"))
            .unwrap();
        let second = symbols
            .iter()
            .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some("second"))
            .unwrap();
        assert!(first.start_line < first.end_line);
        assert!(second.start_line < second.end_line);

        // No module chunk may contain a symbol line range.
        let module_body: String = modules
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!module_body.contains("fn first()"));
        assert!(!module_body.contains("fn second()"));
        // But the top-level lines must appear across the module chunks.
        assert!(module_body.contains("use std::fmt;"));
        assert!(module_body.contains("const VERSION"));
        assert!(module_body.contains("static TAIL"));

        // kind_str for free functions and modules.
        assert_eq!(first.kind_str(), "function");
        assert_eq!(modules[0].kind_str(), "module");
    }

    #[test]
    fn test_chunk_file_rust_methods_are_method_kind() {
        let content = r#"struct S;

impl S {
    fn make(&self) -> u32 {
        7
    }
}
"#;
        let chunks = chunk_file("src/s.rs", content, Some(Language::Rust));
        // `struct S;` is now its OWN symbol chunk, so find the method by name
        // rather than assuming it is the first Symbol chunk.
        let method = chunks
            .iter()
            .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some("make"))
            .unwrap();
        assert_eq!(method.symbol.as_ref().unwrap().name, "make");
        assert_eq!(method.kind_str(), "method");

        // The `struct S;` declaration is captured as a `struct` symbol.
        let s_struct = chunks
            .iter()
            .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some("S"))
            .unwrap();
        assert_eq!(s_struct.kind_str(), "struct");

        // The impl wrapper lines remain in the module chunk per the overlap rule.
        let module = chunks.iter().find(|c| c.kind == ChunkKind::Module).unwrap();
        assert!(module.body.contains("impl S {"));
        assert!(!module.body.contains("fn make"));
    }

    #[test]
    fn test_chunk_file_rust_type_symbols_become_symbol_chunks() {
        // A struct, an enum, and a trait each become their own Symbol chunk with
        // the right kind_str; their lines must NOT reappear in any Module chunk.
        let content = r#"use std::fmt;

struct Point {
    x: f64,
    y: f64,
}

enum Shape {
    Circle,
    Square,
}

trait Area {
    fn area(&self) -> f64;
}
"#;
        let chunks = chunk_file("src/g.rs", content, Some(Language::Rust));

        let kind_of = |name: &str| -> &'static str {
            chunks
                .iter()
                .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some(name))
                .unwrap_or_else(|| panic!("no symbol chunk named {name}"))
                .kind_str()
        };
        assert_eq!(kind_of("Point"), "struct");
        assert_eq!(kind_of("Shape"), "enum");
        assert_eq!(kind_of("Area"), "trait");
        // The trait method is a separate Symbol chunk (range differs from trait).
        assert_eq!(kind_of("area"), "method");

        // The type declarations' lines must not reappear in a Module chunk: only
        // `use std::fmt;` (the lone uncovered top-level line) belongs to Module.
        let module_body: String = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Module)
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(module_body.contains("use std::fmt;"));
        assert!(!module_body.contains("struct Point"));
        assert!(!module_body.contains("enum Shape"));
        assert!(!module_body.contains("trait Area"));
    }

    #[test]
    fn test_chunk_file_go_type_specs() {
        // Go struct / interface / named-type all become Symbol chunks.
        let content = "package p\n\ntype Repo struct {\n    total int\n}\n\ntype Doer interface {\n    Do() int\n}\n\ntype Count int\n";
        let chunks = chunk_file("m.go", content, Some(Language::Go));
        let kind_of = |name: &str| -> &'static str {
            chunks
                .iter()
                .find(|c| c.symbol.as_ref().map(|s| s.name.as_str()) == Some(name))
                .unwrap_or_else(|| panic!("no symbol chunk named {name}"))
                .kind_str()
        };
        assert_eq!(kind_of("Repo"), "struct");
        assert_eq!(kind_of("Doer"), "interface");
        assert_eq!(kind_of("Count"), "type");
    }

    #[test]
    fn test_chunk_file_zero_symbols_is_one_module() {
        let content = "const X: u32 = 1;\nconst Y: u32 = 2;\n";
        let chunks = chunk_file("src/c.rs", content, Some(Language::Rust));
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, ChunkKind::Module);
        assert!(chunks[0].body.contains("const X"));
        assert!(chunks[0].body.contains("const Y"));
    }

    #[test]
    fn test_chunk_file_window_strategy() {
        // ~130 lines of python-ish content, language = None.
        let mut content = String::new();
        for i in 1..=130 {
            content.push_str(&format!("line_{i} = {i}\n"));
        }

        let chunks = chunk_file("script.py", &content, None);
        for c in &chunks {
            assert_eq!(c.kind, ChunkKind::Window);
        }

        // stride = 60 - 15 = 45. Starts (1-based): 1, 46, 91, 136>130 stop.
        // Windows: [1..60], [46..105], [91..130]. => 3 windows.
        assert_eq!(chunks.len(), 3, "expected 3 windows for 130 lines");

        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 60);
        assert_eq!(chunks[1].start_line, 46);
        assert_eq!(chunks[1].end_line, 105);
        assert_eq!(chunks[2].start_line, 91);
        assert_eq!(chunks[2].end_line, 130);

        // Overlap: window[1] starts 15 lines before window[0] ends.
        let overlap = chunks[0].end_line - chunks[1].start_line + 1;
        assert_eq!(overlap, WINDOW_OVERLAP);

        // Last window covers the tail.
        assert!(chunks.last().unwrap().body.contains("line_130 = 130"));
    }

    #[test]
    fn test_chunk_file_window_single_short_file() {
        let content = "a\nb\nc\n";
        let chunks = chunk_file("notes.md", content, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, ChunkKind::Window);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
    }

    /// The central correctness guard against silently dropping a file from the index:
    /// every emitted chunk must have a pairwise-distinct `(start_line, end_line)`
    /// pair across the WHOLE file (Symbol ∪ Module ∪ Window). A duplicate pair
    /// would yield a duplicate `path:start-end` id → the plain `INSERT INTO
    /// chunks` violates the PK → the per-file txn rolls back → the file silently
    /// drops from the index. The adversarial case is a 1-line symbol adjacent to
    /// a 1-line uncovered run (both could otherwise produce `path:L-L`).
    #[test]
    fn chunk_ids_pairwise_distinct() {
        use std::collections::HashSet;

        // Crafted file mixing: a top-level const (uncovered), a one-line symbol
        // (`fn one()` clamps to a single line), a one-line uncovered run right
        // after it, a multi-line function, and several uncovered runs.
        let content = "\
const A: u32 = 1;
fn one() { 1 }
const B: u32 = 2;
fn two() -> u32 {
    let x = 1;
    x + 1
}
const C: u32 = 3;
const D: u32 = 4;
fn three() { 3 }
const E: u32 = 5;
";

        // A handful of crafted Rust files plus a synthetic large module file.
        let mut cases: Vec<(String, Option<Language>)> =
            vec![(content.to_string(), Some(Language::Rust))];

        // Large constants file (no functions) → many module sub-chunks.
        let mut big = String::new();
        for i in 0..1000 {
            big.push_str(&format!("const ITEM_{i}: u32 = {i};\n"));
            if i % 7 == 0 {
                big.push('\n'); // sprinkle blank lines for boundary cuts
            }
        }
        cases.push((big, Some(Language::Rust)));

        // Unparsed window file (overlapping windows have distinct starts).
        let mut win = String::new();
        for i in 0..300 {
            win.push_str(&format!("line_{i}\n"));
        }
        cases.push((win, None));

        // Adversarial type-symbol overlap cases: a one-line
        // `trait { fn ... }` (trait + method both clamp to `L-L`), a multi-line
        // class whose method span sits inside the class span, etc. The chunker's
        // `(start,end)` dedup must keep every emitted chunk id distinct.
        cases.push((
            "trait Baz { fn x(&self); }\nstruct S { a: u32 }\n".to_string(),
            Some(Language::Rust),
        ));
        cases.push((
            "class C:\n    def m(self):\n        pass\n".to_string(),
            Some(Language::Python),
        ));
        cases.push((
            "class C { m(): number { return 1; } }\ninterface I { go(): void; }\n".to_string(),
            Some(Language::TypeScript),
        ));
        cases.push((
            "package p\ntype Q struct { a int }\ntype I interface { Do() }\n".to_string(),
            Some(Language::Go),
        ));

        for (i, (src, lang)) in cases.iter().enumerate() {
            let specs = chunk_file("file.rs", src, lang.clone());
            let mut seen: HashSet<(usize, usize)> = HashSet::new();
            for spec in &specs {
                assert!(
                    seen.insert((spec.start_line, spec.end_line)),
                    "case {i}: duplicate (start,end) = ({},{}) for kind {:?} — \
                     would collide on chunk id and drop the file",
                    spec.start_line,
                    spec.end_line,
                    spec.kind,
                );
            }
        }
    }

    /// (P1-6) A module-remainder run with a blank line near the cap must flush
    /// at the blank line, not mid-construct.
    #[test]
    fn module_split_prefers_blank_line() {
        // Build a no-symbol Rust file (one big uncovered run) whose first cap
        // window contains a blank line a few lines BEFORE the hard cap, so the
        // flush should land on that blank line.
        let mut content = String::new();
        // Lines 1..=MAX_CHUNK_LINES-3 are non-blank.
        for i in 1..=(MAX_CHUNK_LINES - 3) {
            content.push_str(&format!("const ITEM_{i}: u32 = {i};\n"));
        }
        // Blank line at MAX_CHUNK_LINES-2.
        content.push('\n');
        // More non-blank lines past the cap so a split is forced.
        for i in (MAX_CHUNK_LINES - 1)..=(MAX_CHUNK_LINES + 50) {
            content.push_str(&format!("const ITEM_{i}: u32 = {i};\n"));
        }

        let chunks = chunk_file("big.rs", &content, Some(Language::Rust));
        let modules: Vec<&ChunkSpec> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Module)
            .collect();
        assert!(
            modules.len() >= 2,
            "expected the run to split into >= 2 module chunks"
        );

        // The first module chunk must end exactly at the blank line
        // (MAX_CHUNK_LINES - 2), proving the cut preferred the blank boundary
        // over the hard cap at MAX_CHUNK_LINES.
        let blank_line = MAX_CHUNK_LINES - 2;
        assert_eq!(
            modules[0].end_line, blank_line,
            "first module chunk should flush at the preceding blank line"
        );
        // The second chunk starts on the line after the blank line.
        assert_eq!(modules[1].start_line, blank_line + 1);
    }

    /// A synthetic large file with a few small functions must split its
    /// uncovered remainder into multiple bounded Module chunks, each within the
    /// caps, with non-overlapping ranges that cover exactly the uncovered set.
    #[test]
    fn chunk_caps_module_remainder() {
        // ~1000 lines: 2 small functions + a large block of top-level consts.
        let mut content = String::new();
        content.push_str("fn lead() -> u32 { 1 }\n");
        for i in 0..960 {
            content.push_str(&format!("const ITEM_{i}: u32 = {i};\n"));
        }
        content.push_str("fn tail() -> u32 { 2 }\n");
        for i in 0..40 {
            content.push_str(&format!("static EXTRA_{i}: u32 = {i};\n"));
        }

        let chunks = chunk_file("consts.rs", &content, Some(Language::Rust));
        let modules: Vec<&ChunkSpec> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Module)
            .collect();

        assert!(
            modules.len() >= 5,
            "expected >= 5 module chunks, got {}",
            modules.len()
        );

        for m in &modules {
            let line_span = m.end_line - m.start_line + 1;
            assert!(
                line_span <= MAX_CHUNK_LINES,
                "module chunk {}-{} spans {} lines > MAX_CHUNK_LINES",
                m.start_line,
                m.end_line,
                line_span
            );
            assert!(
                m.body.len() <= MAX_CHUNK_BYTES,
                "module chunk {}-{} body {} bytes > MAX_CHUNK_BYTES",
                m.start_line,
                m.end_line,
                m.body.len()
            );
        }

        // Ranges are non-overlapping and strictly increasing.
        let mut ranges: Vec<(usize, usize)> =
            modules.iter().map(|m| (m.start_line, m.end_line)).collect();
        ranges.sort();
        for w in ranges.windows(2) {
            assert!(
                w[0].1 < w[1].0,
                "module ranges overlap: {:?} and {:?}",
                w[0],
                w[1]
            );
        }

        // The module chunks must cover EXACTLY the uncovered lines: the union of
        // their line sets equals all lines except the two function spans.
        let symbol_ranges: Vec<(usize, usize)> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Symbol)
            .map(|c| (c.start_line, c.end_line))
            .collect();
        let total = content.lines().count();
        for line in 1..=total {
            let in_symbol = symbol_ranges.iter().any(|(s, e)| line >= *s && line <= *e);
            let in_module = ranges.iter().any(|(s, e)| line >= *s && line <= *e);
            assert!(
                in_symbol != in_module,
                "line {line} must be in exactly one of symbol/module (symbol={in_symbol}, module={in_module})"
            );
        }
    }
}
