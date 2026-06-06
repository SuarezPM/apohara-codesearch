// SPDX-License-Identifier: MIT OR Apache-2.0

//! apohara-codesearch: stdio MCP server exposing two tools (`search_code`,
//! `reindex`) backed by the `apohara-indexer` engine.
//!
//! Subcommands:
//!   serve             Run the stdio MCP server (JSON-RPC over stdin/stdout).
//!   watch <path>      Watch a repo and incrementally reindex on change (CLI loop).
//!   --sqlite-version  Print the bundled SQLite version and exit.

mod dto;
mod server;
mod watch;

/// Process-global lock serializing tests that mutate the shared `XDG_CONFIG_HOME`
/// (the sidecar registry's config dir). Both `watch::tests` and `server::tests`
/// drive the registry, so they MUST take this ONE lock to avoid clobbering each
/// other's env override when run in parallel.
#[cfg(test)]
pub(crate) fn test_env_guard() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

use std::path::PathBuf;

use anyhow::Result;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

use crate::server::CodeSearchServer;

fn main() -> Result<()> {
    // Skip argv[0]; match on the first real argument.
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("serve") => serve(),
        Some("watch") => {
            // `watch <path>` requires a path argument; default to "." otherwise.
            let path = std::env::args().nth(2).unwrap_or_else(|| ".".to_string());
            watch::run_watch(&PathBuf::from(path))
        }
        Some("--sqlite-version") => {
            // Goes to STDOUT: this is the program's sole output in this mode, so
            // tooling can capture it directly. (The JSON-RPC stdout rule only
            // applies to `serve`.)
            println!("{}", apohara_indexer::sqlite_version());
            Ok(())
        }
        other => {
            // Usage to STDERR; non-zero exit so callers notice the misuse.
            eprintln!(
                "apohara-codesearch: unknown command {:?}\n\
                 usage:\n  apohara-codesearch serve\n  \
                 apohara-codesearch watch <path>\n  \
                 apohara-codesearch --sqlite-version",
                other.unwrap_or("<none>")
            );
            std::process::exit(2);
        }
    }
}

/// Start the stdio MCP server on a current-thread tokio runtime.
///
/// A current-thread runtime keeps the resident memory footprint minimal; the
/// multi-thread scheduler is deliberately avoided.
fn serve() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        // rmcp's service loop arms Tokio timers internally; without the time
        // driver it panics on first non-init message.
        .enable_time()
        .build()?;

    runtime.block_on(async {
        // All diagnostics go to STDERR; STDOUT carries only the JSON-RPC stream
        // and must never be polluted.
        eprintln!("apohara-codesearch: starting stdio MCP server");

        let service = CodeSearchServer::new().serve(stdio()).await?;
        // Block until the peer disconnects (stdin EOF) or the stream closes.
        service.waiting().await?;
        eprintln!("apohara-codesearch: server stopped");
        Ok::<(), anyhow::Error>(())
    })
}
