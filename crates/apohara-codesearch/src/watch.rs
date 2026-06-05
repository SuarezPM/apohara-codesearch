// SPDX-License-Identifier: MIT OR Apache-2.0

//! `watch` subcommand: a long-running filesystem-watch loop that incrementally
//! reindexes a repository on change.
//!
//! This is a plain CLI subcommand and a blocking loop, not a plugin hook. It
//! writes to the same single SQLite index file as `serve`; it adds no new state,
//! service, or transport. All diagnostics go to STDERR; STDOUT is kept clean.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::{Context, Result};
use apohara_indexer::{migrate, open_db, reindex, ReindexReport};
use notify::{recommended_watcher, RecursiveMode, Watcher};

/// Sub-directory under the repo root holding the index database (mirrors the
/// server's `INDEX_DIR`). Writes under it (index.db, WAL) MUST NOT retrigger a
/// reindex, or the watcher would loop on its own output.
const INDEX_DIR: &str = ".apohara-codesearch";

/// Debounce window: coalesce a burst of filesystem events into a single
/// incremental reindex. Picked short enough to feel live, long enough to absorb
/// an editor's save burst (write + rename + chmod) into one reindex.
const WATCH_DEBOUNCE_MS: u64 = 300;

/// Resolve the index database path `<root>/.apohara-codesearch/index.db`,
/// creating the parent directory if needed (mirrors the server's `db_path`;
/// rusqlite's `Connection::open` does not create missing parent dirs).
fn db_path(root: &Path) -> Result<PathBuf> {
    let dir = root.join(INDEX_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("create index dir {}", dir.display()))?;
    Ok(dir.join("index.db"))
}

/// True when `path` lies inside the `<root>/.apohara-codesearch/` index dir.
/// The walker already excludes that dir from *indexing*; the watcher must also
/// ignore *events* under it so a reindex's own writes (index.db, `-wal`, `-shm`)
/// do not retrigger another reindex in an endless loop.
fn is_index_path(root: &Path, path: &Path) -> bool {
    path.starts_with(root.join(INDEX_DIR))
}

/// Run one incremental reindex of `root` against its index database.
///
/// Factored out of the blocking loop so it can be unit-tested synchronously
/// without spawning a watcher: a test mutates a file, calls this, and asserts
/// the report reflects the change. `force == false` → content-hash incremental.
pub fn handle_change(root: &Path) -> Result<ReindexReport> {
    let db = db_path(root)?;
    let conn = open_db(&db).context("open_db")?;
    migrate(&conn).context("migrate")?;
    reindex(&conn, root, false).context("reindex")
}

/// Run the blocking filesystem-watch loop over `root`.
///
/// Sets up a recommended (platform-native) recursive watcher, then debounces
/// bursts of events through `WATCH_DEBOUNCE_MS` before calling `handle_change`.
/// Events whose every path is inside the index dir are dropped so a reindex's
/// own writes never retrigger it. Returns only on an unrecoverable watcher error
/// (the channel disconnecting); otherwise loops forever.
pub fn run_watch(root: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize watch root {}", root.display()))?;

    // Build the index up front (incremental on an existing db, full on a fresh
    // one) so the first reported reindex is meaningful and the db exists before
    // any event arrives.
    let report = handle_change(&root)?;
    log_reindex(&report);

    // std::sync::mpsc::Sender<Result<Event>> implements notify's EventHandler,
    // so the recommended watcher can push events straight into this channel.
    let (tx, rx) = mpsc::channel();
    let mut watcher = recommended_watcher(tx).context("create filesystem watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", root.display()))?;

    eprintln!(
        "apohara-codesearch: watching {} (debounce {}ms, Ctrl-C to stop)",
        root.display(),
        WATCH_DEBOUNCE_MS
    );

    let debounce = Duration::from_millis(WATCH_DEBOUNCE_MS);
    loop {
        // Block until the first relevant event, then keep draining (resetting the
        // window) until the burst goes quiet for `debounce`. One reindex per burst.
        match rx.recv() {
            Ok(event) => {
                if !relevant(&root, event) {
                    continue;
                }
            }
            // All senders dropped: the watcher died. Nothing more will arrive.
            Err(_) => return Ok(()),
        }

        // Coalesce the rest of the burst: drain until the stream is quiet for a
        // full debounce window, then reindex once. We drain ALL queued events
        // here (relevant or not) purely to find the quiet point — relevance was
        // already decided by the outer recv that armed this burst, and the
        // subsequent reindex is incremental so any over-coalescing is harmless.
        loop {
            match rx.recv_timeout(debounce) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        match handle_change(&root) {
            Ok(report) => log_reindex(&report),
            // A transient reindex error must not kill the watcher; log and keep
            // watching so the next save can recover.
            Err(e) => eprintln!("apohara-codesearch: reindex failed: {e:#}"),
        }
    }
}

/// Decide whether a watcher event should trigger a reindex. An event is relevant
/// when it (a) is a content-changing kind — create/modify/remove, NOT a pure
/// read/open access — and (b) touches at least one path outside the index dir.
///
/// Filtering out `Access` (open/read) events is essential: reindexing itself
/// *opens and reads* the source files to walk them, which emits access events on
/// those very files. Treating reads as changes would make every reindex retrigger
/// another — an infinite loop. Reads never change content, so they are ignored.
/// The index-dir filter additionally drops the watcher's own index.db/WAL writes.
fn relevant(root: &Path, event: notify::Result<notify::Event>) -> bool {
    use notify::EventKind;
    match event {
        Ok(event) => {
            let content_changing = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
            );
            content_changing && event.paths.iter().any(|p| !is_index_path(root, p))
        }
        // A watcher-level error (e.g. a dropped inotify event) is conservatively
        // treated as a reason to reindex rather than silently miss a change.
        Err(_) => true,
    }
}

/// Emit a one-line reindex summary to STDERR (STDOUT stays clean for any future
/// machine-readable use, matching the `serve`/MCP stdout discipline).
fn log_reindex(report: &ReindexReport) {
    eprintln!(
        "apohara-codesearch: reindex ({}) files_indexed={} chunks={} duration_ms={}",
        if report.incremental {
            "incremental"
        } else {
            "full"
        },
        report.files_indexed,
        report.chunks,
        report.duration_ms,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::Sender;
    use tempfile::TempDir;

    /// Seed a tiny repo with one Rust source file.
    fn seed_repo(root: &Path) {
        fs::write(root.join("lib.rs"), "pub fn original() -> u32 { 1 }\n").unwrap();
    }

    /// Mutating a file under the watched root makes the *next* incremental
    /// reindex pick it up. Deterministic — no watcher, no sleeps:
    /// it drives `handle_change` (the same function the loop calls per burst)
    /// directly and asserts the change-handling reports the changed file.
    #[test]
    fn handle_change_reindexes_a_mutated_file() {
        let repo = TempDir::new().unwrap();
        let root = repo.path();
        seed_repo(root);

        // First call: cold db → full index of the one file.
        let first = handle_change(root).unwrap();
        assert_eq!(first.files_indexed, 1, "initial index sees the one file");

        // No change → incremental reindex touches zero files (content-hash skip).
        let unchanged = handle_change(root).unwrap();
        assert!(unchanged.incremental, "second run is incremental");
        assert_eq!(
            unchanged.files_indexed, 0,
            "an unchanged repo reindexes no files"
        );

        // Mutate the file; the next incremental reindex must reflect exactly it.
        fs::write(
            root.join("lib.rs"),
            "pub fn original() -> u32 { 2 }\npub fn added() -> u32 { 3 }\n",
        )
        .unwrap();
        let changed = handle_change(root).unwrap();
        assert!(changed.incremental);
        assert_eq!(
            changed.files_indexed, 1,
            "the mutated file is reindexed (and only it)"
        );
    }

    /// Self-exclude: events under `<root>/.apohara-codesearch/` (index.db, WAL,
    /// SHM) must be classified as NOT relevant, so a reindex's own writes can
    /// never retrigger the watcher. Source files outside it stay relevant.
    #[test]
    fn index_dir_events_are_self_excluded() {
        let root = Path::new("/repo");
        assert!(is_index_path(root, &root.join(INDEX_DIR).join("index.db")));
        assert!(is_index_path(
            root,
            &root.join(INDEX_DIR).join("index.db-wal")
        ));
        assert!(!is_index_path(root, &root.join("src").join("main.rs")));

        // `relevant` mirrors that: a pure index-dir event is ignored, a mixed or
        // outside event triggers a reindex.
        let mk = |kind: notify::EventKind, paths: Vec<PathBuf>| -> notify::Result<notify::Event> {
            Ok(notify::Event {
                kind,
                paths,
                attrs: Default::default(),
            })
        };
        let modify = notify::EventKind::Modify(notify::event::ModifyKind::Any);
        assert!(
            !relevant(
                root,
                mk(modify, vec![root.join(INDEX_DIR).join("index.db-wal")])
            ),
            "index-dir-only event must not trigger reindex"
        );
        assert!(
            relevant(root, mk(modify, vec![root.join("src.rs")])),
            "a source-file modify event must trigger reindex"
        );

        // A pure read/open of a source file must NOT trigger a reindex —
        // otherwise reindexing (which reads source files) would loop forever.
        let access = notify::EventKind::Access(notify::event::AccessKind::Open(
            notify::event::AccessMode::Any,
        ));
        assert!(
            !relevant(root, mk(access, vec![root.join("src.rs")])),
            "a read/open access event must not trigger reindex (loop guard)"
        );
    }

    /// Smoke test of the real notify watcher with a TIGHT timeout: writing a file
    /// under the watched root delivers at least one relevant event. Bounded by
    /// `recv_timeout`, so it fails fast rather than hanging if events never come.
    #[test]
    fn real_watcher_delivers_relevant_event() {
        let repo = TempDir::new().unwrap();
        let root = repo.path().canonicalize().unwrap();
        seed_repo(&root);

        let (tx, rx): (Sender<notify::Result<notify::Event>>, _) = mpsc::channel();
        let mut watcher = recommended_watcher(tx).unwrap();
        watcher.watch(&root, RecursiveMode::Recursive).unwrap();

        // Trigger an event after the watcher is armed.
        fs::write(root.join("lib.rs"), "pub fn changed() {}\n").unwrap();

        // Drain up to a bounded number of events within a tight total budget,
        // asserting at least one is relevant (outside the index dir).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw_relevant = false;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
                    if relevant(&root, event) {
                        saw_relevant = true;
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(
            saw_relevant,
            "watcher should deliver a relevant write event"
        );
    }
}
