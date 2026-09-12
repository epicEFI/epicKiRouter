//! Port of the JDK `java.util.Random` 48-bit linear congruential generator.
//!
//! Some Freerouting algorithms (for example randomized optimizer tie-breaks)
//! consume a `java.util.Random`; reproducing its exact output stream keeps
//! Rust runs comparable against Java logs. The LCG is public JDK spec:
//! `seed = (seed ^ 0x5DEECE66D) & ((1 << 48) - 1)` on construction and
//! `seed = (seed * 0x5DEECE66D + 0xB) & ((1 << 48) - 1)` per `next(bits)`.
//! Test constants are pinned against JDK 25 `jshell` output.

/// A bit-exact replica of `java.util.Random`'s output stream.
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: i64,
}

/// Java `Random`'s `multiplier`.
const MULTIPLIER: i64 = 0x5_DEEC_E66D;
/// Java `Random`'s `addend`.
const ADDEND: i64 = 0xB;
/// The state is kept to the low 48 bits.
const MASK: i64 = (1 << 48) - 1;

impl JavaRandom {
    /// Java `new Random(seed)`.
    pub fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    /// Java private `next(int bits)`: advance the 48-bit state and return its
    /// top `bits` bits. `wrapping_*` mirrors Java's silent 64-bit wraparound;
    /// masking to 48 bits makes the result independent of it.
    fn next(&mut self, bits: u32) -> i64 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        self.seed >> (48 - bits)
    }

    /// Java `nextInt()`: the next 32 bits as a signed value.
    pub fn next_int(&mut self) -> i32 {
        self.next(32) as i32
    }

    /// Java `nextDouble()`: `((next(26) << 27) + next(27)) / 2^53`. Both the
    /// sum (at most 53 significant bits) and the power-of-two division are
    /// exact in `f64`, matching the JDK's `* DOUBLE_UNIT` (= `0x1.0p-53`).
    pub fn next_double(&mut self) -> f64 {
        (((self.next(26) << 27) + self.next(27)) as f64) / ((1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Third seed, pinned the same way: `new java.util.Random(0).nextInt()`.
    /// (This also exercises the seed scramble `seed ^ 0x5DEECE66D` of the
    /// public spec, since the scramble feeds the first `next` call.)
    #[test]
    fn random_0_first_next_int_matches_jdk_25() {
        let mut r = JavaRandom::new(0);
        assert_eq!(r.next_int(), -1_155_484_576);
    }

    /// Values produced by JDK 25 via jshell:
    /// `var r = new java.util.Random(99); r.nextInt(); r.nextDouble(); r.nextInt();`
    #[test]
    fn random_99_stream_matches_jdk_25() {
        let mut r = JavaRandom::new(99);
        assert_eq!(r.next_int(), -1_192_035_722);
        assert_eq!(r.next_double(), 0.3895016662367293);
        assert_eq!(r.next_int(), -1_244_121_273);
    }

    /// Second seed, pinned the same way: `new java.util.Random(1).nextInt()`.
    #[test]
    fn random_1_first_next_int_matches_jdk_25() {
        let mut r = JavaRandom::new(1);
        assert_eq!(r.next_int(), -1_155_869_325);
    }
}
