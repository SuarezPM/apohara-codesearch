# Contributing to apohara-codesearch

Thanks for your interest in contributing. This document covers the basics for
building, testing, and submitting changes.

## Building and testing

This is a Cargo workspace. The **default build ships no model and makes no
network calls** — keep it that way (see *Non-negotiable principles* in
[`GOVERNANCE.md`](GOVERNANCE.md)).

```sh
# Build everything (default, zero-model)
cargo build

# Build the release binary
cargo build --release -p apohara-codesearch

# Run the full test suite
cargo test --workspace --all-targets
```

The real embedder backend is an **opt-in, feature-gated** path that loads a
user-supplied local model from disk (no download):

```sh
# Compile the EmbeddingGemma path (candle, pure Rust, no native deps)
cargo build -p apohara-indexer --features gguf-embed

# The gated parity test needs a local weights dir:
APOHARA_EMBED_MODEL=/path/to/embeddinggemma cargo test -p apohara-indexer --features gguf-embed
```

## Quality gate

Every commit MUST keep the following green. CI enforces all of them; please run
them locally before opening a PR:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Pull requests that break `cargo test` or introduce clippy warnings will not be
merged.

## Coding standards

The project's **required coding style is enforced automatically**, so there is no
style guide to memorize:

- **Formatting:** `rustfmt` with the repository defaults (`cargo fmt`). All code
  MUST be `rustfmt`-clean; CI runs `cargo fmt --all --check`.
- **Linting:** `clippy` with **warnings denied** (`cargo clippy --all-targets --all-features -- -D warnings`).
  Contributions MUST be clippy-clean; CI denies any warning.
- **Language:** code and comments are in **English**; comment the *why*, not the
  *what*.

Because both tools run in CI and are required to pass, compliance is checked on
every change rather than left to reviewer discretion.

## Testing policy

Tests are part of the change, not an afterthought:

- **Major new functionality MUST add tests** to the automated test suite in the
  same change. A feature without tests is not considered complete and will not be
  merged.
- **Bug fixes SHOULD add a regression test** that fails before the fix and passes
  after, so the bug cannot silently return.
- The automated suite runs **on every push and pull request** (CI) and reports
  success/failure; a red suite blocks the merge.
- Retrieval-quality claims are **measured, not asserted** — the benchmark harness
  (`BENCHMARK.md`) reports real `recall@k`/`MRR`, including committed known-miss
  queries, rather than hardcoding a pass.

Statement coverage is measured with [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov)
(`cargo llvm-cov --workspace --summary-only`); see
[`docs/best-practices-silver.md`](docs/best-practices-silver.md) for the current
figure.

## Pull requests

- Keep changes focused; one logical change per PR.
- Update [`CHANGELOG.md`](CHANGELOG.md) under `[Unreleased]` when your change is
  user-visible.
- Code and comments are written in English. Comment the *why*, not the *what*.

### Conventional Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `bench:`, etc. This
keeps the history machine-readable and drives changelog generation.

### Developer Certificate of Origin (DCO)

By contributing, you certify the [DCO](https://developercertificate.org/): that
you wrote the patch or otherwise have the right to submit it under the project's
license. Sign off your commits with `git commit -s`, which appends a
`Signed-off-by:` trailer.

## License of contributions

This project is dual-licensed under **MIT OR Apache-2.0**. Per the Rust
ecosystem convention:

> Unless you explicitly state otherwise, any contribution intentionally
> submitted for inclusion in the work by you, as defined in the Apache-2.0
> license, shall be dual-licensed as above, without any additional terms or
> conditions.
