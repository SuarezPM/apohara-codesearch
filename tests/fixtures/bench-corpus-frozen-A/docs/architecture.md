# Architecture notes (bench-corpus)

This document is an UNPARSED file (Markdown): the indexer windows it into
fixed-size line chunks and has no symbol/import/export rows for it. It exists so
the benchmark exercises text-only retrieval over prose, not just code symbols.

## Money representation

All monetary values are stored as integer minor units (cents). We never use a
binary floating-point type for money because repeated addition of fractional
cents accumulates rounding error. Totals are computed by summing integer cents
and formatting only at the very end.

## Idempotency and retries

External calls are wrapped in an exponential backoff with jitter so a transient
failure does not cascade into a thundering herd. Each retry carries the same
idempotency key, so a duplicated request that actually succeeded once is not
applied twice.

## Caching policy

Hot lookups go through a least-recently-used cache with a fixed capacity. When
the working set exceeds capacity, the least recently touched entry is evicted.
The cache is intentionally tiny: correctness first, footprint second.

## Rate limiting

Inbound requests pass a token-bucket gate. The bucket refills at a steady rate
and rejects a request when it is empty, smoothing bursts without a hard global
lock.

## Why no learned embeddings

The retrieval layer uses a deterministic feature-hash vector plus a lexical
index, fused by reciprocal rank fusion. There is no neural model and no GPU
requirement; the trade-off is token-distribution similarity instead of deep
semantic understanding.
