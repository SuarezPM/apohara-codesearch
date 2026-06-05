// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A small validation module: real imports and named exports so the indexer
// records structural rows (file_imports / file_exports) for this TypeScript
// file, plus a couple of parsed functions.

import { circleArea } from "./helpers";

/**
 * Validate that an email string has a plausible shape.
 */
export function validateEmail(email: string): boolean {
  const at = email.indexOf("@");
  return at > 0 && email.indexOf(".", at) > at;
}

/**
 * Validate that a radius is a positive, finite number before measuring it.
 */
export function validateRadius(radius: number): number {
  if (!Number.isFinite(radius) || radius <= 0) {
    return 0;
  }
  return circleArea(radius);
}

export { validateEmail as isEmail };
