//! `no_std`-compatible float math operations.
//!
//! `f64::round()`, `f64::floor()`, and `f64::ceil()` are inherent methods
//! on `f64` in `std` but not available in `core` (stable). This module
//! provides a trait [`F64Ext`] with equivalent methods that work in both
//! `std` and `no_std` environments.

/// Extension trait providing `round_()`, `floor_()`, `ceil_()` on `f64`.
///
/// When `std` is available, these delegate to the standard library.
/// Without `std`, pure-Rust implementations using truncation are used.
pub(crate) trait F64Ext {
    fn round_(self) -> f64;
    fn floor_(self) -> f64;
    fn ceil_(self) -> f64;
}

#[cfg(feature = "std")]
impl F64Ext for f64 {
    #[inline(always)]
    fn round_(self) -> f64 {
        self.round()
    }

    #[inline(always)]
    fn floor_(self) -> f64 {
        self.floor()
    }

    #[inline(always)]
    fn ceil_(self) -> f64 {
        self.ceil()
    }
}

#[cfg(not(feature = "std"))]
impl F64Ext for f64 {
    #[inline(always)]
    fn round_(self) -> f64 {
        // Ties away from zero, matching std::f64::round.
        if self >= 0.0 {
            (self + 0.5).floor_()
        } else {
            (self - 0.5).ceil_()
        }
    }

    #[inline(always)]
    fn floor_(self) -> f64 {
        let i = self as i64;
        let fi = i as f64;
        if self < fi { fi - 1.0 } else { fi }
    }

    #[inline(always)]
    fn ceil_(self) -> f64 {
        let i = self as i64;
        let fi = i as f64;
        if self > fi { fi + 1.0 } else { fi }
    }
}

/// Extension trait providing `sin_()`, `cos_()`, `floor_()`, `ceil_()`
/// and `to_radians_()` on `f32`.
///
/// When `std` is available, these delegate to the standard library.
/// Without `std`, pure-Rust approximations are used.
pub(crate) trait F32Ext {
    fn sin_(self) -> f32;
    fn cos_(self) -> f32;
    fn floor_(self) -> f32;
    fn ceil_(self) -> f32;
    fn abs_(self) -> f32;
    fn to_radians_(self) -> f32;
}

#[cfg(feature = "std")]
impl F32Ext for f32 {
    #[inline(always)]
    fn sin_(self) -> f32 {
        self.sin()
    }
    #[inline(always)]
    fn cos_(self) -> f32 {
        self.cos()
    }
    #[inline(always)]
    fn floor_(self) -> f32 {
        self.floor()
    }
    #[inline(always)]
    fn ceil_(self) -> f32 {
        self.ceil()
    }
    #[inline(always)]
    fn abs_(self) -> f32 {
        self.abs()
    }
    #[inline(always)]
    fn to_radians_(self) -> f32 {
        self.to_radians()
    }
}

#[cfg(not(feature = "std"))]
impl F32Ext for f32 {
    #[inline(always)]
    fn sin_(self) -> f32 {
        // Bhaskara I approximation, good enough for dimension planning.
        // Normalize to [0, 2π]
        let mut x = self % (2.0 * core::f32::consts::PI);
        if x < 0.0 {
            x += 2.0 * core::f32::consts::PI;
        }
        let negate = x > core::f32::consts::PI;
        if negate {
            x -= core::f32::consts::PI;
        }
        // Bhaskara: sin(x) ≈ 16x(π-x) / (5π² - 4x(π-x))
        let pi = core::f32::consts::PI;
        let num = 16.0 * x * (pi - x);
        let den = 5.0 * pi * pi - 4.0 * x * (pi - x);
        let result = num / den;
        if negate { -result } else { result }
    }

    #[inline(always)]
    fn cos_(self) -> f32 {
        (self + core::f32::consts::FRAC_PI_2).sin_()
    }

    #[inline(always)]
    fn floor_(self) -> f32 {
        let i = self as i32;
        let fi = i as f32;
        if self < fi { fi - 1.0 } else { fi }
    }

    #[inline(always)]
    fn ceil_(self) -> f32 {
        let i = self as i32;
        let fi = i as f32;
        if self > fi { fi + 1.0 } else { fi }
    }

    #[inline(always)]
    fn abs_(self) -> f32 {
        if self < 0.0 { -self } else { self }
    }

    #[inline(always)]
    fn to_radians_(self) -> f32 {
        self * (core::f32::consts::PI / 180.0)
    }
}

#[cfg(test)]
mod tests {
    use super::F64Ext;

    #[test]
    fn round_positive() {
        assert_eq!(3.3_f64.round_(), 3.0);
        assert_eq!(3.5_f64.round_(), 4.0);
        assert_eq!(3.7_f64.round_(), 4.0);
        assert_eq!(4.0_f64.round_(), 4.0);
    }

    #[test]
    fn round_negative() {
        assert_eq!((-3.3_f64).round_(), -3.0);
        assert_eq!((-3.5_f64).round_(), -4.0);
        assert_eq!((-3.7_f64).round_(), -4.0);
    }

    #[test]
    fn floor_values() {
        assert_eq!(3.7_f64.floor_(), 3.0);
        assert_eq!(3.0_f64.floor_(), 3.0);
        assert_eq!((-3.2_f64).floor_(), -4.0);
        assert_eq!((-3.0_f64).floor_(), -3.0);
    }

    #[test]
    fn ceil_values() {
        assert_eq!(3.2_f64.ceil_(), 4.0);
        assert_eq!(3.0_f64.ceil_(), 3.0);
        assert_eq!((-3.7_f64).ceil_(), -3.0);
        assert_eq!((-3.0_f64).ceil_(), -3.0);
    }

    #[test]
    fn round_zero() {
        assert_eq!(0.0_f64.round_(), 0.0);
        assert_eq!(0.4_f64.round_(), 0.0);
        assert_eq!(0.5_f64.round_(), 1.0);
    }
}
