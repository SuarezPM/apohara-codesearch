#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# fetch-csn.sh — DELIBERATE, user-run download of the CodeSearchNet slices the
# `bench-codesearchnet` example reads. Run it by hand, ONCE, outside any build:
#
#     bash scripts/fetch-csn.sh /home/you/csn
#
# CRITICAL CONTRACT (N3): this script is NEVER invoked by any cargo target — not
# by build.rs (there is none), not by a [[example]] entry, not by any test. It is
# a standalone shell script, so no `cargo build`/`run`/`test` can trigger a
# network fetch. The benchmark reads the produced JSONL files ONLY from local
# disk via APOHARA_CSN_ROOT, preserving the "zero network in any cargo command"
# honesty contract that BENCHMARK.md commits to.
#
# ─────────────────────────────────────────────────────────────────────────────
# DATASET — version, split, and provenance (reproducibility, BENCHMARK.md):
#
#   Dataset:  CodeSearchNet corpus (Husain et al. 2019, arXiv:1909.09436).
#   Source:   the Hugging Face mirror, which is the maintained distribution today
#             (the original github/CodeSearchNet S3 bucket now returns 403):
#               https://huggingface.co/datasets/code-search-net/code_search_net
#             The modern HF layout serves one Parquet file per language+split at
#               <lang>/<split>-00000-of-00001.parquet
#             with fields `func_documentation_string` (the NL query) and
#             `func_code_string` (the body). This script downloads the Parquet,
#             converts it to the JSONL the benchmark expects, and renames those
#             two fields to `docstring`/`code` so the bench loader (which reads
#             {docstring, code}) is unchanged.
#   Split:    TEST split only (the held-out evaluation split — train/valid are
#             NOT used, so numbers are not measured on data reserved for fitting).
#   Languages: python, go, javascript. JavaScript is materialized as the
#             `typescript` slice (the `.ts` extension) because CodeSearchNet has
#             NO TypeScript split and TS/JS share a parser family — the closest
#             public NL→code proxy for our first-class TypeScript support.
#
# REQUIREMENTS: `curl` and `uv` (https://docs.astral.sh/uv/). Parquet needs a
#   reader; rather than mandate a system pyarrow, we use `uv run --with pyarrow`,
#   which provisions it in an EPHEMERAL environment — nothing is installed
#   permanently and nothing leaks into any cargo build.
#
# OUTPUT — exactly what the example expects under $CSN_ROOT:
#   $CSN_ROOT/python.jsonl
#   $CSN_ROOT/go.jsonl
#   $CSN_ROOT/typescript.jsonl   (from the javascript test split)
#
# Each output is one JSON record per line: {"docstring": ..., "code": ...}.
#
# CHECKSUM — after this finishes, compute and record the sha256 of each file in
# BENCHMARK.md so a second machine can verify byte-identical inputs:
#       sha256sum "$CSN_ROOT"/{python,go,typescript}.jsonl
#   The sums are NOT hard-coded here on purpose: upstream can re-publish, and a
#   stale baked-in checksum that silently disagrees is worse than one you
#   recompute and commit deliberately.
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

CSN_ROOT="${1:-${APOHARA_CSN_ROOT:-}}"
if [[ -z "${CSN_ROOT}" ]]; then
	echo "usage: bash scripts/fetch-csn.sh <dest-dir>" >&2
	echo "       (or set APOHARA_CSN_ROOT and run with no argument)" >&2
	exit 2
fi

for tool in curl uv; do
	if ! command -v "${tool}" >/dev/null 2>&1; then
		echo "error: required tool '${tool}' not found on PATH" >&2
		exit 1
	fi
done

mkdir -p "${CSN_ROOT}"

BASE_URL="https://huggingface.co/datasets/code-search-net/code_search_net/resolve/main"
SPLIT="test"

# Map a CodeSearchNet language to the output filename the bench reads. The
# javascript split is written as typescript.jsonl (see header — JS is the TS
# proxy because CSN has no TypeScript split).
declare -A OUTPUT_NAME=(
	[python]="python.jsonl"
	[go]="go.jsonl"
	[javascript]="typescript.jsonl"
)

for lang in python go javascript; do
	out="${CSN_ROOT}/${OUTPUT_NAME[$lang]}"
	echo ">> fetching CodeSearchNet ${lang} ${SPLIT} split -> ${out}" >&2

	# The HF test split is a single Parquet shard per language.
	url="${BASE_URL}/${lang}/${SPLIT}-00000-of-00001.parquet"
	tmp_parquet="$(mktemp --suffix=.parquet)"
	if ! curl -fsSL "${url}" -o "${tmp_parquet}"; then
		echo "error: failed to fetch ${url}" >&2
		echo "       (check the dataset URL in this script's header — upstream may have moved it)" >&2
		rm -f "${tmp_parquet}"
		exit 1
	fi

	# Convert Parquet -> JSONL, renaming the HF fields to the {docstring, code}
	# shape the bench loader reads. pyarrow runs in an ephemeral uv environment.
	PARQUET_IN="${tmp_parquet}" JSONL_OUT="${out}" uv run --quiet --with pyarrow python3 - <<'PY'
import json
import os
import pyarrow.parquet as pq

src = os.environ["PARQUET_IN"]
dst = os.environ["JSONL_OUT"]
table = pq.read_table(src, columns=["func_documentation_string", "func_code_string"])
docs = table.column("func_documentation_string").to_pylist()
codes = table.column("func_code_string").to_pylist()
written = 0
with open(dst, "w") as fh:
    for doc, code in zip(docs, codes):
        # The bench itself skips empty docstring/code, but drop them here too so
        # the record count and checksum reflect only usable rows.
        if doc and code:
            fh.write(json.dumps({"docstring": doc, "code": code}) + "\n")
            written += 1
print(f"   wrote {written} records to {dst}")
PY
	rm -f "${tmp_parquet}"
done

echo >&2
echo "done. now compute and record the checksums in BENCHMARK.md:" >&2
echo "  sha256sum ${CSN_ROOT}/{python,go,typescript}.jsonl" >&2
echo >&2
echo "then run the benchmark:" >&2
echo "  APOHARA_CSN_ROOT=${CSN_ROOT} cargo run --release --example bench-codesearchnet" >&2
