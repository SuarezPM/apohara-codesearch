// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Corpus-freeze regression guard for the v0.3.0 default-flip golden test.
// See .omc/plans/apohara-codesearch-3frentes.md §6 "Corpus freeze" for the rationale.
//
// Two corpora are pinned to specific content-hashes:
//   - Corpus A (tests/fixtures/bench-corpus-frozen-A/): a copy of examples/bench-corpus/
//     at the v0.2.0 commit, used as the BENCHMARK baseline for the F3 measurement.
//   - Corpus B (tests/fixtures/bench-corpus-frozen-B/queries.json): a 10-query subset
//     used by the F3 golden test (F3-FLIP-CHECK).
//
// If either corpus drifts, this test fails. To update a freeze, do it as its own
// atomic commit with `chore(bench): refreeze corpus X` and a new entry in CHANGELOG.

use std::path::Path;

const CORPUS_A_PATH: &str = "tests/fixtures/bench-corpus-frozen-A";
const CORPUS_A_EXPECTED_HASH: &str =
    "d045e86ca978935f1a292b631941b8bde3d3341f49179a94fdfbebb1a2890b29";

const CORPUS_B_PATH: &str = "tests/fixtures/bench-corpus-frozen-B/queries.json";
const CORPUS_B_EXPECTED_HASH: &str =
    "f5da3d598daee07528676a4ab528db7a70c15a7bd37d245da443857c55338ab2";

/// Compute a content-hash for a directory: sha256 of (sorted, sha256-per-file).
/// Equivalent to `find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum`.
fn dir_content_hash(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    collect_files(root, root, &mut files);
    let mut combined = String::new();
    for (rel, hash) in &files {
        combined.push_str(&format!("{}  {}\n", hash, rel));
    }
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn collect_files(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
    use sha2::{Digest, Sha256};
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel_str = rel.to_string_lossy().to_string();
            if let Ok(bytes) = std::fs::read(&path) {
                let mut h = Sha256::new();
                h.update(&bytes);
                let file_hash = format!("{:x}", h.finalize());
                out.insert(rel_str, file_hash);
            }
        }
    }
}

#[test]
fn corpus_a_frozen_against_v0_2_0() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(CORPUS_A_PATH);
    assert!(
        path.exists(),
        "Corpus A path missing: {}. Re-create the freeze commit.",
        path.display()
    );
    let actual = dir_content_hash(&path);
    assert_eq!(
        actual, CORPUS_A_EXPECTED_HASH,
        "Corpus A drifted! Expected hash {} but got {}. \
         If you intentionally refroze, update CORPUS_A_EXPECTED_HASH in this test \
         and add a 'chore(bench): refreeze corpus A' commit in CHANGELOG.",
        CORPUS_A_EXPECTED_HASH, actual
    );
}

#[test]
fn corpus_b_frozen_against_v0_2_0() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(CORPUS_B_PATH);
    assert!(
        path.exists(),
        "Corpus B path missing: {}. Re-create the freeze commit.",
        path.display()
    );
    let bytes = std::fs::read(&path).expect("read corpus B");
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let actual = format!("{:x}", h.finalize());
    assert_eq!(
        actual, CORPUS_B_EXPECTED_HASH,
        "Corpus B drifted! Expected hash {} but got {}. \
         If you intentionally refroze, update CORPUS_B_EXPECTED_HASH in this test \
         and add a 'chore(bench): reffreeze corpus B' commit in CHANGELOG.",
        CORPUS_B_EXPECTED_HASH, actual
    );
}
