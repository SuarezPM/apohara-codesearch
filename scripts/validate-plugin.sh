#!/usr/bin/env bash
# Validate the Claude Code plugin + marketplace manifests against the required
# fields confirmed from the current docs (code.claude.com, fetched 2026-06-05):
#   - plugin.json:      `name` is the only required field; if `author` is present
#                       it must be an object with a `name`.
#   - marketplace.json: requires `name`, `owner` (object with `name`), and a
#                       `plugins` array; each plugin entry requires `name` and `source`.
# Exit 0 on success; non-zero with a message on the first failure.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGIN="$ROOT/.claude-plugin/plugin.json"
MARKET="$ROOT/marketplace.json"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

command -v python3 >/dev/null 2>&1 || fail "python3 is required to validate JSON"

test -f "$PLUGIN" || fail "missing $PLUGIN"
test -f "$MARKET" || fail "missing $MARKET"

python3 - "$PLUGIN" "$MARKET" <<'PY'
import json
import sys

plugin_path, market_path = sys.argv[1], sys.argv[2]


def die(msg):
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def load(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError) as exc:
        die(f"{path} is not well-formed JSON: {exc}")


# --- plugin.json -----------------------------------------------------------
plugin = load(plugin_path)
if not isinstance(plugin, dict):
    die("plugin.json must be a JSON object")
name = plugin.get("name")
if not isinstance(name, str) or not name.strip():
    die("plugin.json: required field 'name' must be a non-empty string")
if name != name.lower() or " " in name:
    die("plugin.json: 'name' must be kebab-case with no spaces")
if "author" in plugin:
    author = plugin["author"]
    if not isinstance(author, dict) or not isinstance(author.get("name"), str):
        die("plugin.json: 'author' must be an object with a string 'name'")
if "keywords" in plugin and not isinstance(plugin["keywords"], list):
    die("plugin.json: 'keywords' must be an array")

# --- marketplace.json ------------------------------------------------------
market = load(market_path)
if not isinstance(market, dict):
    die("marketplace.json must be a JSON object")
mname = market.get("name")
if not isinstance(mname, str) or not mname.strip():
    die("marketplace.json: required field 'name' must be a non-empty string")
if mname != mname.lower() or " " in mname:
    die("marketplace.json: 'name' must be kebab-case with no spaces")

owner = market.get("owner")
if not isinstance(owner, dict) or not isinstance(owner.get("name"), str):
    die("marketplace.json: required field 'owner' must be an object with a string 'name'")

plugins = market.get("plugins")
if not isinstance(plugins, list) or not plugins:
    die("marketplace.json: required field 'plugins' must be a non-empty array")

for i, entry in enumerate(plugins):
    if not isinstance(entry, dict):
        die(f"marketplace.json: plugins[{i}] must be an object")
    if not isinstance(entry.get("name"), str) or not entry["name"].strip():
        die(f"marketplace.json: plugins[{i}] requires a non-empty string 'name'")
    source = entry.get("source")
    if isinstance(source, str):
        if not source.startswith("./"):
            die(f"marketplace.json: plugins[{i}] string 'source' must start with './'")
    elif isinstance(source, dict):
        if not isinstance(source.get("source"), str):
            die(f"marketplace.json: plugins[{i}] object 'source' requires a 'source' type string")
    else:
        die(f"marketplace.json: plugins[{i}] requires a string or object 'source'")

print("OK: plugin.json and marketplace.json have valid shape and required fields")
PY
