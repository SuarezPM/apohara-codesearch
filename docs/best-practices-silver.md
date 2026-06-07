# OpenSSF Best Practices — Passing + Silver criteria evidence

Project: **apohara-codesearch** · badge entry [#13118](https://www.bestpractices.dev/projects/13118).

This maps every **Passing** and **Silver** criterion
([bestpractices.dev/en/criteria/0](https://www.bestpractices.dev/en/criteria/0),
[criteria/1](https://www.bestpractices.dev/en/criteria/1)) to its status and the
exact evidence, so both questionnaires can be answered quickly. Status is honest:
**Met**, **N/A** (with justification), **Justified unmet** (a SHOULD/SUGGESTED we
consciously do not meet), or **Human action** (something only the maintainer can
do — completing the web form, holding off-site recovery keys). Silver requires the
**Passing** badge first.

> **Coverage figure** referenced below: **≈89% line coverage** (89.08% lines /
> 86.93% regions), measured with `cargo llvm-cov --workspace --summary-only`
> (default suite; the `gguf-embed` parity test is gated behind a feature + a
> user-supplied `APOHARA_EMBED_MODEL` weights dir and is excluded). Re-run to
> refresh.

> **What makes this project's mapping different:** apohara-codesearch is a
> **local, offline** tool that performs **no network I/O and no cryptographic
> operations as a security mechanism** (blake3 is used only as a non-security
> feature-hash / content-delta). The entire `crypto_*` and TLS family is therefore
> **N/A** with that single justification, stated once here and referenced per-row.

---

## Passing — readiness

The repository satisfies the Passing criteria; completing the form is the only
remaining step. Highlights (the full Silver table below subsumes the rest):

| Criterion | Status | Evidence |
|---|---|---|
| `description_good`, `interact`, `contribution`, `contribution_requirements` | Met | `README.md`, GitHub Issues/PRs, `CONTRIBUTING.md`. |
| `floss_license`, `license_location` | Met | MIT OR Apache-2.0; `LICENSE-MIT` + `LICENSE-APACHE` at repo root. |
| `documentation_basics`, `documentation_interface` | Met | `README.md` (Quick Start, Tools, How it works) + the MCP tool docs. |
| `repo_public`, `repo_track`, `repo_distributed` | Met | Public Git on GitHub; full history; standard Git. |
| `version_unique`, `version_semver`, `version_tags` | Met | SemVer; `vX.Y.Z` tags; `CHANGELOG.md`. |
| `report_tracker`, `report_process`, `report_responses` | Met | GitHub Issues; `CONTRIBUTING.md`; maintainer triage. |
| `vulnerability_report_process`, `vulnerability_report_private` | Met | `SECURITY.md` (private GitHub Security Advisories). |
| `build`, `build_common_tools`, `build_floss_tools` | Met | `cargo build` with the FLOSS Rust toolchain. |
| `test`, `test_invocation`, `test_continuous_integration` | Met | `cargo test --workspace`; CI on every push/PR (`.github/workflows/ci.yml`). |
| `warnings`, `warnings_fixed` | Met | `clippy -D warnings` + `cargo fmt --check` in CI. |
| `static_analysis` | Met | `clippy` + `cargo-audit` + `cargo-deny` + OpenSSF Scorecard. |
| `crypto_*` | N/A | No crypto/network as a security mechanism (see header note). |
| `release_notes` | Met | `CHANGELOG.md` (Keep a Changelog). |
| `installation_common` | Met | `cargo install`, `npx @apohara/codesearch-mcp`, or Release binaries. |

| Criterion | Status | Evidence |
|---|---|---|
| `achieve_passing` (Silver prerequisite) | **Human action** | Complete the Passing questionnaire on bestpractices.dev. The repo satisfies it (FLOSS MIT/Apache, public Git, SemVer tags, build+test CI, `SECURITY.md`, signed releases, static analysis). |

---

## Silver

### Basics
| Criterion | Status | Evidence |
|---|---|---|
| `contribution_requirements` | Met | `CONTRIBUTING.md` — quality gate + coding standards + testing policy + acceptable-contribution requirements. |
| `bus_factor` (SHOULD) | Justified unmet | Single maintainer today; `GOVERNANCE.md` documents continuity and an open invitation to co-maintainers. SHOULD, not MUST. |
| `access_continuity` | Met (+ human follow-through) | `GOVERNANCE.md` § Access continuity: credential custody + off-site break-glass recovery + keyless releases + fork-ability. Human half: keep off-site recovery copies with a trusted party. |
| `roles_responsibilities` | Met | `GOVERNANCE.md` § Roles and responsibilities (table). |
| `code_of_conduct` | Met | `CODE_OF_CONDUCT.md` (Contributor Covenant 3.0). |
| `governance` | Met | `GOVERNANCE.md` § Governance model. |
| `dco` (SHOULD) | Met | `CONTRIBUTING.md` § Developer Certificate of Origin (`git commit -s`). |
| `documentation_roadmap` | Met | `README.md` § Roadmap. |
| `documentation_architecture` | Met | `README.md` § Repository layout + § How it works; `docs/ASSURANCE.md` (trust boundaries). |
| `documentation_security` | Met | `SECURITY.md` (threat model) + `docs/ASSURANCE.md` (assurance case). |
| `documentation_quick_start` | Met | `README.md` § Quick Start. |
| `documentation_current` | Met | Docs are versioned with the code and updated in the same change; `CHANGELOG.md` per release; `cargo doc` is kept warning-free. |
| `documentation_achievements` | Met | `README.md` badge block links the OpenSSF Best Practices badge (#13118). |
| `accessibility_best_practices` (SHOULD) | Met | Plain-Markdown docs (semantic headings, no custom widgets) and a plain-text CLI / stdio interface; no GUI to make inaccessible. |
| `internationalization` (SHOULD) | N/A | The CLI/MCP server emits no localized end-user text and does no human-language-specific sorting. |
| `sites_password_security` | N/A | The project operates no website and stores no user passwords (no auth server). |

### Change Control
| Criterion | Status | Evidence |
|---|---|---|
| `maintenance_or_update` | Met | SemVer + `CHANGELOG.md`; backward-compatible, versioned index migrations (`schema.rs`) keep older indexes upgradable. |

### Reporting
| Criterion | Status | Evidence |
|---|---|---|
| `report_tracker` | Met | GitHub Issues. |
| `vulnerability_response_process` | Met | `SECURITY.md` — private GitHub Security Advisories, 7-day acknowledgement target, coordinated disclosure. |
| `vulnerability_report_credit` | N/A | No vulnerabilities resolved in the last 12 months. |

### Quality
| Criterion | Status | Evidence |
|---|---|---|
| `coding_standards` | Met | `CONTRIBUTING.md` § Coding standards (rustfmt + clippy). |
| `coding_standards_enforced` | Met | CI runs `cargo fmt --check` + `cargo clippy --all-features -D warnings` (`.github/workflows/ci.yml`). |
| `build_repeatable` | Met (justified) | `Cargo.lock` pins every dependency and `rust-toolchain.toml` pins the channel, so a build is deterministic **given an identical toolchain version**. Full bit-for-bit reproducibility across compiler versions is **not** guaranteed (standard for Rust: embedded paths, codegen across patch releases); the channel is rolling `stable`. OpenSSF permits this as a justified partial. |
| `build_non_recursive` | N/A | Cargo workspace; no recursive Make with cross-dependencies. |
| `build_preserve_debug` (SHOULD) | Met | Cargo honors profile debug settings; no stripping of requested debug info. |
| `build_standard_variables` | Met | Cargo honors `RUSTFLAGS`; the bundled SQLite native dep builds via `cc`, which honors `CFLAGS`. |
| `installation_development_quick` | Met | `cargo build` / `cargo test` set up the full dev + test environment (`CONTRIBUTING.md`). |
| `installation_standard_variables` | N/A | Distributed via `cargo install` / prebuilt Release binaries / `npx`; no POSIX `DESTDIR`-style installer. |
| `installation_common` | Met | `cargo install --path crates/apohara-codesearch`, `npx -y @apohara/codesearch-mcp`, or the GitHub Release binaries. |
| `interfaces_current` | Met | Dependencies tracked by `cargo-deny`/`cargo-audit`; no deprecated/obsolete APIs where FLOSS alternatives exist. |
| `external_dependencies` | Met | External dependencies are listed in a computer-processable form: `Cargo.toml` + the fully-resolved `Cargo.lock`; `cargo metadata` emits the complete graph as JSON. |
| `dependency_monitoring` | Met | `cargo-audit` (RUSTSEC) + `cargo-deny advisories` run in CI on every push/PR (`.github/workflows/ci.yml`); Dependabot opens update PRs weekly (`.github/dependabot.yml`); the one reviewed exception (RUSTSEC-2024-0436) is documented in `deny.toml`. |
| `updateable_reused_components` | Met | All reused components are standard crates.io crates pinned in `Cargo.lock`, updatable with `cargo update`; nothing is vendored or forked. |
| `test_statement_coverage80` | **Met** | ≈89% line coverage (89.08% lines / 86.93% regions) measured with `cargo llvm-cov --workspace --summary-only` (reproducible locally; the criterion does not require it as a CI gate). |
| `regression_tests_added50` | Met | Bug fixes ship with regression tests (e.g. the multi-repo registry torn-write fix, the incremental-reindex FTS5 fix `ac7`) added to the suite. |
| `automated_integration_testing` | Met | `cargo test --workspace --all-targets --all-features` runs on every push/PR (`.github/workflows/ci.yml`) and reports pass/fail. |
| `tests_documented_added` | Met | `CONTRIBUTING.md` § Testing policy (new functionality must add tests). |
| `test_policy_mandated` | Met | `CONTRIBUTING.md` § Testing policy (written, mandatory). |
| `warnings_strict` | Met | `clippy -D warnings` (no warning tolerated). |

### Security
| Criterion | Status | Evidence |
|---|---|---|
| `implement_secure_design` | Met | `docs/ASSURANCE.md` § 3 (secure-design principles: no execution of indexed code, fail-soft parsing, least surface, offline by construction, audited narrow `unsafe`). |
| `input_validation` | Met | `docs/ASSURANCE.md` § 4: untrusted file content is parsed error-tolerantly (tree-sitter) and otherwise indexed as bounded text — no parse-by-crash path; MCP `path`/`query` args drive a bounded read rooted at the given path; non-UTF-8 boundaries are respected. |
| `crypto_verification_private`, `crypto_certificate_verification`, `crypto_tls12` (SHOULD), `crypto_used_network` (SHOULD), `crypto_credential_agility`, `crypto_algorithm_agility` (SHOULD), `crypto_weaknesses` | N/A | The tool performs **no cryptographic operations as a security mechanism** and makes **no network/TLS connection** (CI-enforced: `offline-isolation` job). blake3 is used only as a non-security feature-hash / content-delta, not to protect data. |
| `assurance_case` | Met | `docs/ASSURANCE.md` (security requirements + threat model + trust boundaries + secure-design + countered weaknesses + evidence index). |
| `hardening` (SHOULD) | Met | Memory-safe Rust with a small, audited `unsafe` surface (sqlite-vec FFI registration; opt-in safetensors `mmap`) — documented in `docs/ASSURANCE.md` § 3; release profile; CI static analysis. |
| `version_tags_signed` (SUGGESTED) | Justified unmet | Git tags are not GPG-signed, but **release artifacts carry SLSA Build L3 provenance** (Sigstore keyless), verifiable with `gh attestation verify`. Signing tags is a possible future addition. |
| `signed_releases` | Met | Release binaries are signed via SLSA Build L3 provenance (Sigstore keyless — no on-site signing key), generated by `cargo-dist` native attestation; verification documented in `SECURITY.md` + `README.md`. |

### Analysis
| Criterion | Status | Evidence |
|---|---|---|
| `static_analysis_common_vulnerabilities` | Met | `clippy` + `cargo-audit` (RUSTSEC) + `cargo-deny` in CI, plus OpenSSF Scorecard. |
| `dynamic_analysis` (SUGGESTED) | Justified unmet | No continuous fuzzing yet; parsing is error-tolerant and bounded, and pathological-input reports are explicitly welcomed in `SECURITY.md`. A fuzz target is a possible future addition. |
| `dynamic_analysis_unsafe` (SHOULD) | N/A | The produced software is memory-safe Rust; the only `unsafe` is a narrow FFI registration + an opt-in `mmap` (not pointer arithmetic on attacker data), so the memory-safety dynamic-analysis requirement does not apply. |

---

## Summary

Every Silver criterion is **Met** or justifiably **N/A**, except the items that
require a human — (1) completing the **Passing** then **Silver** questionnaires on
bestpractices.dev, and (2) the **off-site custody** half of the access-continuity
plan — and three honestly-documented **SHOULD/SUGGESTED** gaps: `bus_factor`
(single maintainer, continuity documented), `version_tags_signed` (artifacts carry
SLSA provenance instead), and `dynamic_analysis` (no fuzzing yet). No criterion is
marked Met that is not genuinely satisfied.
