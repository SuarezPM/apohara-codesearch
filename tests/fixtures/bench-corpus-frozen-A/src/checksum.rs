// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Checksums and hashing helpers. Distinct "crc"/"fletcher"/"rolling" vocabulary
// so an integrity-check query lands here, not on billing or geometry.

/// Fletcher-16 checksum over a byte slice. Two running 8-bit sums combined into
/// a 16-bit result; cheap and catches most single-byte corruption.
pub fn fletcher16(data: &[u8]) -> u16 {
    let mut sum1: u16 = 0;
    let mut sum2: u16 = 0;
    for &byte in data {
        sum1 = (sum1 + byte as u16) % 255;
        sum2 = (sum2 + sum1) % 255;
    }
    (sum2 << 8) | sum1
}

/// A simple additive rolling hash over a window, supporting cheap slide updates
/// without rehashing the whole window. Useful for substring pre-filtering.
pub struct RollingHash {
    pub value: u64,
    pub base: u64,
    pub window: usize,
}

impl RollingHash {
    /// Seed a rolling hash from an initial window of bytes.
    pub fn seed(bytes: &[u8], base: u64) -> RollingHash {
        let mut value = 0u64;
        for &b in bytes {
            value = value.wrapping_mul(base).wrapping_add(b as u64);
        }
        RollingHash {
            value,
            base,
            window: bytes.len(),
        }
    }
}

/// CRC-32 (IEEE) over a byte slice, computed bit-by-bit. Not table-optimized —
/// clarity over speed for this small corpus.
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
