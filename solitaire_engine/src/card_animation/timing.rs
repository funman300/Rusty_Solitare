//! Distance-based duration calculation and stagger utilities.
//!
//! All functions are pure (no Bevy dependency) and can be tested in isolation.

/// Minimum animation duration — applied to very short or zero-distance moves.
pub const MIN_DURATION_SECS: f32 = 0.12;

/// Hard cap on animation duration regardless of distance.
pub const MAX_DURATION_SECS: f32 = 0.35;

/// Sqrt scale factor calibrated so a 600-pixel move hits `MAX_DURATION_SECS`:
/// `MIN + √600 × SCALE ≈ 0.35 s`.
const SQRT_SCALE: f32 = 0.0094;

/// Micro-variation amplitude: ±0.4 % of the computed duration.
///
/// Small enough to be imperceptible in isolation but enough to break the
/// "robotic" uniformity when many cards animate simultaneously.
const MICRO_VARY_AMPLITUDE: f32 = 0.004;

/// Computes animation duration from a pixel distance using square-root scaling.
///
/// Square-root growth keeps short moves feeling instant while preventing long
/// moves from feeling excessively slow.
///
/// | Distance | Duration  |
/// |----------|-----------|
/// | 25 px    | ~0.17 s   |
/// | 100 px   | ~0.21 s   |
/// | 300 px   | ~0.28 s   |
/// | 600 px   | ~0.35 s   |
/// | 1200 px  | ~0.35 s ← capped |
#[inline]
pub fn compute_duration(distance: f32) -> f32 {
    (MIN_DURATION_SECS + distance.abs().sqrt() * SQRT_SCALE).min(MAX_DURATION_SECS)
}

/// Applies a deterministic ±0.4 % micro-variation to `duration`.
///
/// `entity_index` should be a stable per-entity value (e.g. `Entity::index()`).
/// The same index always produces the same variation so animations don't
/// change between frames.
#[inline]
pub fn micro_vary(duration: f32, entity_index: u32) -> f32 {
    // Multiplicative Fibonacci hash — cheap, decent distribution.
    let hash = entity_index.wrapping_mul(2_654_435_761);
    let noise = (hash >> 16) as f32 / 65_536.0; // 0.0 ..= 1.0
    let variation = (noise - 0.5) * 2.0 * MICRO_VARY_AMPLITUDE;
    duration * (1.0 + variation)
}

/// Returns the pre-animation delay for card at `index` in a staggered cascade.
///
/// `delay = index × interval_secs`.
#[inline]
pub fn cascade_delay(index: usize, interval_secs: f32) -> f32 {
    index as f32 * interval_secs
}

/// Recommended per-card interval for the win cascade (Normal speed).
pub const WIN_CASCADE_INTERVAL_SECS: f32 = 0.018;

/// Recommended per-card interval for deal animations (Normal speed).
pub const DEAL_INTERVAL_SECS: f32 = 0.022;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_distance_gives_minimum_duration() {
        assert!(
            (compute_duration(0.0) - MIN_DURATION_SECS).abs() < 1e-5,
            "zero distance must yield MIN_DURATION_SECS"
        );
    }

    #[test]
    fn large_distance_is_capped() {
        assert!(
            (compute_duration(10_000.0) - MAX_DURATION_SECS).abs() < 1e-5,
            "very large distance must be capped at MAX_DURATION_SECS"
        );
    }

    #[test]
    fn duration_increases_monotonically() {
        let mut prev = 0.0f32;
        for d in [10, 50, 100, 200, 400, 600] {
            let dur = compute_duration(d as f32);
            assert!(dur >= prev, "duration must be monotone: d={d} dur={dur} prev={prev}");
            prev = dur;
        }
    }

    #[test]
    fn duration_is_within_bounds() {
        for d in [0, 1, 25, 100, 300, 600, 1200] {
            let dur = compute_duration(d as f32);
            assert!(
                (MIN_DURATION_SECS..=MAX_DURATION_SECS).contains(&dur),
                "duration out of bounds for d={d}: {dur}"
            );
        }
    }

    #[test]
    fn micro_vary_stays_within_tolerance() {
        for i in 0..=1000u32 {
            let base = 0.25;
            let varied = micro_vary(base, i);
            let ratio = (varied - base).abs() / base;
            assert!(
                ratio <= MICRO_VARY_AMPLITUDE + 1e-6,
                "variation for index {i} exceeds amplitude: ratio={ratio}"
            );
        }
    }

    #[test]
    fn micro_vary_is_deterministic() {
        let a = micro_vary(0.2, 42);
        let b = micro_vary(0.2, 42);
        assert!((a - b).abs() < 1e-9, "micro_vary must be deterministic");
    }

    #[test]
    fn micro_vary_differs_for_different_indices() {
        let a = micro_vary(0.2, 1);
        let b = micro_vary(0.2, 2);
        // Very unlikely to be equal (would require hash collision mod 65536).
        assert!((a - b).abs() > 1e-9, "micro_vary should differ for different indices");
    }

    #[test]
    fn cascade_delay_zero_index_is_zero() {
        assert_eq!(cascade_delay(0, 0.018), 0.0);
    }

    #[test]
    fn cascade_delay_scales_linearly() {
        let interval = 0.018;
        for i in 0..52usize {
            let expected = i as f32 * interval;
            let actual = cascade_delay(i, interval);
            assert!(
                (actual - expected).abs() < 1e-6,
                "cascade_delay({i}) = {actual}, expected {expected}"
            );
        }
    }
}
