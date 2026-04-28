//! Motion curve definitions for card animations.
//!
//! All curves map `t ∈ [0, 1]` to a position ratio. Curves with overshoot
//! (`SmoothSnap`, `SoftBounce`, `Expressive`) may return values slightly
//! outside `[0, 1]` near the destination — callers should not clamp the output
//! before applying it to a lerp, as the overshoot is intentional.
//!
//! # Curve selection guide
//!
//! | Interaction          | Recommended curve |
//! |----------------------|-------------------|
//! | Standard card move   | `SmoothSnap`      |
//! | Foundation placement | `SoftBounce`      |
//! | Invalid snap-back    | `Responsive`      |
//! | Win cascade          | `Expressive`      |

use std::f32::consts::PI;

/// Motion curve variant controlling animation easing behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionCurve {
    /// Cubic ease-out with a 1.5 % terminal overshoot.
    ///
    /// Overshoot is a sine arch in the final 25 % of the animation that peaks
    /// ~1.5 % beyond the target, settling cleanly to 1.0 at `t = 1`. Gives a
    /// lively, slightly "alive" feel without feeling heavy.
    #[default]
    SmoothSnap,

    /// Underdamped spring (ζ = 0.65, ω = 20 rad/s).
    ///
    /// One visible overshoot of ~8 % followed by fast decay. Good for
    /// satisfying "thud" feedback when placing cards on foundations or tableau.
    SoftBounce,

    /// Quintic ease-out — aggressive deceleration, zero overshoot.
    ///
    /// Starts extremely fast and decelerates hard. Best for snap-back on
    /// invalid drops: the card returns instantly without any bounce.
    Responsive,

    /// Underdamped spring (ζ = 0.45, ω = 18 rad/s).
    ///
    /// Two visible bounces before settling. High visual energy — reserved for
    /// win cascade animations where expressivity matters more than subtlety.
    Expressive,
}

/// Samples `curve` at normalised time `t ∈ [0, 1]`.
///
/// The return value is the interpolation factor to pass to `Vec2::lerp` /
/// `Vec3::lerp`. Values may slightly exceed 1.0 for curves with overshoot.
#[inline]
pub fn sample_curve(curve: MotionCurve, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        MotionCurve::SmoothSnap => smooth_snap(t),
        MotionCurve::SoftBounce => soft_bounce(t),
        MotionCurve::Responsive => responsive(t),
        MotionCurve::Expressive => expressive(t),
    }
}

/// Cubic ease-out with a sine-arch overshoot in the final 25 % of `t`.
///
/// The overshoot term is `sin(tail * π) * 0.015` where `tail` is `t` linearly
/// rescaled from `[0.75, 1.0]` to `[0, 1]`. At `t = 0.875` the card is ~1.5 %
/// past its target; at `t = 1` the card is exactly on target.
#[inline]
fn smooth_snap(t: f32) -> f32 {
    let base = 1.0 - (1.0 - t).powi(3);
    let tail = ((t - 0.75) / 0.25).clamp(0.0, 1.0);
    let overshoot = (tail * PI).sin() * 0.015;
    base + overshoot
}

/// Underdamped spring response (ζ = 0.65, ω₀ = 20 rad/s).
///
/// Derived from the exact closed-form solution:
/// `x(t) = 1 − e^{−ζω₀t}[cos(ωd·t) + (ζω₀/ωd)·sin(ωd·t)]`
/// where `ωd = ω₀·√(1 − ζ²)`.
#[inline]
fn soft_bounce(t: f32) -> f32 {
    const OMEGA: f32 = 20.0;
    const ZETA: f32 = 0.65;
    let omega_d = OMEGA * (1.0 - ZETA * ZETA).sqrt();
    let decay = (-ZETA * OMEGA * t).exp();
    1.0 - decay * ((omega_d * t).cos() + (ZETA * OMEGA / omega_d) * (omega_d * t).sin())
}

/// Quintic ease-out: `f(t) = 1 − (1 − t)^5`.
///
/// Reaches ~97 % of the target by `t = 0.5`. No overshoot.
#[inline]
fn responsive(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(5)
}

/// Underdamped spring response (ζ = 0.45, ω₀ = 18 rad/s) — two visible bounces.
///
/// Uses the same closed-form spring formula as `soft_bounce` but with lower
/// damping, producing higher overshoot (~18 %) and two discernible oscillations
/// before settling.
#[inline]
fn expressive(t: f32) -> f32 {
    const OMEGA: f32 = 18.0;
    const ZETA: f32 = 0.45;
    let omega_d = OMEGA * (1.0 - ZETA * ZETA).sqrt();
    let decay = (-ZETA * OMEGA * t).exp();
    1.0 - decay * ((omega_d * t).cos() + (ZETA * OMEGA / omega_d) * (omega_d * t).sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_near(a: f32, b: f32, eps: f32, msg: &str) {
        assert!((a - b).abs() < eps, "{msg}: expected ~{b}, got {a}");
    }

    #[test]
    fn all_curves_start_at_zero() {
        for curve in [
            MotionCurve::SmoothSnap,
            MotionCurve::SoftBounce,
            MotionCurve::Responsive,
            MotionCurve::Expressive,
        ] {
            assert_near(sample_curve(curve, 0.0), 0.0, 1e-5, &format!("{curve:?} at t=0"));
        }
    }

    #[test]
    fn all_curves_end_at_one() {
        for curve in [
            MotionCurve::SmoothSnap,
            MotionCurve::SoftBounce,
            MotionCurve::Responsive,
        ] {
            assert_near(sample_curve(curve, 1.0), 1.0, 1e-4, &format!("{curve:?} at t=1"));
        }
        // Spring-based curves have residual oscillation at finite t=1; allow 2 e-3.
        assert_near(
            sample_curve(MotionCurve::Expressive, 1.0),
            1.0,
            2e-3,
            "Expressive at t=1",
        );
    }

    #[test]
    fn responsive_reaches_half_before_midpoint() {
        // Quintic ease-out accelerates fast — >50 % by t=0.5.
        let v = sample_curve(MotionCurve::Responsive, 0.5);
        assert!(v > 0.96, "Responsive should be >96 % at t=0.5, got {v}");
    }

    #[test]
    fn smooth_snap_overshoots_slightly_near_end() {
        // Peak overshoot is around t = 0.875.
        let peak = sample_curve(MotionCurve::SmoothSnap, 0.875);
        assert!(peak > 1.0, "SmoothSnap should overshoot at t=0.875, got {peak}");
        assert!(peak < 1.03, "SmoothSnap overshoot should be small (<3 %), got {peak}");
    }

    #[test]
    fn soft_bounce_overshoots_and_returns() {
        let v = sample_curve(MotionCurve::SoftBounce, 1.0);
        assert_near(v, 1.0, 1e-3, "SoftBounce must settle at 1.0");
    }

    #[test]
    fn expressive_has_more_overshoot_than_soft_bounce() {
        // Compare max value in [0,1] range.
        let max_soft: f32 = (0..=100)
            .map(|i| sample_curve(MotionCurve::SoftBounce, i as f32 / 100.0))
            .fold(f32::NEG_INFINITY, f32::max);
        let max_expr: f32 = (0..=100)
            .map(|i| sample_curve(MotionCurve::Expressive, i as f32 / 100.0))
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max_expr > max_soft,
            "Expressive should overshoot more than SoftBounce: {max_expr} vs {max_soft}"
        );
    }

    #[test]
    fn sample_curve_clamps_t_below_zero() {
        assert_near(sample_curve(MotionCurve::SmoothSnap, -1.0), 0.0, 1e-5, "t<0 clamped");
    }

    #[test]
    fn sample_curve_clamps_t_above_one() {
        assert_near(sample_curve(MotionCurve::Responsive, 2.0), 1.0, 1e-5, "t>1 clamped");
    }
}
