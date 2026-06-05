// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gitignore-aware filesystem traversal for indexing.
//!
//! Walks a repository root using the `ignore` crate (so `.gitignore`,
//! `.ignore`, global gitignore, and hidden-file rules are honored), yielding one
//! [`WalkedFile`] per indexable file. The walker:
//!
//! - self-excludes the index's own `<root>/.apohara-codesearch/` directory via
//!   an explicit override (NOT relying on it being gitignored in the target
//!   repo),
//! - skips non-UTF8 / binary files (anything whose bytes are not valid UTF-8),
//! - skips files larger than [`MAX_FILE_BYTES`],
//! - normalizes `rel_path` to a forward-slashed path relative to `root`, stable
//!   across CWD and OS.

use std::path::Path;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use crate::parser::{detect_language, Language};

/// Maximum file size (in bytes) the walker will read. Files larger than this are
/// skipped — they are almost always generated artifacts, vendored blobs, or
/// minified bundles that pollute the index without adding searchable signal.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The index's own working directory, relative to the repo root. Self-excluded
/// from traversal so we never index our own database/state.
const SELF_DIR: &str = ".apohara-codesearch";

/// One file discovered by [`walk_repo`], ready to be chunked and indexed.
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// Path relative to the walk root, forward-slashed and normalized.
    pub rel_path: String,
    /// Detected language, or `None` for files outside the supported set.
    pub language: Option<Language>,
    /// Full file contents (guaranteed valid UTF-8).
    pub content: String,
}

/// Walk `root`, returning every indexable file as a [`WalkedFile`].
///
/// Traversal honors `.gitignore` and friends via the `ignore` crate. Files are
/// skipped when they are directories, larger than [`MAX_FILE_BYTES`], unreadable,
/// or not valid UTF-8. The `<root>/.apohara-codesearch/` directory is excluded
/// explicitly regardless of the target repo's ignore rules.
pub fn walk_repo(root: &Path) -> Vec<WalkedFile> {
    // Explicit self-exclusion: an override glob beats the target repo's
    // (possibly absent) gitignore entry for our own state directory. In the
    // `ignore` override model, a leading `!` marks an ignore (exclude) pattern.
    // The globs are static and valid, so `build()` cannot fail here.
    let mut overrides = OverrideBuilder::new(root);
    overrides
        .add(&format!("!{}", SELF_DIR))
        .and_then(|b| b.add(&format!("!{}/**", SELF_DIR)))
        .expect("static self-exclusion globs are valid");
    let overrides = overrides.build().expect("static override set builds");

    let walker = WalkBuilder::new(root)
        .overrides(overrides)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        // Honor `.gitignore` even when `root` is not inside a git work tree.
        // Without this, `ignore` only applies gitignore rules under a `.git`
        // directory, which would silently disable filtering for repos indexed
        // before init or for non-git source trees.
        .require_git(false)
        .build();

    let mut files = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Only regular files.
        match entry.file_type() {
            Some(ft) if ft.is_file() => {}
            _ => continue,
        }

        let path = entry.path();

        // Belt-and-suspenders: skip anything inside our self dir even if the
        // override did not catch it (e.g. symlinked or odd path shapes).
        let rel_path = match normalize_rel_path(root, path) {
            Some(p) => p,
            None => continue,
        };
        if rel_path == SELF_DIR || rel_path.starts_with(&format!("{}/", SELF_DIR)) {
            continue;
        }

        // Size gate before reading the bytes.
        match entry.metadata() {
            Ok(md) if md.len() > MAX_FILE_BYTES => continue,
            Ok(_) => {}
            Err(_) => continue,
        }

        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Skip non-UTF8 / binary content.
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let language = detect_language(path);

        files.push(WalkedFile {
            rel_path,
            language,
            content,
        });
    }

    files
}

/// Normalize `path` to a forward-slashed string relative to `root`. Returns
/// `None` if `path` is not under `root`.
fn normalize_rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut out = String::new();
    for (i, comp) in rel.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&comp.as_os_str().to_string_lossy());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_walk_repo_basic_and_language_detection() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("a.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("b.ts"), "function f() {}\n").unwrap();
        fs::write(root.join("c.txt"), "plain text\n").unwrap();

        let mut files = walk_repo(root);
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].rel_path, "a.rs");
        assert_eq!(files[0].language, Some(Language::Rust));
        assert_eq!(files[1].rel_path, "b.ts");
        assert_eq!(files[1].language, Some(Language::TypeScript));
        assert_eq!(files[2].rel_path, "c.txt");
        assert_eq!(files[2].language, None);
    }

    #[test]
    fn test_walk_repo_self_excludes_apohara_dir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("keep.rs"), "fn keep() {}\n").unwrap();
        let self_dir = root.join(SELF_DIR);
        fs::create_dir_all(&self_dir).unwrap();
        fs::write(self_dir.join("index.db"), "should not be indexed\n").unwrap();
        fs::write(self_dir.join("nested.rs"), "fn nope() {}\n").unwrap();

        let files = walk_repo(root);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "keep.rs");
    }

    #[test]
    fn test_walk_repo_honors_gitignore() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(root.join("kept.rs"), "fn kept() {}\n").unwrap();
        fs::write(root.join("ignored.rs"), "fn ignored() {}\n").unwrap();

        let rels: Vec<String> = walk_repo(root).into_iter().map(|f| f.rel_path).collect();
        assert!(rels.contains(&"kept.rs".to_string()));
        assert!(!rels.contains(&"ignored.rs".to_string()));
    }

    #[test]
    fn test_walk_repo_skips_oversized_and_binary() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Oversized file (> MAX_FILE_BYTES).
        let big = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
        fs::write(root.join("big.rs"), &big).unwrap();
        // Non-UTF8 / binary content.
        fs::write(root.join("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        // A normal file to confirm the walk still yields something.
        fs::write(root.join("ok.rs"), "fn ok() {}\n").unwrap();

        let rels: Vec<String> = walk_repo(root).into_iter().map(|f| f.rel_path).collect();
        assert_eq!(rels, vec!["ok.rs".to_string()]);
    }
}
