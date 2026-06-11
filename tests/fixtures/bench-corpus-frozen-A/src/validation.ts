// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Input validation helpers. TypeScript so the indexer records structural
// imports/exports and per-symbol signatures.

import { normalizeWhitespace } from "./strings";

/// Result of validating one field: ok flag plus an optional message.
export interface ValidationResult {
  ok: boolean;
  message: string;
}

/// Validate that an email address has the minimal `local@domain.tld` shape.
/// Intentionally permissive: it rejects obvious garbage, not every RFC edge.
export function validateEmail(input: string): ValidationResult {
  const trimmed = normalizeWhitespace(input);
  const at = trimmed.indexOf("@");
  const dot = trimmed.lastIndexOf(".");
  if (at <= 0 || dot < at + 2 || dot === trimmed.length - 1) {
    return { ok: false, message: "malformed email address" };
  }
  return { ok: true, message: "" };
}

/// Validate that a password meets a minimum length and contains at least one
/// digit. Returns a descriptive message on failure.
export function validatePasswordStrength(password: string): ValidationResult {
  if (password.length < 8) {
    return { ok: false, message: "password too short" };
  }
  if (!/[0-9]/.test(password)) {
    return { ok: false, message: "password needs a digit" };
  }
  return { ok: true, message: "" };
}

/// Clamp a number into the inclusive `[lo, hi]` range.
export function clampNumber(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}

/// A reusable range validator built from a low/high bound.
export class RangeValidator {
  constructor(private lo: number, private hi: number) {}

  /// True when `value` lies within the configured inclusive bounds.
  contains(value: number): boolean {
    return value >= this.lo && value <= this.hi;
  }
}
