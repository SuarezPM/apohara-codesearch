# Security Policy

`apohara-codesearch` is a **local, offline** code-search tool: a single Rust
binary that indexes and searches source code on the user's own machine and
speaks the Model Context Protocol over local stdio. It performs **no network
I/O during indexing or querying**, downloads no model, and uses no external
service. The security posture below is shaped by that niche.

## Supported Versions

The project is pre-1.0; only the current `0.x` line receives security fixes.
There is no long-term-support branch — upgrade to the latest release.

| Version | Supported          |
| ------- | ------------------ |
| 0.x     | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub's built-in
[Private Vulnerability Reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability):
go to the repository's **Security** tab and choose **"Report a vulnerability"**.
This opens a private advisory visible only to the maintainer, where details,
a fix, and a coordinated disclosure can be handled before anything is made
public.

Include, where possible: affected version, a minimal reproduction (e.g. a
crafted input file or query), the observed vs. expected behavior, and any
relevant environment details.

### Response targets

This is an open-source project maintained by a single person; the targets
below are best-effort, not a contractual guarantee.

| Stage                       | Target                  |
| --------------------------- | ----------------------- |
| Acknowledgement of report   | within **7 days**       |
| Initial assessment / triage | within **30 days**      |
| Fix or mitigation plan      | communicated after triage, coordinated in the advisory |

Once a fix is available, the advisory is published and credit is given to the
reporter unless anonymity is requested.

## Threat Model

The relevant trust boundary is narrow and specific to this tool. State it
plainly so users can reason about their own exposure.

### Trust assumptions

- **The user trusts their own machine.** The tool runs locally with the
  user's privileges; it is not a sandbox and does not add a privilege
  boundary the OS does not already provide.
- **The indexed source code is treated as untrusted data.** A repository may
  contain hostile, malformed, or adversarially crafted files (this is the
  whole point of a code-search tool — you point it at arbitrary code).
- **The MCP client is trusted.** The server is driven over local stdio by a
  coding agent the user has already chosen to run.

### In scope

- **Parsing untrusted source files.** Files are read and parsed with
  tree-sitter grammars and chunked. A crafted file should not crash the
  process unrecoverably, hang it, exhaust memory, or corrupt the on-disk
  index. Parser panics, pathological inputs that blow up memory or time, and
  index-corruption-on-malformed-input are valid reports.
- **Path and symlink handling.** The walker traverses a directory tree the
  user pointed it at. Path traversal that escapes the indexed root, or
  symlink following that causes reads outside the intended tree, are valid
  reports.
- **Local index integrity.** The single SQLite index file is the only state.
  Inputs that can silently corrupt it, or a migration that loses or mangles
  data, are valid reports.
- **The MCP surface (local stdio).** Malformed JSON-RPC, or tool arguments
  (`path`, `query`) that trigger a crash, an unbounded operation, or a read
  outside the requested path, are valid reports.

### Explicitly out of scope (by design)

- **No execution of indexed code.** The tool **never compiles, evaluates, or
  runs** the source it indexes — it only reads and parses it as text/AST.
  "The repo I indexed contained malware" is not a vulnerability in this tool:
  indexing does not execute anything from the target repository.
- **No network attack surface at index/query time.** The binary makes no
  outbound connections and listens on no socket during indexing or querying;
  there is no remote attacker in the normal operating model. (The only
  network touch points are the *separate* `npx`/Release download path used to
  acquire the binary — covered by the SLSA provenance attestation on each
  release artifact, see the README — and never the running tool itself.)
- **A malicious or compromised local machine / MCP client.** If the host or
  the agent driving the server is already compromised, that is outside the
  trust boundary; this tool does not defend against an attacker who already
  has local code execution.
- **Resource use on a trusted very large repo.** Expected memory/time growth
  on legitimately huge inputs is a performance matter, not a security issue —
  unless a *small crafted* input forces disproportionate resource use (that
  is in scope, above).

If you are unsure whether something fits the model above, report it privately
anyway and let the triage decide.
