// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Session/token bookkeeping. The function that checks whether a session is
// still valid is named `isFresh` and its body talks about "expiry" and
// "elapsed" — NOT "login" or "authenticate" — so a natural-language query about
// "is the user still logged in" is a known-miss for lexical search.

import { clampNumber } from "./validation";

/// A bearer session: an opaque token plus the epoch-second it was issued.
export interface Session {
  token: string;
  issuedAtEpoch: number;
}

/// Build a session stamped at the given issue time.
export function openSession(token: string, issuedAtEpoch: number): Session {
  return { token, issuedAtEpoch };
}

/// True when the session has not yet exceeded its time-to-live. The check is
/// purely arithmetic over elapsed seconds versus the ttl.
export function isFresh(session: Session, nowEpoch: number, ttlSeconds: number): boolean {
  const elapsed = nowEpoch - session.issuedAtEpoch;
  return elapsed >= 0 && elapsed <= ttlSeconds;
}

/// Remaining lifetime of a session in seconds, never negative.
export function secondsRemaining(session: Session, nowEpoch: number, ttlSeconds: number): number {
  const elapsed = nowEpoch - session.issuedAtEpoch;
  return clampNumber(ttlSeconds - elapsed, 0, ttlSeconds);
}
