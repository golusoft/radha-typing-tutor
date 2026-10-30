//! A tiny deterministic PRNG (xorshift64*), so every generated passage is
//! reproducible from its seed: the same sources always give the same bytes.

/// xorshift64* state.
pub struct Rng(u64);

impl Rng {
    /// Seeded generator (a zero seed is remapped; xorshift cannot leave 0).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish index below `n` (`0` when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % n as u64).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<u64> = {
            let mut r = Rng::new(7);
            (0..5).map(|_| r.next_u64()).collect()
        };
        let mut r = Rng::new(7);
        assert_eq!(a, (0..5).map(|_| r.next_u64()).collect::<Vec<_>>());
        assert!(Rng::new(0).below(10) < 10);
        assert_eq!(Rng::new(1).below(0), 0);
    }
}
