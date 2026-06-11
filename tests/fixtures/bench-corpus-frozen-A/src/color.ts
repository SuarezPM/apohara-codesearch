// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Color conversions. The hex parser is named `decodeHexColor` and works in RGB
// channels; a natural-language query about "convert #rrggbb to red green blue"
// is a partial-miss because the words "red"/"green"/"blue" never appear here.

/// An RGB color with 8-bit channels.
export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/// Parse a `#rrggbb` hex string into an Rgb triple. Throws on malformed input.
export function decodeHexColor(hex: string): Rgb {
  const cleaned = hex.startsWith("#") ? hex.slice(1) : hex;
  if (cleaned.length !== 6) {
    throw new Error("hex color must be 6 digits");
  }
  return {
    r: parseInt(cleaned.slice(0, 2), 16),
    g: parseInt(cleaned.slice(2, 4), 16),
    b: parseInt(cleaned.slice(4, 6), 16),
  };
}

/// Render an Rgb triple back into a `#rrggbb` hex string.
export function encodeHexColor(color: Rgb): string {
  const part = (n: number) => n.toString(16).padStart(2, "0");
  return `#${part(color.r)}${part(color.g)}${part(color.b)}`;
}

/// Relative luminance of a color in the 0..1 range, using the Rec. 601 weights.
export function relativeLuminance(color: Rgb): number {
  return (0.299 * color.r + 0.587 * color.g + 0.114 * color.b) / 255;
}
