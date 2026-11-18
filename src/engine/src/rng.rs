//! Seeded pseudorandom numbers. One run seed; everything random derives from it.

/// SplitMix64: tiny, fast, and identical on every platform.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// Uniform integer in `0..n` (rejection sampling, so there is no modulo bias).
    /// Panics if `n` is zero.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0)");
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let x = self.next_u64();
            if x < zone {
                return x % n;
            }
        }
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Derives an independent sub-seed from the run seed and a list of labels
/// (for a pairing: the two actor ids). It depends only on its inputs, never on
/// the order pairings happen to be executed in.
pub fn derive_seed(seed: u64, labels: &[&str]) -> u64 {
    let mut h = mix(seed ^ 0xCBF2_9CE4_8422_2325);
    for label in labels {
        for &byte in label.as_bytes() {
            h = (h ^ byte as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
        // Separator, so ["ab", "c"] and ["a", "bc"] differ.
        h = mix(h ^ 0xFF);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn known_values() {
        // Reference outputs of SplitMix64 for seed 0.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn below_stays_in_range_and_shuffle_permutes() {
        let mut r = Rng::new(7);
        for _ in 0..1000 {
            assert!(r.below(10) < 10);
        }
        let mut v: Vec<u32> = (0..20).collect();
        r.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert_ne!(v, sorted);
    }

    #[test]
    fn derived_seeds_differ_by_label_and_seed() {
        let s = derive_seed(42, &["a", "b"]);
        assert_eq!(s, derive_seed(42, &["a", "b"]));
        assert_ne!(s, derive_seed(42, &["b", "a"]));
        assert_ne!(s, derive_seed(43, &["a", "b"]));
        assert_ne!(derive_seed(1, &["ab", "c"]), derive_seed(1, &["a", "bc"]));
    }
}
