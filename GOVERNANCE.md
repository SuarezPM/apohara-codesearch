# Governance

This document describes how **apohara-codesearch** is governed: how decisions are
made, who holds which roles, and how the project continues if the maintainer
becomes unavailable. It is intentionally lightweight and honest about the
project's current size (a single maintainer with outside contributors welcome).

## Governance model

apohara-codesearch follows a **single-maintainer (BDFL-style) model** with
open, consensus-seeking discussion:

- **Proposals and decisions happen in the open.** Features, changes, and bug
  reports are discussed in GitHub [Issues](https://github.com/SuarezPM/apohara-codesearch/issues)
  and [Pull Requests](https://github.com/SuarezPM/apohara-codesearch/pulls). Anyone may
  open an issue or PR.
- **The maintainer is the final decision-maker** on what is merged and released,
  but seeks consensus with contributors and prefers the least-surprising,
  best-justified option. Disagreements are resolved by discussion in the
  relevant issue/PR; the maintainer's decision is final if consensus is not
  reached.
- **Non-negotiable design principles** constrain every decision and changes that
  weaken them are rejected on principle:
  - **Zero-model, zero-network default.** The default build ships no embedding
    model and makes **no network calls at runtime or at build time** — no model
    fetch, no telemetry, no API keys. A real model is only ever an **opt-in,
    user-supplied** build feature; it is never downloaded.
  - **One-binary, one-file state.** No required external service, daemon, or
    database; the only state is a single SQLite file.
  - **Honesty over hype.** Retrieval quality is **measured, not asserted**: the
    benchmark publishes real `recall@k`/`MRR` (including committed known-miss
    queries) rather than hiding the feature-hash vector's limits — see
    [`BENCHMARK.md`](BENCHMARK.md) and the README *How it works / honesty*
    section.

## Roles and responsibilities

| Role | Who | Responsibilities |
|------|-----|------------------|
| **Maintainer** | [@SuarezPM](https://github.com/SuarezPM) (Pablo Suarez) | Reviews and merges changes; cuts releases; triages issues and security reports; owns the crates.io / npm / GitHub credentials; final decision-maker. |
| **Security contact** | the maintainer, via [`SECURITY.md`](SECURITY.md) | Receives and responds to vulnerability reports (private GitHub Security Advisories). |
| **Code of Conduct moderator** | the maintainer, via [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) | Receives and acts on conduct reports. |
| **Contributors** | anyone | Open issues/PRs; contributions are accepted per [`CONTRIBUTING.md`](CONTRIBUTING.md) and dual-licensed MIT OR Apache-2.0. |

There is currently **one maintainer**; the project actively welcomes additional
maintainers. A contributor with a sustained track record of high-quality,
on-principle contributions may be invited by the maintainer to become a
co-maintainer (gaining merge/release rights and credential access under the
continuity plan below).

## Access continuity (bus factor)

The project must be able to continue — create and close issues, accept changes,
and publish releases — within about a week even if the maintainer becomes
unavailable. The continuity plan:

- **Credential custody.** The credentials required to operate the project — the
  GitHub account (and repository admin), the crates.io API token, and the npm
  token for the `@apohara` scope — are stored in the maintainer's password
  manager, **with recovery/break-glass copies kept off-site** so a designated
  trusted party can recover access if the maintainer is incapacitated.
- **No on-site secret is load-bearing for users.** The tool runs **fully
  offline** with no server, no key, and no account: a downstream user keeps
  indexing and searching indefinitely regardless of the project's operational
  state. Release binaries are signed via **keyless** Sigstore attestation (SLSA
  build provenance — no long-lived signing key to lose; see
  [`SECURITY.md`](SECURITY.md) and the README).
- **Reproducible from source.** The repository is the single source of truth;
  anyone with the credentials can rebuild and re-publish from a clean checkout
  (`cargo build --release`, then a `vMAJOR.MINOR.PATCH` tag that drives the
  `cargo-dist` release workflow).
- **Fork-ability.** Under the permissive MIT OR Apache-2.0 license, the community
  can fork and continue the project without the maintainer's involvement if ever
  required.

> Maintainer action (kept current out-of-band): ensure the break-glass recovery
> copies are held by a trusted second party. This is the human half of the bus
> factor and is not something the repository can enforce on its own.

## Releases

Releases follow [Semantic Versioning](https://semver.org); each release is a git
tag (`vMAJOR.MINOR.PATCH`) that triggers the `cargo-dist` publish workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)) — per-OS
prebuilt binaries on a GitHub Release, plus the `@apohara/codesearch-mcp` npm
wrapper. The release **binaries** carry a **SLSA Build L3** provenance
attestation (Sigstore keyless), verifiable with `gh attestation verify`; the git
tags themselves are not GPG-signed. The changes per release are recorded in
[`CHANGELOG.md`](CHANGELOG.md).

## Changing this document

Changes to governance are proposed via pull request and decided by the maintainer
in the open, like any other change.
