//! Realistic corpora for the containment measurements: a generated lcov
//! report and a multi-language source tree. Both are built from a fixed
//! seed by a small generator, so a run is repeatable and no megabytes are
//! checked in.

// Each test binary uses the part of the corpus it measures.
#![allow(dead_code)]

pub mod lcov;
pub mod tree;

/// A small deterministic generator (splitmix64): the same seed gives the
/// same corpus on every machine and every run.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `lo..=hi`.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as usize
    }

    /// One of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, items.len() - 1)]
    }

    /// True about `percent` times in a hundred.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.range(0, 99) < percent
    }
}
