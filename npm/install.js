// Postinstall script for @apohara/codesearch-mcp.
//
// Detects the host platform/arch, maps it to the matching dist-built archive on
// the GitHub Release for this package version, downloads it, and extracts the
// `apohara-codesearch` binary into ./bin/.
//
// Zero runtime dependencies: uses only Node built-ins (https, fs, path, os,
// zlib via child `tar`/`Expand-Archive`). The download is guarded behind
// `main()` so `require('./install.js')` does NOT trigger any network access —
// requiring the module is side-effect free (verification + testability).

"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const https = require("https");
const { spawnSync } = require("child_process");

// Owner/repo and tag the dist release was published under. The asset names dist
// produces embed the full git tag (with the leading "v"): see PLATFORMS below.
const REPO = "SuarezPM/apohara-codesearch";
const VERSION = require("./package.json").version; // "0.1.0"
const TAG = `v${VERSION}`; // dist tags releases as v0.1.0
const BIN_DIR = path.join(__dirname, "bin");
// dist's binary basename (matches the [[bin]] name in the Rust crate).
const BIN_BASENAME = process.platform === "win32"
  ? "apohara-codesearch.exe"
  : "apohara-codesearch";

// Map Node's `${platform}-${arch}` to the Rust target triple dist built and the
// archive extension dist uses (unix-archive=.tar.xz, windows-archive=.zip — the
// dist 0.32 defaults). These five entries mirror the five targets in
// [workspace.metadata.dist] in the root Cargo.toml.
const PLATFORMS = {
  "linux-x64": { target: "x86_64-unknown-linux-gnu", ext: ".tar.xz" },
  "linux-arm64": { target: "aarch64-unknown-linux-gnu", ext: ".tar.xz" },
  "darwin-x64": { target: "x86_64-apple-darwin", ext: ".tar.xz" },
  "darwin-arm64": { target: "aarch64-apple-darwin", ext: ".tar.xz" },
  "win32-x64": { target: "x86_64-pc-windows-msvc", ext: ".zip" },
};

function resolvePlatform() {
  const key = `${process.platform}-${process.arch}`;
  const entry = PLATFORMS[key];
  if (!entry) {
    const supported = Object.keys(PLATFORMS).join(", ");
    throw new Error(
      `Unsupported platform "${key}". @apohara/codesearch-mcp ships prebuilt ` +
        `binaries for: ${supported}. Build from source instead: ` +
        `cargo install --path crates/apohara-codesearch ` +
        `(https://github.com/${REPO}).`
    );
  }
  // dist asset shape: <app>-<tag>-<target><ext>, e.g.
  // apohara-codesearch-v0.1.0-x86_64-apple-darwin.tar.xz
  const asset = `apohara-codesearch-${TAG}-${entry.target}${entry.ext}`;
  const url = `https://github.com/${REPO}/releases/download/${TAG}/${asset}`;
  return { asset, url, ext: entry.ext };
}

// Download a URL to a local file, following redirects (GitHub release assets
// 302-redirect to objects.githubusercontent.com).
function download(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > 10) {
      reject(new Error(`Too many redirects fetching ${url}`));
      return;
    }
    const req = https.get(
      url,
      { headers: { "User-Agent": "apohara-codesearch-mcp-installer" } },
      (res) => {
        if (
          res.statusCode &&
          res.statusCode >= 300 &&
          res.statusCode < 400 &&
          res.headers.location
        ) {
          res.resume(); // drain
          const next = new URL(res.headers.location, url).toString();
          resolve(download(next, dest, redirects + 1));
          return;
        }
        if (res.statusCode !== 200) {
          res.resume();
          reject(
            new Error(
              `Download failed (HTTP ${res.statusCode}) for ${url}. ` +
                `Was the v${VERSION} GitHub Release published with all assets?`
            )
          );
          return;
        }
        const out = fs.createWriteStream(dest);
        res.pipe(out);
        out.on("finish", () => out.close(() => resolve()));
        out.on("error", reject);
      }
    );
    req.on("error", reject);
  });
}

// Extract the binary from the downloaded archive into BIN_DIR.
//   - .tar.xz  -> `tar -xJf` (tar present on Linux/macOS and Win10+; the J flag
//                 selects xz; bsdtar/GNU tar both accept it).
//   - .zip     -> `tar -xf` also unpacks zip on Win10+ bsdtar; fall back to
//                 PowerShell Expand-Archive if tar is unavailable.
function extract(archivePath, ext, destDir) {
  if (ext === ".tar.xz") {
    run("tar", ["-xJf", archivePath, "-C", destDir]);
    return;
  }
  // .zip (windows)
  const tarResult = spawnSync("tar", ["-xf", archivePath, "-C", destDir], {
    stdio: "inherit",
  });
  if (tarResult.status === 0) return;
  // Fallback: PowerShell Expand-Archive.
  run("powershell", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    `Expand-Archive -Path '${archivePath}' -DestinationPath '${destDir}' -Force`,
  ]);
}

function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: "inherit" });
  if (r.error) throw r.error;
  if (r.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} exited with code ${r.status}`);
  }
}

// dist archives contain a top-level directory named after the archive stem
// (e.g. apohara-codesearch-v0.1.0-x86_64-apple-darwin/apohara-codesearch).
// Locate the binary wherever it landed and move it to bin/.
function placeBinary(destDir) {
  const direct = path.join(destDir, BIN_BASENAME);
  if (fs.existsSync(direct)) {
    finalizeBinary(direct);
    return;
  }
  for (const entry of fs.readdirSync(destDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const candidate = path.join(destDir, entry.name, BIN_BASENAME);
    if (fs.existsSync(candidate)) {
      const target = path.join(BIN_DIR, BIN_BASENAME);
      fs.copyFileSync(candidate, target);
      finalizeBinary(target);
      return;
    }
  }
  throw new Error(
    `Extracted archive did not contain "${BIN_BASENAME}" under ${destDir}.`
  );
}

function finalizeBinary(binPath) {
  const target = path.join(BIN_DIR, BIN_BASENAME);
  if (path.resolve(binPath) !== path.resolve(target)) {
    fs.copyFileSync(binPath, target);
  }
  if (process.platform !== "win32") {
    fs.chmodSync(target, 0o755);
  }
}

async function main() {
  const { asset, url, ext } = resolvePlatform();
  fs.mkdirSync(BIN_DIR, { recursive: true });

  // If a binary is already present (e.g. re-install, or vendored), skip the
  // network round-trip.
  const existing = path.join(BIN_DIR, BIN_BASENAME);
  if (fs.existsSync(existing)) {
    process.stderr.write(
      `apohara-codesearch-mcp: binary already present at ${existing}, skipping download.\n`
    );
    return;
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "apohara-mcp-"));
  const archivePath = path.join(tmpDir, asset);
  process.stderr.write(`apohara-codesearch-mcp: downloading ${url}\n`);
  await download(url, archivePath);
  process.stderr.write(`apohara-codesearch-mcp: extracting ${asset}\n`);
  extract(archivePath, ext, tmpDir);
  placeBinary(tmpDir);
  // Best-effort cleanup; ignore failures.
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch (_) {
    /* ignore */
  }
  process.stderr.write(
    `apohara-codesearch-mcp: installed ${BIN_BASENAME} into ${BIN_DIR}\n`
  );
}

// Only run the installer when executed directly (`node install.js` /
// npm postinstall). Requiring the module exposes the functions for testing
// WITHOUT triggering any download.
if (require.main === module) {
  main().catch((err) => {
    process.stderr.write(`apohara-codesearch-mcp install failed: ${err.message}\n`);
    process.exit(1);
  });
}

module.exports = { resolvePlatform, PLATFORMS, TAG, VERSION };
