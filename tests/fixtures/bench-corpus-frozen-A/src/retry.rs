// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Retry / backoff helpers. Vocabulary here ("backoff", "jitter", "attempt") is
// deliberately distinct from the rest of the corpus so a query about transient
// failure handling lands here.

/// Compute the exponential backoff delay in milliseconds for a given attempt,
/// capped at `max_ms`. Attempt 0 yields `base_ms`; each later attempt doubles
/// until the cap is reached.
pub fn exponential_backoff_ms(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let factor = 1u64 << attempt.min(20);
    (base_ms.saturating_mul(factor)).min(max_ms)
}

/// Add deterministic pseudo-jitter to a delay so a thundering herd of clients
/// does not retry in lockstep. The jitter is derived from `seed` so the result
/// is reproducible in tests (no RNG).
pub fn jittered_delay_ms(delay_ms: u64, seed: u64) -> u64 {
    let spread = delay_ms / 4 + 1;
    let offset = seed % spread;
    delay_ms.saturating_sub(spread / 2).saturating_add(offset)
}

/// Decide whether another attempt should be made given the attempt count and a
/// maximum. Returns true while there are attempts left to spend.
pub fn should_retry(attempt: u32, max_attempts: u32) -> bool {
    attempt + 1 < max_attempts
}

/// The outcome of a retry budget: how many attempts were spent and whether the
/// operation eventually succeeded.
pub struct RetryOutcome {
    pub attempts_spent: u32,
    pub succeeded: bool,
}

impl RetryOutcome {
    /// A successful outcome that took `attempts` tries.
    pub fn success(attempts: u32) -> RetryOutcome {
        RetryOutcome {
            attempts_spent: attempts,
            succeeded: true,
        }
    }
}
