// SPDX-License-Identifier: MIT OR Apache-2.0

//! Multi-repo sidecar registry: a `path -> index.db` map (Decision E1).
//!
//! Each repo keeps its OWN `<root>/.apohara-codesearch/index.db` (the "one repo
//! per DB" invariant). This registry is the only SHARED mutable state — a tiny
//! JSON file under the XDG config dir (e.g. `~/.config/apohara-codesearch/
//! registry.json`) recording which repos have an index. It is deliberately NOT
//! a shared SQLite DB (Decision E2 is rejected): a central DB would break the
//! per-repo invariant and put every reindex in cross-process write contention,
//! which the process-level `build_locks` (`server.rs:99-113`) cannot serialize.
//!
//! ## Concurrency (AC7)
//!
//! Writes are ATOMIC: the new content is written to a temp file in the same
//! directory and then `rename`d over the target, so a reader never observes a
//! torn/partial file (rename is atomic within a filesystem). A LOST UPDATE is
//! accepted: two racing writers may have one overwrite the other. That is
//! self-healed — the dropped repo re-registers on its next reindex, and
//! read-time self-heal prunes entries whose `index.db` no longer exists. We do
//! NOT take a read-modify-write lock for v0.7 (declared in the plan, N6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

/// Per-write nonce making each temp file name unique even across threads of the
/// SAME process. The process id alone is NOT enough: two threads writing
/// concurrently would share `.registry.<pid>.tmp` and one could rename a file
/// the other is still writing — a torn read. pid keeps cross-process temps
/// distinct; the nonce keeps intra-process temps distinct.
static TMP_NONCE: AtomicU64 = AtomicU64::new(0);

/// Application config-dir name under the XDG base (e.g. `~/.config/<name>`).
const APP_CONFIG_DIR: &str = "apohara-codesearch";
/// Registry file name inside the config dir.
const REGISTRY_FILE: &str = "registry.json";

/// The path->index map. Keys are canonical repo root paths (as strings); values
/// are the absolute path to that repo's `index.db`. A `BTreeMap` keeps the
/// serialized JSON deterministic (sorted keys) so two equal registries serialize
/// byte-identically.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    entries: BTreeMap<String, PathBuf>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered repos.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no repos are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The registered `index.db` path for `root`, or `None`. `root` is
    /// canonicalized for the lookup so different spellings resolve to one entry.
    pub fn get(&self, root: &Path) -> Option<&PathBuf> {
        self.entries.get(&canonical_key(root))
    }

    /// Register (or update) `root -> index_db`. In-memory only; persist with
    /// [`save`].
    pub fn insert(&mut self, root: &Path, index_db: &Path) {
        self.entries
            .insert(canonical_key(root), index_db.to_path_buf());
    }

    /// Remove `root`'s entry if present, returning whether anything was removed.
    pub fn remove(&mut self, root: &Path) -> bool {
        self.entries.remove(&canonical_key(root)).is_some()
    }

    /// Drop entries whose `index.db` no longer exists on disk (self-heal),
    /// returning how many were pruned.
    pub fn prune_missing(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, index_db| index_db.exists());
        before - self.entries.len()
    }

    /// Serialize to a stable JSON object (sorted keys). Public for testing the
    /// on-disk shape without touching the filesystem.
    fn to_json(&self) -> String {
        // BTreeMap<String, PathBuf> -> JSON object {root: index_db}. Lossy path
        // rendering is fine: these are display/lookup keys, not re-opened bytes.
        let map: BTreeMap<&str, String> = self
            .entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.to_string_lossy().into_owned()))
            .collect();
        serde_json::to_string_pretty(&map).expect("serialize registry map")
    }

    /// Parse from JSON, tolerating an empty/whitespace string as an empty
    /// registry (a fresh install).
    fn from_json(s: &str) -> Result<Self> {
        if s.trim().is_empty() {
            return Ok(Self::new());
        }
        let map: BTreeMap<String, String> =
            serde_json::from_str(s).context("parse registry json")?;
        Ok(Self {
            entries: map
                .into_iter()
                .map(|(k, v)| (k, PathBuf::from(v)))
                .collect(),
        })
    }
}

/// Canonicalize `root` for use as a stable map key, falling back to the raw path
/// when canonicalization fails (e.g. the dir does not exist yet). Mirrors
/// `build_lock_for` / `repo_id_for`.
fn canonical_key(root: &Path) -> String {
    std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Resolve the registry file path under the XDG config dir, creating the config
/// dir if needed.
pub fn registry_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("resolve XDG config dir")?
        .join(APP_CONFIG_DIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create config dir {}", dir.display()))?;
    Ok(dir.join(REGISTRY_FILE))
}

/// Load the registry from `path`, returning an empty registry when the file does
/// not yet exist. Prunes stale entries (self-heal) but does NOT write back —
/// callers that want the pruned state persisted should [`save`] afterwards.
pub fn load(path: &Path) -> Result<Registry> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::new()),
        Err(e) => return Err(e).with_context(|| format!("read registry {}", path.display())),
    };
    let mut registry = Registry::from_json(&raw)?;
    registry.prune_missing();
    Ok(registry)
}

/// Persist `registry` to `path` ATOMICALLY: write to a temp file in the same
/// directory, then `rename` over the target. A reader therefore always sees a
/// complete, valid file — never a torn write (AC7). The temp file name carries
/// BOTH the process id (distinct across processes) AND a per-write atomic nonce
/// (distinct across threads of one process), so no two concurrent writers ever
/// share a temp path — the bug a pid-only name would have.
pub fn save(path: &Path, registry: &Registry) -> Result<()> {
    let parent = path
        .parent()
        .context("registry path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create registry dir {}", parent.display()))?;

    let nonce = TMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".registry.{}.{nonce}.tmp", std::process::id()));
    std::fs::write(&tmp, registry.to_json())
        .with_context(|| format!("write temp registry {}", tmp.display()))?;
    // Atomic replace. On failure, best-effort clean up the temp file.
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e).context(format!("rename temp registry onto {}", path.display()))
    })?;
    Ok(())
}

/// Register `root -> index_db` in the registry at `path` (load → insert → save).
/// Convenience for the common reindex-time update. Self-heals stale entries on
/// the load. NOTE: this is load-modify-save, NOT atomic across the whole
/// sequence — a concurrent writer may lose its update (accepted, AC7); only the
/// individual file write is atomic.
pub fn register(path: &Path, root: &Path, index_db: &Path) -> Result<()> {
    let mut registry = load(path)?;
    registry.insert(root, index_db);
    save(path, &registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn registry_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let reg_path = dir.path().join("registry.json");

        // A real index.db so the entry survives self-heal pruning.
        let idx = dir.path().join("a-index.db");
        fs::write(&idx, b"db").unwrap();
        let root = dir.path().join("repo-a");
        fs::create_dir_all(&root).unwrap();

        register(&reg_path, &root, &idx).unwrap();

        let loaded = load(&reg_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get(&root), Some(&idx));
    }

    #[test]
    fn registry_self_heals_stale_entries() {
        // An entry whose index.db no longer exists is pruned on read.
        let dir = tempdir().unwrap();
        let reg_path = dir.path().join("registry.json");

        let live_idx = dir.path().join("live.db");
        fs::write(&live_idx, b"db").unwrap();
        let live_root = dir.path().join("live-repo");
        fs::create_dir_all(&live_root).unwrap();

        let dead_idx = dir.path().join("dead.db"); // never created
        let dead_root = dir.path().join("dead-repo");
        fs::create_dir_all(&dead_root).unwrap();

        let mut reg = Registry::new();
        reg.insert(&live_root, &live_idx);
        reg.insert(&dead_root, &dead_idx);
        save(&reg_path, &reg).unwrap();

        let healed = load(&reg_path).unwrap();
        assert_eq!(healed.len(), 1, "the dead entry must be pruned on read");
        assert!(healed.get(&live_root).is_some());
        assert!(healed.get(&dead_root).is_none());
    }

    #[test]
    fn registry_write_is_atomic_no_torn_file() {
        // After many concurrent atomic writes the file is always complete &
        // parseable (no torn write). A lost update is acceptable; corruption is
        // not. We assert every reader-observed snapshot parses cleanly, and the
        // final file is valid.
        use std::sync::Arc;
        use std::thread;

        let dir = tempdir().unwrap();
        let reg_path = Arc::new(dir.path().join("registry.json"));

        // Seed a real index.db so the surviving entry is not pruned.
        let idx = dir.path().join("shared.db");
        fs::write(&idx, b"db").unwrap();
        let root = dir.path().join("shared-repo");
        fs::create_dir_all(&root).unwrap();
        register(&reg_path, &root, &idx).unwrap();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let p = Arc::clone(&reg_path);
            let r = root.clone();
            let i = idx.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..25 {
                    // Each writer does a full load→insert→save. Concurrent runs
                    // may lose updates but must never produce a torn file: every
                    // load below must parse without error.
                    let _ = register(&p, &r, &i);
                    // A concurrent reader: must always parse cleanly.
                    load(&p).expect("registry must always parse (no torn write)");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // Final file is valid and still contains the shared repo.
        let final_reg = load(&reg_path).expect("final registry parses");
        assert!(
            final_reg.get(&root).is_some(),
            "the shared repo survives concurrent writers"
        );
    }

    #[test]
    fn empty_string_is_empty_registry() {
        // A fresh/empty file must not error.
        assert!(Registry::from_json("").unwrap().is_empty());
        assert!(Registry::from_json("   \n").unwrap().is_empty());
    }

    #[test]
    fn json_shape_is_deterministic() {
        // Two equal registries serialize byte-identically (sorted keys).
        let dir = tempdir().unwrap();
        let idx = dir.path().join("d.db");
        let root_b = dir.path().join("b");
        let root_a = dir.path().join("a");

        let mut r1 = Registry::new();
        r1.insert(&root_b, &idx);
        r1.insert(&root_a, &idx);

        let mut r2 = Registry::new();
        r2.insert(&root_a, &idx);
        r2.insert(&root_b, &idx);

        assert_eq!(r1.to_json(), r2.to_json(), "key order must not matter");
    }
}
