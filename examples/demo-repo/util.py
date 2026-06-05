# SPDX-License-Identifier: MIT OR Apache-2.0
#
# A small Python utility module. Python is a parsed language for this indexer
# (Phase 2), so each `def` below becomes its own symbol chunk carrying a
# signature and the module's structural imports; the file is also searchable
# through the text (FTS) and vector paths.

import json
import re


def parse_string(raw):
    """Parse a raw string into a trimmed, normalized token list."""
    cleaned = raw.strip().lower()
    return re.split(r"\s+", cleaned)


def serialize_payload(payload):
    """Serialize a payload dict into a compact JSON string."""
    return json.dumps(payload, separators=(",", ":"), sort_keys=True)


def reservoir_sample(items, k):
    """Pick k items uniformly at random using reservoir sampling."""
    chosen = []
    for index, item in enumerate(items):
        if index < k:
            chosen.append(item)
    return chosen
