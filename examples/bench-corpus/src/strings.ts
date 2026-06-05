// SPDX-License-Identifier: MIT OR Apache-2.0
//
// String manipulation helpers shared across the TypeScript side of the corpus.

/// Collapse every run of whitespace to a single space and trim the ends.
export function normalizeWhitespace(input: string): string {
  return input.replace(/\s+/g, " ").trim();
}

/// Convert a snake_case or kebab-case identifier into camelCase.
export function toCamelCase(identifier: string): string {
  return identifier
    .split(/[-_]/)
    .map((part, index) =>
      index === 0 ? part : part.charAt(0).toUpperCase() + part.slice(1),
    )
    .join("");
}

/// Truncate `text` to at most `max` characters, appending an ellipsis when the
/// text was actually shortened.
export function truncateWithEllipsis(text: string, max: number): string {
  if (text.length <= max) {
    return text;
  }
  return text.slice(0, Math.max(0, max - 1)) + "…";
}

/// Repeat `unit` until the result reaches `width` characters, used to draw
/// simple separator rules in console output.
export function padToWidth(unit: string, width: number): string {
  if (unit.length === 0) {
    return "";
  }
  let out = "";
  while (out.length < width) {
    out += unit;
  }
  return out.slice(0, width);
}
