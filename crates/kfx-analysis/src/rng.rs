//! Deterministic randomness for the generator.
//!
//! Everything here gives the same results on every platform: integer arithmetic, and floats
//! only through `+ - * /` and comparisons, which IEEE 754 defines exactly (Rust does not fuse
//! multiply-adds on its own). The library `sin` / `cos` are not used (their precision is platform
//! dependent); [`cos_sin`] is a fixed polynomial.

/// SplitMix64 (Steele, Lea, Flood 2014): small, fast, full 64-bit period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x6A09_E667_F3BC_C909)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }
    /// Uniform in `0..n` (`n > 0`; 0 when `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Lemire's multiply-shift, slightly biased for huge n, exact enough here
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }
    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as u64) as i32
    }
    /// True with probability `num / den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    /// Pick from a slice.
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            v.get(self.below(v.len() as u64) as usize)
        }
    }
    /// A derived generator (independent stream for a sub-task).
    pub fn fork(&mut self, tag: u64) -> Rng {
        Rng::new(self.next_u64() ^ tag.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }
}

/// The SplitMix64 finaliser.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash of a lattice point, 0..65535.
pub fn hash2(seed: u64, x: i64, y: i64) -> u32 {
    (mix(seed
        ^ mix((x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)))
        >> 48) as u32
}

/// Integer value noise: lattice spacing `cell` (slabs), smoothstep interpolation in fixed
/// point, result 0..65535.
pub fn value_noise(seed: u64, x: i64, y: i64, cell: i64) -> u32 {
    let cell = cell.max(1);
    let (gx, gy) = (x.div_euclid(cell), y.div_euclid(cell));
    let (fx, fy) = (x.rem_euclid(cell), y.rem_euclid(cell));
    // t in 0..=1024
    let s = |f: i64| {
        let t = f * 1024 / cell;
        t * t * (3 * 1024 - 2 * t) / (1024 * 1024)
    };
    let (sx, sy) = (s(fx), s(fy));
    let a = hash2(seed, gx, gy) as i64;
    let b = hash2(seed, gx + 1, gy) as i64;
    let c = hash2(seed, gx, gy + 1) as i64;
    let d = hash2(seed, gx + 1, gy + 1) as i64;
    let top = a + (b - a) * sx / 1024;
    let bot = c + (d - c) * sx / 1024;
    (top + (bot - top) * sy / 1024) as u32
}

/// Two octaves of [`value_noise`] (0..65535).
pub fn fractal(seed: u64, x: i64, y: i64, cell: i64) -> u32 {
    let a = value_noise(seed, x, y, cell) as u64;
    let b = value_noise(seed ^ 0x5555, x, y, (cell / 2).max(1)) as u64;
    ((2 * a + b) / 3) as u32
}

/// `(cos, sin)` of `turns` full turns (any value), by range reduction to an octant and a
/// fixed Taylor polynomial (error below 1e-11). Deterministic: basic float operations only.
pub fn cos_sin(turns: f64) -> (f64, f64) {
    let t = turns - (turns as i64) as f64; // fraction, may be negative
    let t = if t < 0.0 { t + 1.0 } else { t };
    // quadrant
    let q = (t * 4.0) as i64;
    let r = t - q as f64 / 4.0; // 0 .. 0.25
    let a = r * 6.283_185_307_179_586; // radians 0 .. pi/2
    let a2 = a * a;
    let c = 1.0 - a2 / 2.0 + a2 * a2 / 24.0 - a2 * a2 * a2 / 720.0 + a2 * a2 * a2 * a2 / 40320.0
        - a2 * a2 * a2 * a2 * a2 / 3_628_800.0
        + a2 * a2 * a2 * a2 * a2 * a2 / 479_001_600.0
        - a2 * a2 * a2 * a2 * a2 * a2 * a2 / 87_178_291_200.0
        + a2 * a2 * a2 * a2 * a2 * a2 * a2 * a2 / 20_922_789_888_000.0;
    let s = a
        * (1.0 - a2 / 6.0 + a2 * a2 / 120.0 - a2 * a2 * a2 / 5040.0
            + a2 * a2 * a2 * a2 / 362_880.0
            - a2 * a2 * a2 * a2 * a2 / 39_916_800.0
            + a2 * a2 * a2 * a2 * a2 * a2 / 6_227_020_800.0
            - a2 * a2 * a2 * a2 * a2 * a2 * a2 / 1_307_674_368_000.0
            + a2 * a2 * a2 * a2 * a2 * a2 * a2 * a2 / 355_687_428_096_000.0);
    match q {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    }
}

/// Round half away from zero without `f64::round` (which is exact anyway, but this keeps the
/// rule visible): used for positions.
pub fn round(v: f64) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i32
    } else {
        -((-v + 0.5) as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_sin_matches_the_library() {
        for i in -40..=40 {
            let t = i as f64 / 13.0;
            let (c, s) = cos_sin(t);
            let a = t * std::f64::consts::TAU;
            assert!(
                (c - a.cos()).abs() < 1e-11 && (s - a.sin()).abs() < 1e-11,
                "{t}: {c} {s} vs {} {}",
                a.cos(),
                a.sin()
            );
        }
    }

    #[test]
    fn noise_is_smooth() {
        let mut big = 0;
        for x in 0..200 {
            let d = value_noise(9, x + 1, 5, 8) as i64 - value_noise(9, x, 5, 8) as i64;
            big = big.max(d.abs());
        }
        assert!(big < 65536 / 4, "neighbouring cells differ by {big}");
    }
}
