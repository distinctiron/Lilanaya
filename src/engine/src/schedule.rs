//! Pairing schedules.

/// Every unordered pair `(i, j)` with `i < j` over `n` actors, in a stable
/// order: (0,1), (0,2), …, (1,2), …. The caller sorts actors by id first, so
/// the schedule does not depend on the order actors were supplied in.
pub fn all_pairs(n: usize) -> Vec<(usize, usize)> {
    (0..n).flat_map(|i| (i + 1..n).map(move |j| (i, j))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_unordered_pairs_in_stable_order() {
        assert_eq!(all_pairs(0), vec![]);
        assert_eq!(all_pairs(1), vec![]);
        assert_eq!(all_pairs(4), vec![(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]);
        assert_eq!(all_pairs(10).len(), 45);
    }
}
