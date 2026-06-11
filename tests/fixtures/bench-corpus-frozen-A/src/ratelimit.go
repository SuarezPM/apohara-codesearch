// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A token-bucket rate limiter in Go. Distinct "token bucket"/"refill"
// vocabulary so a throttling query lands here.

package corpus

// TokenBucket is a classic token-bucket rate limiter: it holds up to `capacity`
// tokens and refills at `refillPerSecond` tokens per second. A request is
// allowed only when at least one token is available.
type TokenBucket struct {
	capacity        int64
	tokens          int64
	refillPerSecond int64
}

// NewTokenBucket builds a full bucket with the given capacity and refill rate.
func NewTokenBucket(capacity, refillPerSecond int64) *TokenBucket {
	return &TokenBucket{
		capacity:        capacity,
		tokens:          capacity,
		refillPerSecond: refillPerSecond,
	}
}

// Refill adds tokens for the elapsed whole seconds, never exceeding capacity.
func (b *TokenBucket) Refill(elapsedSeconds int64) {
	b.tokens += elapsedSeconds * b.refillPerSecond
	if b.tokens > b.capacity {
		b.tokens = b.capacity
	}
}

// Allow consumes a single token if one is available, reporting whether the
// request may proceed.
func (b *TokenBucket) Allow() bool {
	if b.tokens <= 0 {
		return false
	}
	b.tokens--
	return true
}

// Throttle is a convenience interface for anything that can gate a request.
type Throttle interface {
	Allow() bool
}
