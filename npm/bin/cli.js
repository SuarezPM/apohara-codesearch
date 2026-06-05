#!/usr/bin/env node
// Entrypoint for @apohara/codesearch-mcp.
//
// Spawns the prebuilt `apohara-codesearch serve` binary (placed here by
// install.js) as a stdio MCP server.
//
// CRITICAL (Risk R-4): MCP speaks JSON-RPC over stdin/stdout and the framing
// must flow byte-for-byte, unbuffered. We use `stdio: 'inherit'` so the child's
// stdin/stdout/stderr ARE this process's — no piping, no transforms, no line
// buffering. Equally critical: this file must NEVER write to stdout
// (no console.log) — any stray byte on stdout corrupts the JSON-RPC stream.
// Diagnostics, if any, go to stderr only.

"use strict";

const path = require("path");
const fs = require("fs");
const { spawn } = require("child_process");

const binName =
  process.platform === "win32"
    ? "apohara-codesearch.exe"
    : "apohara-codesearch";
const binPath = path.join(__dirname, binName);

if (!fs.existsSync(binPath)) {
  process.stderr.write(
    `apohara-codesearch-mcp: binary not found at ${binPath}. ` +
      `The postinstall step (install.js) may have failed — try reinstalling ` +
      `(npm install @apohara/codesearch-mcp) with network access.\n`
  );
  process.exit(1);
}

// Forward any extra CLI args after the implicit `serve` subcommand.
const args = ["serve", ...process.argv.slice(2)];

const child = spawn(binPath, args, { stdio: "inherit" });

child.on("error", (err) => {
  process.stderr.write(`apohara-codesearch-mcp: failed to start binary: ${err.message}\n`);
  process.exit(1);
});

// Pass through the child's exit status. On signal termination, mirror the
// conventional 128+signal exit code so callers can detect it.
child.on("exit", (code, signal) => {
  if (signal) {
    process.exit(128 + (require("os").constants.signals[signal] || 0));
  }
  process.exit(code === null ? 1 : code);
});

// Relay termination signals to the child so the MCP host can shut it down
// cleanly (SIGINT/SIGTERM); the child's exit handler above then propagates.
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    if (!child.killed) {
      child.kill(sig);
    }
  });
}
