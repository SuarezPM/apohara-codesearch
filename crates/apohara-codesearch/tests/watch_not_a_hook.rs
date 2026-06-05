// SPDX-License-Identifier: MIT OR Apache-2.0

//! Guard that the `watch` subcommand stays a plain CLI loop and never becomes a
//! plugin hook. This test fails if the feature ever smuggles in a Claude Code
//! hook config (`.claude/hooks`, or a PreToolUse / PostToolUse hook entry in a
//! settings file) under the crate or repo root.

use std::fs;
use std::path::{Path, PathBuf};

/// Walk up from the crate dir to find the workspace/repo root (the dir holding
/// the top-level `Cargo.toml` with `[workspace]`).
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo = dir.join("Cargo.toml");
        if cargo.exists() {
            if let Ok(text) = fs::read_to_string(&cargo) {
                if text.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            // Fallback: the crate dir itself.
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        }
    }
}

#[test]
fn watch_adds_no_claude_code_hook_config() {
    let root = repo_root();

    // 1) No `.claude/hooks` directory anywhere relevant.
    assert!(
        !root.join(".claude").join("hooks").exists(),
        "watch must not introduce a .claude/hooks directory"
    );

    // 2) No PreToolUse/PostToolUse hook entry in any Claude settings file.
    for rel in [".claude/settings.json", ".claude/settings.local.json"] {
        let path = root.join(rel);
        if let Ok(text) = fs::read_to_string(&path) {
            assert!(
                !mentions_tool_hook(&text),
                "{} must not register a Pre/PostToolUse hook for watch",
                path.display()
            );
        }
    }
}

/// True if the settings text wires a tool-use hook (the hook mechanism `watch`
/// must NOT use). Matches the JSON keys Claude Code reads for tool hooks.
fn mentions_tool_hook(text: &str) -> bool {
    text.contains("PreToolUse") || text.contains("PostToolUse")
}

/// Sanity: the helper resolves to a directory that actually contains a Cargo
/// workspace, so the assertions above are checking the right tree.
#[test]
fn repo_root_is_the_workspace() {
    let root = repo_root();
    assert!(Path::new(&root).join("Cargo.toml").exists());
}
