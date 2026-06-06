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
#   Dataset:  CodeSearchNet corpus (the dataset published with the CodeSearchNet
#             Challenge, Husain et al. 2019, arXiv:1909.09436).
#   Source:   the official S3 mirror referenced by github/CodeSearchNet,
#             https://github.com/github/CodeSearchNet#data
#             base URL: https://huggingface.co/datasets/code-search-net/code_search_net
#             (the per-language `.jsonl.gz` shards under the canonical layout
#             `<lang>/final/jsonl/{train,valid,test}/<lang>_<split>_<n>.jsonl.gz`).
#   Split:    we use the TEST split only (the held-out evaluation split — the
#             honest retrieval slice; train/valid are NOT used so the numbers are
#             not measured on data the dataset itself reserves for fitting).
#   Languages: python, go, javascript. JavaScript is materialized as the
#             `typescript` slice (the `.ts` extension) because CodeSearchNet has
#             NO TypeScript split and TS/JS share a parser family — the closest
#             public NL→code proxy for our first-class TypeScript support.
#
# OUTPUT — exactly what the example expects under $CSN_ROOT:
#   $CSN_ROOT/python.jsonl
#   $CSN_ROOT/go.jsonl
#   $CSN_ROOT/typescript.jsonl   (from the javascript test split)
#
# Each output is the concatenation of the language's TEST-split shards,
# decompressed, one JSON record per line (the native CodeSearchNet JSONL schema:
# {repo, path, func_name, original_string, language, code, code_tokens,
#  docstring, docstring_tokens, url, partition}).
#
# CHECKSUM — how to obtain and record it (do NOT trust a number you did not
# compute yourself):
#
#   After this script finishes, compute the sha256 of each produced file:
#
#       sha256sum "$CSN_ROOT"/{python,go,typescript}.jsonl
#
#   Paste those three sums into BENCHMARK.md's CodeSearchNet section so a second
#   machine can verify it fetched byte-identical inputs. The sums are NOT
#   hard-coded here on purpose: the upstream shards can be re-published, and a
#   stale baked-in checksum that silently disagrees is worse than one you
#   recompute and commit deliberately. The dataset VERSION (test split of the
#   CodeSearchNet corpus at the URL above) is the reproducibility anchor; the
#   sha256 you record pins the exact bytes you measured.
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

CSN_ROOT="${1:-${APOHARA_CSN_ROOT:-}}"
if [[ -z "${CSN_ROOT}" ]]; then
	echo "usage: bash scripts/fetch-csn.sh <dest-dir>" >&2
	echo "       (or set APOHARA_CSN_ROOT and run with no argument)" >&2
	exit 2
fi

for tool in curl gzip; do
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
	: >"${out}"

	# Each language's test split is sharded as
	#   <lang>/final/jsonl/test/<lang>_test_<n>.jsonl.gz
	# The test split is small (a single shard, index 0) for every language in the
	# CodeSearchNet corpus; we fetch shard 0 and append. If upstream re-shards the
	# test split, extend this loop's range accordingly.
	shard=0
	url="${BASE_URL}/${lang}/final/jsonl/${SPLIT}/${lang}_${SPLIT}_${shard}.jsonl.gz"
	tmp_gz="$(mktemp)"
	if ! curl -fsSL "${url}" -o "${tmp_gz}"; then
		echo "error: failed to fetch ${url}" >&2
		echo "       (check the dataset URL in this script's header — upstream may have moved it)" >&2
		rm -f "${tmp_gz}"
		exit 1
	fi
	gzip -dc "${tmp_gz}" >>"${out}"
	rm -f "${tmp_gz}"

	lines="$(wc -l <"${out}")"
	echo "   wrote ${lines} records to ${out}" >&2
done

echo >&2
echo "done. now compute and record the checksums in BENCHMARK.md:" >&2
echo "  sha256sum ${CSN_ROOT}/{python,go,typescript}.jsonl" >&2
echo >&2
echo "then run the benchmark:" >&2
echo "  APOHARA_CSN_ROOT=${CSN_ROOT} cargo run --release --example bench-codesearchnet" >&2
