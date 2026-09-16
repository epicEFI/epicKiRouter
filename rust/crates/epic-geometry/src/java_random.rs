//! Port of the JDK `java.util.Random` 48-bit linear congruential generator.
//!
//! Some Freerouting algorithms (for example randomized optimizer tie-breaks)
//! consume a `java.util.Random`; reproducing its exact output stream keeps
//! Rust runs comparable against Java logs. The LCG is public JDK spec:
//! `seed = (seed ^ 0x5DEECE66D) & ((1 << 48) - 1)` on construction and
//! `seed = (seed * 0x5DEECE66D + 0xB) & ((1 << 48) - 1)` per `next(bits)`.
//! Test constants are pinned against JDK 25 `jshell` output.

/// A bit-exact replica of `java.util.Random`'s output stream.
///
/// Java `setSeed(seed)` is exactly [`JavaRandom::new`] (the scramble only,
/// no other stream memory), so call sites that reseed — e.g.
/// `PolygonShape.java` via `setSeed` — are modeled by constructing a fresh
/// [`JavaRandom`].
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
    /// Java `new Random(seed)` / `Random.setSeed(seed)`.
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

    /// Java `nextInt(int bound)`, transliterated from the JDK 25 source:
    ///
    /// ```java
    /// int r = next(31);
    /// int m = bound - 1;
    /// if ((bound & m) == 0)  // i.e., bound is a power of 2
    ///     r = (int)((bound * (long)r) >> 31);
    /// else { // reject over-represented candidates
    ///     for (int u = r;
    ///          u - (r = u % bound) + m < 0;
    ///          u = next(31))
    ///         ;
    /// }
    /// return r;
    /// ```
    ///
    /// Rejection sampling consumes a VARIABLE number of `next(31)` draws
    /// (up to ~1/2 for bound `2^30 + 1`), so this cannot be emulated from
    /// [`JavaRandom::next_int`]. `bound == 0` (and any value above
    /// `i32::MAX`, unrepresentable as a Java `int`) panics, mirroring Java's
    /// `IllegalArgumentException("bound must be positive")`.
    pub fn next_int_bound(&mut self, bound: u32) -> i32 {
        assert!(
            bound != 0 && bound <= i32::MAX as u32,
            "bound must be positive (java.lang.IllegalArgumentException)"
        );
        let r = self.next(31) as i32;
        let m = (bound - 1) as i32;
        if (bound & (bound - 1)) == 0 {
            // i.e., bound is a power of 2
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        // Reject over-represented candidates. `wrapping_*` reproduces Java's
        // intentional `int` overflow in the loop condition (it detects
        // candidates too close to 2^31). As in the Java for-loop, the initial
        // draw only seeds `u`; every iteration rewrites the candidate.
        let mut u = r;
        loop {
            let r = u % (bound as i32);
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.next(31) as i32;
        }
    }

    /// Java `nextDouble()`: `((next(26) << 27) + next(27)) / 2^53`. Both the
    /// sum (at most 53 significant bits) and the power-of-two division are
    /// exact in `f64`, matching the JDK's `* DOUBLE_UNIT` (= `0x1.0p-53`).
    pub fn next_double(&mut self) -> f64 {
        (((self.next(26) << 27) + self.next(27)) as f64) / ((1u64 << 53) as f64)
    }

    /// Java `Collections.shuffle(list, this)`, the Fisher-Yates descending
    /// loop `for (int i = size; i > 1; i--) swap(list, i - 1, nextInt(i))`.
    /// The JDK takes an array-copy detour for non-`RandomAccess` lists, but
    /// the resulting PERMUTATION is identical to the direct path, so the
    /// slice shuffle reproduces it bit-exactly. Draw count (and therefore
    /// the generator state afterwards) is `size - 1` `nextInt` calls.
    pub fn shuffle<T>(&mut self, list: &mut [T]) {
        let mut i = list.len();
        while i > 1 {
            i -= 1;
            let j = self.next_int_bound(i as u32 + 1) as usize;
            list.swap(i, j);
        }
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

    /// Sequence of `nextInt(bound)` calls, pinned against JDK 25 jshell:
    /// `var a = new java.util.Random(99); a.nextInt(7); a.nextInt(7);
    /// a.nextInt(5); a.nextInt(8); a.nextInt(3); a.nextInt();` — the trailing
    /// unbounded draw locks the total stream position after the bounds.
    #[test]
    fn next_int_bound_sequence_matches_jdk_25() {
        let mut r = JavaRandom::new(99);
        assert_eq!(r.next_int_bound(7), 4);
        assert_eq!(r.next_int_bound(7), 6);
        assert_eq!(r.next_int_bound(5), 4);
        assert_eq!(r.next_int_bound(8), 5); // power of two
        assert_eq!(r.next_int_bound(3), 0);
        assert_eq!(r.next_int(), 1_681_943_525);
    }

    /// Stream-advancement pin (JDK 25 jshell): on seed 99, `nextInt(7)`
    /// rejects nothing, so it consumes exactly one draw — the following
    /// unbounded `nextInt()` equals the second plain draw of a fresh
    /// `Random(99)`.
    #[test]
    fn next_int_bound_advances_stream_exactly_one_draw_when_unrejected() {
        let mut with_bound = JavaRandom::new(99);
        assert_eq!(with_bound.next_int_bound(7), 4);
        assert_eq!(with_bound.next_int(), 1_672_896_916);
        let mut plain = JavaRandom::new(99);
        assert_eq!(plain.next_int(), -1_192_035_722);
        assert_eq!(plain.next_int(), 1_672_896_916);
    }

    /// Rejection sampling, pinned against JDK 25 jshell:
    /// `new java.util.Random(99)` with bound `2^30 + 1` (the documented worst
    /// case, reject probability 1/2) yields 836448458, 745722429, 840971762,
    /// 120279599. A bit-exact LCG simulation of the JDK loop shows these four
    /// calls consume 10 `next(31)` draws (1, 0, 2, and 3 rejections), so this
    /// pin fails if the rejection loop draws even once too few or too often.
    #[test]
    fn next_int_bound_rejection_loop_matches_jdk_25() {
        let mut r = JavaRandom::new(99);
        assert_eq!(r.next_int_bound((1 << 30) + 1), 836_448_458);
        assert_eq!(r.next_int_bound((1 << 30) + 1), 745_722_429);
        assert_eq!(r.next_int_bound((1 << 30) + 1), 840_971_762);
        assert_eq!(r.next_int_bound((1 << 30) + 1), 120_279_599);
    }

    /// Java throws `IllegalArgumentException` for `bound <= 0`; with a `u32`
    /// signature only 0 is expressible, and it panics with the same intent.
    #[test]
    #[should_panic(expected = "bound must be positive")]
    fn next_int_bound_panics_on_zero() {
        let mut r = JavaRandom::new(99);
        let _ = r.next_int_bound(0);
    }
}
