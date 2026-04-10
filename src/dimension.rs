//! Dimension effects for operations that change output size.
//!
//! [`DimensionEffect`] describes how an operation transforms dimensions,
//! enabling the pipeline planner to compute resize targets that account
//! for spatial transforms before and after resize.
//!
//! zenlayout provides built-in implementations for common effects
//! ([`RotateEffect`], [`PadEffect`], [`ExpandEffect`], [`TrimEffect`]).
//! Downstream crates can implement the trait for new operations
//! (perspective correction, lens distortion, etc.) without modifying
//! zenlayout.

use alloc::boxed::Box;
use core::fmt::Debug;

use crate::constraint::CanvasColor;
use crate::float_math::F32Ext;
use crate::plan::RegionCoord;

// ── Trait ──

/// Describes how an operation changes output dimensions.
///
/// Used by [`Command::Effect`](crate::Command) in the pipeline planner.
/// Effects are processed in user-specified order — the planner tracks
/// dimensions through each step and adjusts the resize target accordingly.
pub trait DimensionEffect: Debug + Send + Sync {
    /// Output dimensions given input dimensions.
    fn forward(&self, w: u32, h: u32) -> (u32, u32);

    /// Required input dimensions for desired output.
    /// Returns `None` if non-invertible (content-adaptive operations
    /// whose output depends on pixel analysis).
    fn inverse(&self, w: u32, h: u32) -> Option<(u32, u32)>;

    /// Map a point from input space to output space.
    ///
    /// Coordinates are fractional (0.0–1.0 = within the image).
    /// Values outside 0.0–1.0 are valid (padding regions, etc.).
    ///
    /// Default implementation scales linearly by the dimension ratio.
    fn forward_point(&self, x: f32, y: f32, in_w: u32, in_h: u32) -> (f32, f32) {
        let (out_w, out_h) = self.forward(in_w, in_h);
        (
            x * out_w as f32 / in_w.max(1) as f32,
            y * out_h as f32 / in_h.max(1) as f32,
        )
    }

    /// Map a point from output space back to input space.
    ///
    /// Default implementation scales linearly by the inverse dimension ratio.
    fn inverse_point(&self, x: f32, y: f32, out_w: u32, out_h: u32) -> Option<(f32, f32)> {
        let (in_w, in_h) = self.inverse(out_w, out_h)?;
        Some((
            x * in_w as f32 / out_w.max(1) as f32,
            y * in_h as f32 / out_h.max(1) as f32,
        ))
    }

    /// Clone into a boxed trait object.
    fn clone_boxed(&self) -> Box<dyn DimensionEffect>;
}

impl Clone for Box<dyn DimensionEffect> {
    fn clone(&self) -> Self {
        self.clone_boxed()
    }
}

// ── Built-in effects ──

/// Rotation by an arbitrary angle.
///
/// For cardinal angles (90°/180°/270°), prefer composing into
/// [`Orientation`](crate::Orientation) instead — it's free (no resampling).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RotateEffect {
    /// Rotation angle in radians.
    pub angle_rad: f32,
    /// How rotation affects the output canvas.
    pub mode: RotateMode,
}

/// How rotation affects the output canvas.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RotateMode {
    /// Largest inscribed axis-aligned rectangle (photo straightening).
    /// Output shrinks. No fill color needed.
    InscribedCrop,
    /// Bounding box of rotated rectangle (document deskew).
    /// Output grows. Corner areas filled with color.
    Expand { color: CanvasColor },
    /// Crop to original dimensions (subtle correction).
    /// Output same size as input. Corners lost.
    CropToOriginal,
}

impl RotateEffect {
    /// Create a rotation effect from degrees.
    pub fn from_degrees(angle_degrees: f32, mode: RotateMode) -> Self {
        Self {
            angle_rad: angle_degrees.to_radians_(),
            mode,
        }
    }
}

impl DimensionEffect for RotateEffect {
    fn forward(&self, w: u32, h: u32) -> (u32, u32) {
        match self.mode {
            RotateMode::InscribedCrop => inscribed_crop_dims(w, h, self.angle_rad),
            RotateMode::Expand { .. } => expanded_canvas_dims(w, h, self.angle_rad),
            RotateMode::CropToOriginal => (w, h),
        }
    }

    fn inverse(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        Some(match self.mode {
            RotateMode::InscribedCrop => inscribed_crop_inverse(w, h, self.angle_rad),
            RotateMode::Expand { .. } => expanded_canvas_inverse(w, h, self.angle_rad),
            RotateMode::CropToOriginal => (w, h),
        })
    }

    fn forward_point(&self, x: f32, y: f32, in_w: u32, in_h: u32) -> (f32, f32) {
        let (out_w, out_h) = self.forward(in_w, in_h);
        let fw = in_w as f32;
        let fh = in_h as f32;
        let ow = out_w as f32;
        let oh = out_h as f32;

        // Center of input and output
        let (cx_in, cy_in) = (fw / 2.0, fh / 2.0);
        let (cx_out, cy_out) = (ow / 2.0, oh / 2.0);

        // Rotate point around input center
        let (sin, cos) = (self.angle_rad.sin_(), self.angle_rad.cos_());
        let dx = x - cx_in;
        let dy = y - cy_in;
        let rx = dx * cos - dy * sin;
        let ry = dx * sin + dy * cos;

        // Translate to output center
        (rx + cx_out, ry + cy_out)
    }

    fn inverse_point(&self, x: f32, y: f32, out_w: u32, out_h: u32) -> Option<(f32, f32)> {
        let (in_w, in_h) = self.inverse(out_w, out_h)?;
        let fw = in_w as f32;
        let fh = in_h as f32;
        let ow = out_w as f32;
        let oh = out_h as f32;

        let (cx_in, cy_in) = (fw / 2.0, fh / 2.0);
        let (cx_out, cy_out) = (ow / 2.0, oh / 2.0);

        // Inverse rotation (negate angle)
        let (sin, cos) = ((-self.angle_rad).sin_(), (-self.angle_rad).cos_());
        let dx = x - cx_out;
        let dy = y - cy_out;
        let rx = dx * cos - dy * sin;
        let ry = dx * sin + dy * cos;

        Some((rx + cx_in, ry + cy_in))
    }

    fn clone_boxed(&self) -> Box<dyn DimensionEffect> {
        Box::new(*self)
    }
}

/// Padding/border using percentage or pixel amounts.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PadEffect {
    pub top: RegionCoord,
    pub right: RegionCoord,
    pub bottom: RegionCoord,
    pub left: RegionCoord,
    pub color: CanvasColor,
}

impl PadEffect {
    /// Uniform padding as a percentage of content dimensions.
    pub fn percent(amount: f32, color: CanvasColor) -> Self {
        Self {
            top: RegionCoord::pct(amount),
            right: RegionCoord::pct(amount),
            bottom: RegionCoord::pct(amount),
            left: RegionCoord::pct(amount),
            color,
        }
    }

    /// Uniform padding in pixels.
    pub fn pixels(amount: u32, color: CanvasColor) -> Self {
        let px = amount as i32;
        Self {
            top: RegionCoord::px(px),
            right: RegionCoord::px(px),
            bottom: RegionCoord::px(px),
            left: RegionCoord::px(px),
            color,
        }
    }
}

impl DimensionEffect for PadEffect {
    fn forward(&self, w: u32, h: u32) -> (u32, u32) {
        let left = self.left.resolve(w).max(0) as u32;
        let right = self.right.resolve(w).max(0) as u32;
        let top = self.top.resolve(h).max(0) as u32;
        let bottom = self.bottom.resolve(h).max(0) as u32;
        (w + left + right, h + top + bottom)
    }

    fn inverse(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        let left = self.left.resolve(w).max(0) as u32;
        let right = self.right.resolve(w).max(0) as u32;
        let top = self.top.resolve(h).max(0) as u32;
        let bottom = self.bottom.resolve(h).max(0) as u32;
        Some((
            w.saturating_sub(left + right),
            h.saturating_sub(top + bottom),
        ))
    }

    fn forward_point(&self, x: f32, y: f32, in_w: u32, in_h: u32) -> (f32, f32) {
        // Content shifts right/down by left/top padding
        let left = self.left.resolve(in_w).max(0) as f32;
        let top = self.top.resolve(in_h).max(0) as f32;
        (x + left, y + top)
    }

    fn inverse_point(&self, x: f32, y: f32, out_w: u32, out_h: u32) -> Option<(f32, f32)> {
        let (in_w, in_h) = self.inverse(out_w, out_h)?;
        let left = self.left.resolve(in_w).max(0) as f32;
        let top = self.top.resolve(in_h).max(0) as f32;
        Some((x - left, y - top))
    }

    fn clone_boxed(&self) -> Box<dyn DimensionEffect> {
        Box::new(*self)
    }
}

/// Canvas expansion by absolute pixel amounts.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExpandEffect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl DimensionEffect for ExpandEffect {
    fn forward(&self, w: u32, h: u32) -> (u32, u32) {
        (w + self.left + self.right, h + self.top + self.bottom)
    }

    fn inverse(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        Some((
            w.saturating_sub(self.left + self.right),
            h.saturating_sub(self.top + self.bottom),
        ))
    }

    fn forward_point(&self, x: f32, y: f32, _in_w: u32, _in_h: u32) -> (f32, f32) {
        (x + self.left as f32, y + self.top as f32)
    }

    fn inverse_point(&self, x: f32, y: f32, _out_w: u32, _out_h: u32) -> Option<(f32, f32)> {
        Some((x - self.left as f32, y - self.top as f32))
    }

    fn clone_boxed(&self) -> Box<dyn DimensionEffect> {
        Box::new(*self)
    }
}

/// Content-aware trim (non-invertible).
///
/// Actual dimensions are determined at runtime by pixel analysis.
/// The `estimated_margin_percent` is a planning hint — the planner
/// estimates output as `(1 - 2*margin) * input` per axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TrimEffect {
    /// Expected margin to trim, as a fraction (0.05 = ~5% per side).
    pub estimated_margin_percent: f32,
}

impl DimensionEffect for TrimEffect {
    fn forward(&self, w: u32, h: u32) -> (u32, u32) {
        let scale = 1.0 - 2.0 * self.estimated_margin_percent;
        ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
    }

    fn inverse(&self, _w: u32, _h: u32) -> Option<(u32, u32)> {
        None // Non-invertible: actual trim depends on content
    }

    fn clone_boxed(&self) -> Box<dyn DimensionEffect> {
        Box::new(*self)
    }
}

// ── Pure math functions ──

/// Largest axis-aligned rectangle inside a rotated `w × h` frame,
/// preserving the original aspect ratio.
///
/// Returns `(crop_w, crop_h)`. For angle = 0, returns `(w, h)`.
pub fn inscribed_crop_dims(w: u32, h: u32, angle_rad: f32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let theta = angle_rad.abs_() % core::f32::consts::FRAC_PI_2;
    if theta < 1e-7 {
        return (w, h);
    }
    let (sin, cos) = (theta.sin_(), theta.cos_());
    let fw = w as f32;
    let fh = h as f32;

    // The rotated rectangle's bounding box is (fw*cos + fh*sin) × (fw*sin + fh*cos).
    // We want the largest axis-aligned sub-rectangle with aspect ratio fw:fh.
    //
    // The inscribed rectangle is constrained by two pairs of rotated edges.
    // Each pair gives an upper bound on the inscribed width:
    //   from horizontal edges: fw*cos - fh*sin (valid when θ < atan(fw/fh))
    //   from vertical edges:   fh*cos - fw*sin (valid when θ < atan(fh/fw))
    //
    // The scale factor is: min of (each constraint / fw, each constraint / fh)
    // adjusted for aspect ratio preservation.

    let long = fw.max(fh);
    let short = fw.min(fh);

    // Scale factor: how much the inscribed rect is smaller than the original
    let scale = if long * sin <= short * cos {
        // Small angle: both edge constraints are positive
        // Inscribed width is limited by the tighter constraint
        let s1 = (fw * cos - fh * sin) / fw;
        let s2 = (fh * cos - fw * sin) / fh;
        s1.min(s2).max(0.0)
    } else {
        // Large angle: one constraint goes negative, use the other
        // This happens when θ > atan(short/long)
        short / (long * sin + short * cos)
    };

    let crop_w = (fw * scale).floor_().max(1.0);
    let crop_h = (fh * scale).floor_().max(1.0);
    (crop_w as u32, crop_h as u32)
}

/// Bounding box of a rotated `w × h` rectangle.
///
/// Returns `(canvas_w, canvas_h)`. Always ≥ `(w, h)` for non-zero angles.
pub fn expanded_canvas_dims(w: u32, h: u32, angle_rad: f32) -> (u32, u32) {
    let theta = angle_rad.abs_() % core::f32::consts::PI;
    if theta < 1e-7 {
        return (w, h);
    }
    let (sin, cos) = (theta.sin_(), theta.cos_().abs_());
    let fw = w as f32;
    let fh = h as f32;
    let canvas_w = fw * cos + fh * sin;
    let canvas_h = fw * sin + fh * cos;
    (canvas_w.ceil_() as u32, canvas_h.ceil_() as u32)
}

/// Inverse of inscribed crop: what source dimensions produce `(out_w, out_h)`
/// after rotation and inscribed crop.
pub fn inscribed_crop_inverse(out_w: u32, out_h: u32, angle_rad: f32) -> (u32, u32) {
    if out_w == 0 || out_h == 0 {
        return (0, 0);
    }
    let theta = angle_rad.abs_() % core::f32::consts::FRAC_PI_2;
    if theta < 1e-7 {
        return (out_w, out_h);
    }
    // Approximate inverse: if forward shrinks by ratio r, inverse expands by 1/r
    let (fw, fh) = (out_w as f32, out_h as f32);
    let (cw, _ch) = inscribed_crop_dims(1000, ((1000.0 * fh / fw) as u32).max(1), theta);
    let ratio = cw as f32 / 1000.0;
    if ratio > 0.0 {
        ((fw / ratio).ceil_() as u32, (fh / ratio).ceil_() as u32)
    } else {
        (out_w, out_h)
    }
}

/// Inverse of expanded canvas: what source dimensions produce `(out_w, out_h)`
/// after rotation and canvas expansion.
pub fn expanded_canvas_inverse(out_w: u32, out_h: u32, angle_rad: f32) -> (u32, u32) {
    let theta = angle_rad.abs_() % core::f32::consts::PI;
    if theta < 1e-7 {
        return (out_w, out_h);
    }
    let (sin, cos) = (theta.sin_(), theta.cos_().abs_());
    // Solve: out_w = src_w * cos + src_h * sin
    //        out_h = src_w * sin + src_h * cos
    // This is a 2×2 linear system.
    let det = cos * cos - sin * sin;
    if det.abs_() < 1e-7 {
        // 45° — degenerate, source is a square
        return (out_w, out_h);
    }
    let fw = out_w as f32;
    let fh = out_h as f32;
    let src_w = (fw * cos - fh * sin) / det;
    let src_h = (fh * cos - fw * sin) / det;
    (src_w.ceil_().max(1.0) as u32, src_h.ceil_().max(1.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inscribed_crop_zero_angle() {
        assert_eq!(inscribed_crop_dims(1000, 800, 0.0), (1000, 800));
    }

    #[test]
    fn inscribed_crop_small_angle() {
        let (w, h) = inscribed_crop_dims(1000, 800, 2.0_f32.to_radians_());
        // At 2°, loss should be minimal (< 5%)
        assert!(w >= 950, "w={w}");
        assert!(h >= 760, "h={h}");
        assert!(w < 1000);
    }

    #[test]
    fn expanded_canvas_small_angle() {
        let (w, h) = expanded_canvas_dims(1000, 800, 2.0_f32.to_radians_());
        // Canvas should grow
        assert!(w > 1000, "w={w}");
        assert!(h > 800, "h={h}");
        // But not by much at 2°
        assert!(w < 1050, "w={w}");
    }

    #[test]
    fn expanded_canvas_90_degrees() {
        let (w, h) = expanded_canvas_dims(1000, 800, core::f32::consts::FRAC_PI_2);
        // 90° rotation swaps dimensions
        assert!((w as i32 - 800).abs() <= 1, "w={w}");
        assert!((h as i32 - 1000).abs() <= 1, "h={h}");
    }

    #[test]
    fn inscribed_crop_inverse_roundtrip() {
        let (w, h) = (1000u32, 800u32);
        let angle = 10.0_f32.to_radians_();
        let (cw, ch) = inscribed_crop_dims(w, h, angle);
        let (iw, ih) = inscribed_crop_inverse(cw, ch, angle);
        // Inverse should recover approximately the original
        assert!(
            (iw as i32 - w as i32).unsigned_abs() <= 5,
            "iw={iw} expected ~{w}"
        );
        assert!(
            (ih as i32 - h as i32).unsigned_abs() <= 5,
            "ih={ih} expected ~{h}"
        );
    }

    #[test]
    fn expanded_canvas_inverse_roundtrip() {
        let (w, h) = (1000u32, 800u32);
        let angle = 10.0_f32.to_radians_();
        let (ew, eh) = expanded_canvas_dims(w, h, angle);
        let (iw, ih) = expanded_canvas_inverse(ew, eh, angle);
        assert!(
            (iw as i32 - w as i32).unsigned_abs() <= 2,
            "iw={iw} expected ~{w}"
        );
        assert!(
            (ih as i32 - h as i32).unsigned_abs() <= 2,
            "ih={ih} expected ~{h}"
        );
    }

    #[test]
    fn rotate_effect_inscribed_crop() {
        let effect = RotateEffect::from_degrees(15.0, RotateMode::InscribedCrop);
        let (w, h) = effect.forward(1000, 800);
        assert!(w < 1000);
        assert!(h < 800);
        let (iw, ih) = effect.inverse(w, h).unwrap();
        assert!((iw as i32 - 1000).unsigned_abs() <= 10, "iw={iw}");
    }

    #[test]
    fn rotate_effect_expand() {
        let effect = RotateEffect::from_degrees(
            15.0,
            RotateMode::Expand {
                color: CanvasColor::Transparent,
            },
        );
        let (w, h) = effect.forward(1000, 800);
        assert!(w > 1000, "w={w}");
        assert!(h > 800, "h={h}");
    }

    #[test]
    fn pad_effect_percent() {
        let effect = PadEffect::percent(0.1, CanvasColor::Transparent);
        let (w, h) = effect.forward(1000, 800);
        // 10% padding on each side: 1000 + 100 + 100 = 1200
        assert_eq!(w, 1200);
        assert_eq!(h, 960);
    }

    #[test]
    fn pad_effect_inverse() {
        let effect = PadEffect::pixels(20, CanvasColor::Transparent);
        let (w, h) = effect.forward(1000, 800);
        assert_eq!((w, h), (1040, 840));
        let (iw, ih) = effect.inverse(1040, 840).unwrap();
        assert_eq!((iw, ih), (1000, 800));
    }

    #[test]
    fn trim_non_invertible() {
        let effect = TrimEffect {
            estimated_margin_percent: 0.05,
        };
        assert!(effect.inverse(900, 720).is_none());
    }

    #[test]
    fn expand_effect() {
        let effect = ExpandEffect {
            left: 10,
            top: 20,
            right: 10,
            bottom: 20,
        };
        assert_eq!(effect.forward(100, 100), (120, 140));
        assert_eq!(effect.inverse(120, 140), Some((100, 100)));
    }

    // ── Point mapping tests ──

    fn approx_eq(a: (f32, f32), b: (f32, f32), tol: f32) -> bool {
        (a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol
    }

    #[test]
    fn rotate_point_center_stays() {
        // Center of image should stay at center after rotation
        let effect = RotateEffect::from_degrees(30.0, RotateMode::InscribedCrop);
        let (out_w, out_h) = effect.forward(1000, 800);
        let p = effect.forward_point(500.0, 400.0, 1000, 800);
        assert!(
            approx_eq(p, (out_w as f32 / 2.0, out_h as f32 / 2.0), 1.0),
            "center mapped to {p:?}, expected ~({}, {})",
            out_w as f32 / 2.0,
            out_h as f32 / 2.0
        );
    }

    #[test]
    fn rotate_point_roundtrip() {
        let effect = RotateEffect::from_degrees(
            15.0,
            RotateMode::Expand {
                color: CanvasColor::Transparent,
            },
        );
        let (out_w, out_h) = effect.forward(1000, 800);
        let p = effect.forward_point(200.0, 300.0, 1000, 800);
        let back = effect.inverse_point(p.0, p.1, out_w, out_h).unwrap();
        assert!(
            approx_eq(back, (200.0, 300.0), 2.0),
            "roundtrip: {back:?} expected ~(200, 300)"
        );
    }

    #[test]
    fn pad_point_shifts_by_padding() {
        let effect = PadEffect::pixels(20, CanvasColor::Transparent);
        let p = effect.forward_point(100.0, 50.0, 1000, 800);
        assert_eq!(p, (120.0, 70.0)); // shifted by left=20, top=20
    }

    #[test]
    fn pad_point_inverse() {
        let effect = PadEffect::pixels(20, CanvasColor::Transparent);
        let back = effect.inverse_point(120.0, 70.0, 1040, 840).unwrap();
        assert!(approx_eq(back, (100.0, 50.0), 0.1), "back={back:?}");
    }

    #[test]
    fn expand_point_shifts() {
        let effect = ExpandEffect {
            left: 10,
            top: 20,
            right: 10,
            bottom: 20,
        };
        assert_eq!(effect.forward_point(50.0, 50.0, 100, 100), (60.0, 70.0));
        assert_eq!(
            effect.inverse_point(60.0, 70.0, 120, 140),
            Some((50.0, 50.0))
        );
    }
}
