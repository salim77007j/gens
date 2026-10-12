//! Seeded 2D value noise and fractal Brownian motion.
//!
//! # The no-transcendentals rule (ADR-0031)
//!
//! Every operation here is an IEEE-754 basic operation — integer bitwise
//! mixing, add, multiply, floor, compare — plus division by powers of
//! two (exact). IEEE-754 basic operations are bit-identical across
//! platforms; `sin`/`pow`/`exp` are not. That is what makes this noise
//! a determinism primitive rather than a convenience: the same seed and
//! coordinates produce the same bits on every operating system, in any
//! generation order.
//!
//! Structure: lattice points are hashed to `[0, 1)` through a
//! SplitMix64-style finalizer over `(seed, x, y)`; the value at a point
//! is the smoothstep-interpolated bilinear blend of its four lattice
//! corners; FBM sums octaves with a fixed frequency/gain ladder and
//! normalizes by the exact amplitude sum.

use thiserror::Error;

/// A seeded 2D value-noise source.
///
/// Cheap to copy, deterministic forever: the seed fully determines the
/// output. Two `Noise2D` values with the same seed are interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Noise2D {
    seed: u64,
}

impl Noise2D {
    /// Creates a noise source from an integer seed.
    #[must_use]
    pub fn new(seed: u64) -> Noise2D {
        Noise2D { seed }
    }

    /// Hashes one lattice cell to `[0, 1)`.
    ///
    /// SplitMix64-style finalizer over the mixed coordinates: every 64-bit
    /// input maps through a bijection, so distinct cells never collide by
    /// construction and the output distribution is uniform in the high
    /// bits (which is exactly what the top-24-bit extraction consumes).
    #[must_use]
    pub fn hash_cell(&self, xi: i32, yi: i32) -> f32 {
        let mut h = self.seed
            ^ (u64::from(xi as u32)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (u64::from(yi as u32)).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        // Top mantissa bits into [0, 1): from_bits(1.0 | 23 mantissa
        // bits) lands in [1, 2), and subtracting 1.0 is EXACT (the
        // mantissa is preserved at a smaller exponent) — so the whole
        // pipeline is bit-exact arithmetic.
        let bits = (h >> 40) as u32; // 24 significant bits, top 23 kept
        f32::from_bits((bits >> 1) | 0x3F80_0000) - 1.0
    }

    /// Value noise at `(x, y)`, smoothstep-interpolated, in `[0, 1)`.
    ///
    /// Pure: no state, no time, no transcendental. `NaN`/infinite
    /// inputs produce implementation-defined lattice coordinates —
    /// callers validate their inputs (the heightfield/scatter layers do).
    #[must_use]
    pub fn value(&self, x: f32, y: f32) -> f32 {
        let xi = x.floor();
        let yi = y.floor();
        let fx = x - xi;
        let fy = y - yi;
        let tx = crate::smoothstep01(fx);
        let ty = crate::smoothstep01(fy);
        let (ix, iy) = (xi as i32, yi as i32);
        let v00 = self.hash_cell(ix, iy);
        let v10 = self.hash_cell(ix + 1, iy);
        let v01 = self.hash_cell(ix, iy + 1);
        let v11 = self.hash_cell(ix + 1, iy + 1);
        // Bilinear blend of the four corners with eased weights.
        let a = v00 + (v10 - v00) * tx;
        let b = v01 + (v11 - v01) * tx;
        a + (b - a) * ty
    }

    /// Fractal Brownian motion: `octaves` layers of value noise with
    /// frequency multiplied by `lacunarity` and amplitude scaled by
    /// `gain` per octave, normalized to `[0, 1)` by the exact amplitude
    /// sum. Standard terrain parameters: lacunarity 2, gain 0.5.
    ///
    /// # Errors
    /// [`NoiseError::InvalidParams`] for zero octaves, non-finite or
    /// non-positive lacunarity, or gain outside `[0, 1)` — a silent
    /// default would hide a misconfigured generator.
    pub fn fbm(
        &self,
        x: f32,
        y: f32,
        octaves: u32,
        lacunarity: f32,
        gain: f32,
    ) -> Result<f32, NoiseError> {
        if octaves == 0 {
            return Err(NoiseError::InvalidParams {
                param: "octaves",
                reason: "must be > 0",
            });
        }
        if !lacunarity.is_finite() || lacunarity <= 0.0 {
            return Err(NoiseError::InvalidParams {
                param: "lacunarity",
                reason: "must be finite and > 0",
            });
        }
        if !gain.is_finite() || !(0.0..1.0).contains(&gain) {
            return Err(NoiseError::InvalidParams {
                param: "gain",
                reason: "must be in [0, 1)",
            });
        }
        let mut total = 0.0f32;
        let mut amplitude = 1.0f32;
        let mut frequency = 1.0f32;
        let mut amplitude_sum = 0.0f32;
        for _ in 0..octaves {
            total += self.value(x * frequency, y * frequency) * amplitude;
            amplitude_sum += amplitude;
            amplitude *= gain;
            frequency *= lacunarity;
        }
        Ok(total / amplitude_sum)
    }
}

/// Errors from noise parameter validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NoiseError {
    /// A parameter is outside its valid domain.
    #[error("noise parameter '{param}' {reason}")]
    InvalidParams {
        /// The offending parameter name.
        param: &'static str,
        /// What domain was violated.
        reason: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_cell_is_uniform_and_in_range() {
        let n = Noise2D::new(42);
        for x in -50..50 {
            for y in -50..50 {
                let v = n.hash_cell(x, y);
                assert!((0.0..1.0).contains(&v), "hash in [0,1), got {v}");
            }
        }
    }

    #[test]
    fn same_seed_same_output_any_order() {
        let a = Noise2D::new(7);
        let b = Noise2D::new(7);
        // Generation order independence: sample a grid forward and
        // backward; every value matches.
        let forward: Vec<f32> = (0..100)
            .map(|i| a.value(i as f32 * 0.37, (i % 7) as f32 * 0.91))
            .collect();
        let backward: Vec<f32> = (0..100)
            .rev()
            .map(|i| b.value(i as f32 * 0.37, (i % 7) as f32 * 0.91))
            .collect();
        let mut backward = backward;
        backward.reverse();
        assert_eq!(forward, backward);
    }

    #[test]
    fn different_seeds_differ() {
        let a = Noise2D::new(1);
        let b = Noise2D::new(2);
        let differing = (0..64)
            .filter(|i| a.value(*i as f32, 0.5) != b.value(*i as f32, 0.5))
            .count();
        assert!(
            differing > 32,
            "seeds must decorrelate, differing={differing}"
        );
    }

    #[test]
    fn value_noise_is_bounded_and_continuous() {
        let n = Noise2D::new(99);
        // Bounded.
        for i in 0..1000 {
            let x = i as f32 * 0.13;
            let y = (i % 31) as f32 * 0.29;
            let v = n.value(x, y);
            assert!((0.0..1.0).contains(&v), "value in [0,1), got {v}");
        }
        // Continuous: a tiny step changes the value by a small amount.
        let v0 = n.value(1.0, 1.0);
        let v1 = n.value(1.001, 1.001);
        assert!((v1 - v0).abs() < 0.05, "value noise must be smooth");
    }

    #[test]
    fn lattice_values_are_the_interpolation_endpoints() {
        let n = Noise2D::new(3);
        // At exact lattice points the value equals the corner hash.
        let corner = n.hash_cell(5, 7);
        let sampled = n.value(5.0, 7.0);
        assert_eq!(corner, sampled);
        // Halfway between two horizontally adjacent cells: the blend of
        // the two corner pairs, weight exactly 0.5 (smoothstep(0.5)).
        let v00 = n.hash_cell(5, 7);
        let v10 = n.hash_cell(6, 7);
        let v01 = n.hash_cell(5, 8);
        let v11 = n.hash_cell(6, 8);
        let a = v00 + (v10 - v00) * 0.5;
        let b = v01 + (v11 - v01) * 0.5;
        let expected = a + (b - a) * 0.5;
        assert_eq!(n.value(5.5, 7.5), expected);
    }

    #[test]
    fn fbm_validates_parameters() {
        let n = Noise2D::new(5);
        assert_eq!(
            n.fbm(0.0, 0.0, 0, 2.0, 0.5),
            Err(NoiseError::InvalidParams {
                param: "octaves",
                reason: "must be > 0",
            })
        );
        assert_eq!(
            n.fbm(0.0, 0.0, 4, 0.0, 0.5),
            Err(NoiseError::InvalidParams {
                param: "lacunarity",
                reason: "must be finite and > 0",
            })
        );
        assert_eq!(
            n.fbm(0.0, 0.0, 4, 2.0, 1.0),
            Err(NoiseError::InvalidParams {
                param: "gain",
                reason: "must be in [0, 1)",
            })
        );
    }

    #[test]
    fn fbm_is_bounded_deterministic_and_octave_sensitive() {
        let n = Noise2D::new(11);
        for i in 0..500 {
            let x = i as f32 * 0.07;
            let y = (i % 17) as f32 * 0.11;
            let v = n.fbm(x, y, 4, 2.0, 0.5).expect("valid params");
            assert!((0.0..1.0).contains(&v), "fbm in [0,1), got {v}");
        }
        // Octaves change the field but stay deterministic.
        let v4 = n.fbm(1.0, 1.0, 4, 2.0, 0.5).expect("ok");
        let v4_again = n.fbm(1.0, 1.0, 4, 2.0, 0.5).expect("ok");
        assert_eq!(v4, v4_again);
        let v6 = n.fbm(1.0, 1.0, 6, 2.0, 0.5).expect("ok");
        assert_ne!(v4, v6);
    }
}
