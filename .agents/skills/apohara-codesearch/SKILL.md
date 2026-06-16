```markdown
# apohara-codesearch Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill teaches you the core development conventions, workflows, and best practices for contributing to the `apohara-codesearch` Rust codebase. You'll learn how to add language support using tree-sitter grammars, manage reproducible benchmarking, perform releases, and document benchmarking processes. The guide covers coding conventions, workflow steps, testing patterns, and common developer commands.

---

## Coding Conventions

### File Naming

- **CamelCase** is used for file names.
  - Example: `parserLogic.rs`, `incrementalParser.rs`

### Imports

- **Relative imports** are preferred.
  - Example:
    ```rust
    use crate::parser::parse_file;
    use super::incremental;
    ```

### Exports

- **Named exports** are used.
  - Example:
    ```rust
    pub fn extract_symbols() { ... }
    pub struct ParseResult { ... }
    ```

### Commit Messages

- **Conventional commit** style with prefixes: `chore`, `feat`, `docs`, `style`, `release`.
  - Example: `feat: add tree-sitter-ruby grammar integration`

---

## Workflows

### Add New Tree-Sitter Grammar

**Trigger:** When adding structural extraction for a new language (e.g., Bash, Java, C, Ruby, C++).  
**Command:** `/add-grammar <language>`

1. Add the new `tree-sitter-<lang>` dependency to the workspace:
    - Update `Cargo.toml`, `Cargo.lock`, and `crates/apohara-indexer/Cargo.toml`.
2. Update parser logic to support the new language variant:
    - Edit `crates/apohara-indexer/src/parser.rs` and `incremental.rs`.
    - Example:
      ```rust
      match language {
          Language::Ruby => parse_ruby_source(source),
          // ...
      }
      ```
3. Add language detection and extraction logic.
4. Add new test fixtures:
    - Place files in `crates/apohara-indexer/tests/fixtures/fixture.<ext>` and `imports.<ext>`.
5. Write unit tests for detection, extraction, and import logic.
6. Add a new fuzz target and update `fuzz/Cargo.toml`:
    - Create `fuzz/fuzz_targets/parse_<lang>_source.rs`.
7. Update integration tests if necessary (e.g., `integration.rs` for extension handling).
8. Document binary size impact and test results in the commit message.

---

### Freeze Benchmark Corpus and Guard

**Trigger:** When establishing or updating a benchmark baseline for performance or regression testing.  
**Command:** `/freeze-corpus`

1. Copy or update files in `tests/fixtures/bench-corpus-frozen-*/`.
2. Add or update a guard test to check content hashes:
    - Edit `crates/apohara-codesearch/tests/corpus_freeze.rs`.
    - Example:
      ```rust
      #[test]
      fn test_corpus_hashes() {
          // assert_eq!(calculate_hash("file.rs"), "expected_hash");
      }
      ```
3. Update or add relevant dev-dependencies (e.g., `sha2`) in `Cargo.toml` and `Cargo.lock`.
4. Document the freeze and hashes in the commit message and/or `BENCHMARK.md`.

---

### Release Version Bump and Docs

**Trigger:** When cutting a new release version.  
**Command:** `/release <version>`

1. Update version numbers in all relevant manifests:
    - `crates/apohara-indexer/Cargo.toml`
    - `crates/apohara-codesearch/Cargo.toml`
    - `npm/package.json`
    - `marketplace.json`
    - `.claude-plugin/plugin.json`
2. Update `CHANGELOG.md` with a new version entry.
3. Update or add release notes to `BENCHMARK.md`, `README.md`, and/or `progress.txt`.
4. Prepare a git tag and document it in the commit message.

---

### Update or Add Benchmark Documentation

**Trigger:** When recording or explaining a new benchmark baseline or corpus freeze.  
**Command:** `/doc-benchmark`

1. Edit `BENCHMARK.md` to add a new section or update the existing one with:
    - Commit links
    - Rationale
    - Measurement plan
2. Reference relevant corpus freeze commits and guard tests.
3. Ensure documentation is cross-linked to code/tests.

---

## Testing Patterns

- **Framework:** Not explicitly specified, but Rust's built-in test framework is likely used.
- **File Pattern:** Test files and fixtures are typically located in `crates/apohara-indexer/tests/` and named with `.rs` extensions.
- **Example Test:**
    ```rust
    #[test]
    fn test_language_detection() {
        let result = detect_language("example.rb");
        assert_eq!(result, Language::Ruby);
    }
    ```
- **Fuzz Testing:** Fuzz targets are located in `fuzz/fuzz_targets/` (e.g., `parse_ruby_source.rs`).

---

## Commands

| Command            | Purpose                                                        |
|--------------------|----------------------------------------------------------------|
| /add-grammar <lang>| Add support for a new language with a tree-sitter grammar      |
| /freeze-corpus     | Freeze benchmark corpus and add guard test for reproducibility |
| /release <version> | Perform a release, bump versions, update changelogs/docs       |
| /doc-benchmark     | Document benchmark corpus, rationale, and measurement plan     |
```
