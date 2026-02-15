//! Command pipeline and decoder negotiation.
//!
//! Two-phase layout planning:
//! 1. [`plan()`] — compute ideal layout from commands + source dimensions → [`IdealLayout`] + [`DecoderRequest`]
//! 2. [`finalize()`] — given what the decoder actually did ([`DecoderOffer`]), compute remaining work → [`LayoutPlan`]

use crate::constraint::{
    CanvasColor, Constraint, ConstraintMode, Layout, LayoutError, Rect, SourceCrop,
};
use crate::orientation::Orientation;

/// Rotation amount for manual rotation commands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Rotation {
    /// 90 degrees clockwise.
    Rotate90,
    /// 180 degrees.
    Rotate180,
    /// 270 degrees clockwise (90 counter-clockwise).
    Rotate270,
}

/// Axis for manual flip commands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FlipAxis {
    /// Flip left-right.
    Horizontal,
    /// Flip top-bottom.
    Vertical,
}

/// A single image processing command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Apply EXIF orientation correction (value 1-8).
    AutoOrient(u8),
    /// Manual rotation, stacks with EXIF.
    Rotate(Rotation),
    /// Manual flip, stacks with other orientation commands.
    Flip(FlipAxis),
    /// Crop in post-orientation coordinates.
    Crop(SourceCrop),
    /// Constrain dimensions in post-orientation coordinates.
    Constrain {
        /// The constraint to apply.
        constraint: Constraint,
    },
    /// Add padding around the image.
    Pad {
        /// Top padding in pixels.
        top: u32,
        /// Right padding in pixels.
        right: u32,
        /// Bottom padding in pixels.
        bottom: u32,
        /// Left padding in pixels.
        left: u32,
        /// Padding color.
        color: CanvasColor,
    },
}

/// Result of the first phase of layout planning.
#[derive(Clone, Debug, PartialEq)]
pub struct IdealLayout {
    /// Net orientation to apply.
    pub orientation: Orientation,
    /// Layout computed in post-orientation space.
    pub layout: Layout,
    /// Source crop transformed back to pre-orientation source coordinates.
    pub source_crop: Option<Rect>,
    /// Padding to add around the final image.
    pub padding: Option<Padding>,
    /// If [`Align::Extend`] was used, crop to these dimensions after encoding.
    /// Canvas was extended with replicated edges; this records the real content size.
    pub encode_crop: Option<(u32, u32)>,
}

/// Explicit padding specification.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Padding {
    /// Top padding in pixels.
    pub top: u32,
    /// Right padding in pixels.
    pub right: u32,
    /// Bottom padding in pixels.
    pub bottom: u32,
    /// Left padding in pixels.
    pub left: u32,
    /// Padding color.
    pub color: CanvasColor,
}

/// What the layout engine wants the decoder to do.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecoderRequest {
    /// Crop region in pre-orientation source coordinates.
    pub crop: Option<Rect>,
    /// Hint for prescale target dimensions.
    pub prescale_target: (u32, u32),
    /// Orientation the engine would like the decoder to handle.
    pub orientation: Orientation,
}

/// What the decoder actually did.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecoderOffer {
    /// Dimensions of the decoded output.
    pub dimensions: (u32, u32),
    /// Crop the decoder applied (in source coordinates).
    pub crop_applied: Option<Rect>,
    /// Orientation the decoder applied.
    pub orientation_applied: Orientation,
}

impl DecoderOffer {
    /// Default offer: decoder did nothing special, just decoded at full size.
    pub fn full_decode(w: u32, h: u32) -> Self {
        Self {
            dimensions: (w, h),
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        }
    }
}

/// Final layout plan after decoder negotiation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutPlan {
    /// What was requested of the decoder.
    pub decoder_request: DecoderRequest,
    /// Trim rect to apply to decoder output (for block-aligned overshoot).
    pub trim: Option<Rect>,
    /// Dimensions to resize to.
    pub resize_to: (u32, u32),
    /// Orientation remaining after decoder's contribution.
    pub remaining_orientation: Orientation,
    /// Final canvas dimensions (may be extended for alignment).
    pub canvas: (u32, u32),
    /// Placement offset on canvas.
    pub placement: (u32, u32),
    /// Canvas background color.
    pub canvas_color: CanvasColor,
    /// True when no resize is needed (enables lossless path).
    pub resize_is_identity: bool,
    /// If [`Align::Extend`] was used, crop to these dimensions after encoding.
    /// Renderer should replicate edge pixels into the extension area.
    pub encode_crop: Option<(u32, u32)>,
}

/// How to align canvas dimensions to codec-required multiples.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    /// Round canvas down to nearest multiple. May lose up to `(n-1)` pixels per axis.
    /// Use for video codecs (mod-2) where pixel-exact dimensions aren't critical.
    RoundDown(u32),
    /// Extend canvas up to nearest multiple. Image placed at `(0, 0)`, renderer
    /// replicates edge pixels into the extension area. Original content dimensions
    /// stored in [`IdealLayout::encode_crop`] / [`LayoutPlan::encode_crop`] for
    /// post-encode cropping. No content loss.
    ///
    /// This is how JPEG MCU padding works — encode at extended size, store real
    /// dimensions in the header, decoders crop automatically.
    Extend(u32),
}

/// Post-computation safety limits applied after all layout computation.
///
/// All limits target the **canvas** (the encoded output dimensions):
/// - `max`: prevents absurdly large outputs (security). Proportional downscale.
/// - `min`: prevents degenerate tiny outputs. Proportional upscale.
/// - `align`: snaps canvas to codec-required multiples.
///
/// If `max` and `min` conflict, `max` wins (security trumps aesthetics).
///
/// Applied to the [`Layout`] after constraint + padding computation, before
/// source crop is transformed back to source coordinates.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MandatoryConstraints {
    /// Maximum canvas dimensions. If exceeded, everything scales down proportionally.
    pub max: Option<(u32, u32)>,
    /// Minimum canvas dimensions. If below, everything scales up proportionally.
    pub min: Option<(u32, u32)>,
    /// Snap canvas to multiples. See [`Align`] for round-down vs extend modes.
    pub align: Option<Align>,
}

impl MandatoryConstraints {
    /// Apply limits to a computed layout.
    ///
    /// Returns the modified layout and an optional encode_crop. If [`Align::Extend`]
    /// was used, `encode_crop` contains the original content dimensions — the
    /// renderer should replicate edge pixels into the extension area, and the
    /// encoder should record these as the real image dimensions.
    ///
    /// Order: max (cap canvas) → min (floor canvas) → align (snap canvas).
    /// Max wins if min conflicts.
    pub fn apply(&self, layout: Layout) -> (Layout, Option<(u32, u32)>) {
        let mut layout = layout;

        // 1. Max: if canvas exceeds max, scale everything down proportionally.
        if let Some((max_w, max_h)) = self.max {
            if max_w > 0 && max_h > 0 && (layout.canvas.0 > max_w || layout.canvas.1 > max_h) {
                let scale = f64::min(
                    max_w as f64 / layout.canvas.0 as f64,
                    max_h as f64 / layout.canvas.1 as f64,
                );
                Self::scale_layout(&mut layout, scale);
            }
        }

        // 2. Min: if canvas is below min, scale everything up proportionally.
        if let Some((min_w, min_h)) = self.min {
            if min_w > 0
                && min_h > 0
                && (layout.canvas.0 < min_w || layout.canvas.1 < min_h)
            {
                let scale = f64::max(
                    min_w as f64 / layout.canvas.0 as f64,
                    min_h as f64 / layout.canvas.1 as f64,
                );
                Self::scale_layout(&mut layout, scale);

                // Re-apply max if min pushed us past it (max wins).
                if let Some((max_w, max_h)) = self.max {
                    if max_w > 0
                        && max_h > 0
                        && (layout.canvas.0 > max_w || layout.canvas.1 > max_h)
                    {
                        let clamp = f64::min(
                            max_w as f64 / layout.canvas.0 as f64,
                            max_h as f64 / layout.canvas.1 as f64,
                        );
                        Self::scale_layout(&mut layout, clamp);
                    }
                }
            }
        }

        // 3. Align canvas to multiples.
        let encode_crop = match self.align {
            Some(Align::RoundDown(n)) if n > 1 => {
                let cw = (layout.canvas.0 / n).max(1) * n;
                let ch = (layout.canvas.1 / n).max(1) * n;
                layout.canvas = (cw, ch);

                // resize_to can't exceed canvas.
                layout.resize_to = (
                    layout.resize_to.0.min(cw),
                    layout.resize_to.1.min(ch),
                );

                // Clamp placement so image fits within canvas.
                layout.placement = (
                    layout.placement.0.min(cw.saturating_sub(layout.resize_to.0)),
                    layout.placement.1.min(ch.saturating_sub(layout.resize_to.1)),
                );
                None
            }
            Some(Align::Extend(n)) if n > 1 => {
                let (ow, oh) = layout.canvas;
                let cw = ow.div_ceil(n) * n;
                let ch = oh.div_ceil(n) * n;

                if cw != ow || ch != oh {
                    // Image at (0,0), extend right/bottom with edge replication.
                    layout.placement = (0, 0);
                    layout.canvas = (cw, ch);
                    Some((ow, oh))
                } else {
                    None // already aligned
                }
            }
            _ => None,
        };

        (layout, encode_crop)
    }

    /// Scale all layout dimensions by a factor.
    fn scale_layout(layout: &mut Layout, scale: f64) {
        layout.resize_to = (
            (layout.resize_to.0 as f64 * scale).round().max(1.0) as u32,
            (layout.resize_to.1 as f64 * scale).round().max(1.0) as u32,
        );
        layout.canvas = (
            (layout.canvas.0 as f64 * scale).round().max(1.0) as u32,
            (layout.canvas.1 as f64 * scale).round().max(1.0) as u32,
        );
        layout.placement = (
            (layout.placement.0 as f64 * scale).round() as u32,
            (layout.placement.1 as f64 * scale).round() as u32,
        );
    }
}

/// Builder for image processing pipelines.
///
/// Provides a fluent API for specifying orientation, crop, constraint, and
/// padding operations. All operations are in post-orientation coordinates
/// (what the user sees after rotation).
///
/// # Example
///
/// ```
/// use zenlayout::{Pipeline, DecoderOffer};
///
/// // EXIF-rotated JPEG, fit to 400×300
/// let (ideal, request) = Pipeline::new(4000, 3000)
///     .auto_orient(6)
///     .fit(400, 300)
///     .plan()
///     .unwrap();
///
/// // Decoder just decoded at full size
/// let plan = ideal.finalize(&request, &DecoderOffer::full_decode(4000, 3000));
/// assert!(!plan.resize_is_identity);
/// ```
#[derive(Clone, Debug)]
pub struct Pipeline {
    source_w: u32,
    source_h: u32,
    orientation: Orientation,
    crop: Option<SourceCrop>,
    constraint: Option<Constraint>,
    padding: Option<Padding>,
    limits: Option<MandatoryConstraints>,
}

impl Pipeline {
    /// Create a pipeline for a source image of the given dimensions.
    pub fn new(source_w: u32, source_h: u32) -> Self {
        Self {
            source_w,
            source_h,
            orientation: Orientation::IDENTITY,
            crop: None,
            constraint: None,
            padding: None,
            limits: None,
        }
    }

    /// Apply EXIF orientation correction (value 1-8). Invalid values are ignored.
    pub fn auto_orient(mut self, exif: u8) -> Self {
        if let Some(o) = Orientation::from_exif(exif) {
            self.orientation = self.orientation.compose(o);
        }
        self
    }

    /// Rotate 90 degrees clockwise. Stacks with EXIF and other rotations.
    pub fn rotate_90(mut self) -> Self {
        self.orientation = self.orientation.compose(Orientation::ROTATE_90);
        self
    }

    /// Rotate 180 degrees. Stacks with EXIF and other rotations.
    pub fn rotate_180(mut self) -> Self {
        self.orientation = self.orientation.compose(Orientation::ROTATE_180);
        self
    }

    /// Rotate 270 degrees clockwise. Stacks with EXIF and other rotations.
    pub fn rotate_270(mut self) -> Self {
        self.orientation = self.orientation.compose(Orientation::ROTATE_270);
        self
    }

    /// Flip horizontally. Stacks with EXIF and other orientation commands.
    pub fn flip_h(mut self) -> Self {
        self.orientation = self.orientation.compose(Orientation::FLIP_H);
        self
    }

    /// Flip vertically. Stacks with EXIF and other orientation commands.
    pub fn flip_v(mut self) -> Self {
        self.orientation = self.orientation.compose(Orientation::FLIP_V);
        self
    }

    /// Crop to pixel coordinates in post-orientation space.
    pub fn crop_pixels(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        if self.crop.is_none() {
            self.crop = Some(SourceCrop::pixels(x, y, width, height));
        }
        self
    }

    /// Crop using percentage coordinates (0.0–1.0) in post-orientation space.
    pub fn crop_percent(mut self, x: f32, y: f32, width: f32, height: f32) -> Self {
        if self.crop.is_none() {
            self.crop = Some(SourceCrop::percent(x, y, width, height));
        }
        self
    }

    /// Crop with a pre-built [`SourceCrop`].
    pub fn crop(mut self, source_crop: SourceCrop) -> Self {
        if self.crop.is_none() {
            self.crop = Some(source_crop);
        }
        self
    }

    /// Fit within target dimensions, preserving aspect ratio. May upscale.
    pub fn fit(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::Fit, width, height))
    }

    /// Fit within target dimensions, never upscaling.
    pub fn within(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::Within, width, height))
    }

    /// Scale to fill target, cropping overflow. Preserves aspect ratio.
    pub fn fit_crop(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::FitCrop, width, height))
    }

    /// Like [`fit_crop`](Self::fit_crop), but never upscales.
    pub fn within_crop(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::WithinCrop, width, height))
    }

    /// Fit within target, padding to exact target dimensions.
    pub fn fit_pad(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::FitPad, width, height))
    }

    /// Like [`fit_pad`](Self::fit_pad), but never upscales.
    pub fn within_pad(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::WithinPad, width, height))
    }

    /// Scale to exact target dimensions, distorting aspect ratio.
    pub fn distort(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::Distort, width, height))
    }

    /// Crop to target aspect ratio without scaling.
    pub fn aspect_crop(self, width: u32, height: u32) -> Self {
        self.constrain(Constraint::new(ConstraintMode::AspectCrop, width, height))
    }

    /// Apply a pre-built [`Constraint`] for advanced cases (gravity, canvas color, single-axis).
    pub fn constrain(mut self, constraint: Constraint) -> Self {
        if self.constraint.is_none() {
            self.constraint = Some(constraint);
        }
        self
    }

    /// Add uniform padding on all sides.
    pub fn pad_uniform(self, amount: u32, color: CanvasColor) -> Self {
        self.pad(amount, amount, amount, amount, color)
    }

    /// Add padding around the image.
    pub fn pad(mut self, top: u32, right: u32, bottom: u32, left: u32, color: CanvasColor) -> Self {
        if self.padding.is_none() {
            self.padding = Some(Padding {
                top,
                right,
                bottom,
                left,
                color,
            });
        }
        self
    }

    /// Apply safety limits after layout computation.
    ///
    /// See [`MandatoryConstraints`] for details on max/min/align behavior.
    pub fn limits(mut self, limits: MandatoryConstraints) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Compute the ideal layout and decoder request.
    ///
    /// This is phase 1 of the two-phase layout process. Pass the returned
    /// [`DecoderRequest`] to the decoder, then call [`IdealLayout::finalize()`]
    /// with the decoder's [`DecoderOffer`].
    pub fn plan(self) -> Result<(IdealLayout, DecoderRequest), LayoutError> {
        plan_from_parts(
            self.source_w,
            self.source_h,
            self.orientation,
            self.crop.as_ref(),
            self.constraint.as_ref(),
            self.padding,
            self.limits.as_ref(),
        )
    }
}

impl IdealLayout {
    /// Finalize layout after decoder reports what it actually did.
    ///
    /// Convenience method — equivalent to calling [`finalize()`].
    pub fn finalize(&self, request: &DecoderRequest, offer: &DecoderOffer) -> LayoutPlan {
        finalize(self, request, offer)
    }

    /// Derive an `(IdealLayout, DecoderRequest)` for a secondary plane that must
    /// stay spatially locked with the primary plane.
    ///
    /// Use this for gain maps, depth maps, alpha planes, or any auxiliary image
    /// that shares spatial extent with the primary image but lives at a different
    /// resolution and is decoded independently.
    ///
    /// The secondary plane goes through the same two-phase negotiation as the
    /// primary: send the `DecoderRequest` to the secondary decoder, get back a
    /// `DecoderOffer`, and call [`finalize()`] to compute remaining work.
    /// Each decoder independently handles what it can; `finalize()` compensates.
    ///
    /// # Arguments
    ///
    /// * `primary_source` — Source dimensions of the primary plane (before orientation).
    /// * `secondary_source` — Source dimensions of the secondary plane.
    /// * `secondary_target` — Desired output dimensions for the secondary plane.
    ///   Pass `None` to automatically maintain the source ratio
    ///   (e.g., if gain map is 1/4 of SDR source, output will be 1/4 of SDR output).
    ///
    /// # Coordinate scaling
    ///
    /// Source crop coordinates are scaled from primary to secondary space with
    /// **round-outward** logic: origin floors, extent ceils. This ensures the
    /// secondary plane always covers at least the full spatial extent of the
    /// primary crop. The renderer handles any sub-pixel offset.
    ///
    /// # Example
    ///
    /// ```
    /// use zenlayout::{Pipeline, DecoderOffer};
    ///
    /// // SDR: 4000×3000, gain map: 1000×750 (1/4 scale)
    /// let (sdr_ideal, sdr_req) = Pipeline::new(4000, 3000)
    ///     .auto_orient(6)
    ///     .crop_pixels(100, 100, 2000, 2000)
    ///     .fit(800, 800)
    ///     .plan()
    ///     .unwrap();
    ///
    /// // Derive gain map plan from SDR plan
    /// let (gm_ideal, gm_req) = sdr_ideal.derive_secondary(
    ///     (4000, 3000),     // primary source
    ///     (1000, 750),      // gain map source
    ///     None,             // auto: 1/4 of SDR output
    /// );
    ///
    /// // Each decoder independently does its thing
    /// let sdr_plan = sdr_ideal.finalize(&sdr_req, &DecoderOffer::full_decode(4000, 3000));
    /// let gm_plan = gm_ideal.finalize(&gm_req, &DecoderOffer::full_decode(1000, 750));
    ///
    /// // Both plans produce spatially-locked results
    /// assert_eq!(sdr_plan.remaining_orientation, gm_plan.remaining_orientation);
    /// ```
    pub fn derive_secondary(
        &self,
        primary_source: (u32, u32),
        secondary_source: (u32, u32),
        secondary_target: Option<(u32, u32)>,
    ) -> (IdealLayout, DecoderRequest) {
        let (ps_w, ps_h) = primary_source;
        let (ss_w, ss_h) = secondary_source;

        // Scale ratios from primary source to secondary source.
        let scale_x = ss_w as f64 / ps_w as f64;
        let scale_y = ss_h as f64 / ps_h as f64;

        // Scale the source crop (in pre-orientation coords) with round-outward.
        let secondary_crop = self.source_crop.map(|crop| {
            scale_rect_outward(crop, scale_x, scale_y, ss_w, ss_h)
        });

        // Compute the oriented secondary source dimensions.
        let (sec_ow, sec_oh) = self.orientation.transform_dimensions(ss_w, ss_h);

        // Scale the layout's source crop (in post-orientation coords) with round-outward.
        // Use the oriented scale factors.
        let orient_scale_x = sec_ow as f64 / self.layout.source.0 as f64;
        let orient_scale_y = sec_oh as f64 / self.layout.source.1 as f64;

        let secondary_layout_crop = self.layout.source_crop.map(|crop| {
            scale_rect_outward(crop, orient_scale_x, orient_scale_y, sec_ow, sec_oh)
        });

        // Effective source after crop in oriented space.
        let (eff_w, eff_h) = match &secondary_layout_crop {
            Some(r) => (r.width, r.height),
            None => (sec_ow, sec_oh),
        };

        // Compute target dimensions for the secondary plane.
        let (target_w, target_h) = match secondary_target {
            Some(t) => t,
            None => {
                // Auto: maintain source ratio relative to primary output.
                let (pri_rw, pri_rh) = self.layout.resize_to;
                let tw = (pri_rw as f64 * scale_x).round().max(1.0) as u32;
                let th = (pri_rh as f64 * scale_y).round().max(1.0) as u32;
                (tw, th)
            }
        };

        let sec_layout = Layout {
            source: (sec_ow, sec_oh),
            source_crop: secondary_layout_crop,
            resize_to: (target_w, target_h),
            canvas: (target_w, target_h),
            placement: (0, 0),
            canvas_color: CanvasColor::default(),
        };

        // Effective source is the crop region (or full secondary if no crop).
        // resize_is_identity will be computed by finalize().
        let _ = (eff_w, eff_h);

        let sec_ideal = IdealLayout {
            orientation: self.orientation,
            layout: sec_layout,
            source_crop: secondary_crop,
            padding: None, // secondary planes don't get padded
            encode_crop: None,
        };

        let sec_request = DecoderRequest {
            crop: secondary_crop,
            prescale_target: (target_w, target_h),
            orientation: self.orientation,
        };

        (sec_ideal, sec_request)
    }
}

/// Scale a rect from one coordinate space to another, rounding outward.
///
/// Origin (x, y) is floored, far edge (x+w, y+h) is ceiled, then clamped
/// to the target dimensions. This ensures the scaled rect always covers
/// at least the full spatial extent of the original.
fn scale_rect_outward(rect: Rect, scale_x: f64, scale_y: f64, max_w: u32, max_h: u32) -> Rect {
    let x0 = (rect.x as f64 * scale_x).floor() as u32;
    let y0 = (rect.y as f64 * scale_y).floor() as u32;
    let x1 = ((rect.x + rect.width) as f64 * scale_x).ceil().min(max_w as f64) as u32;
    let y1 = ((rect.y + rect.height) as f64 * scale_y).ceil().min(max_h as f64) as u32;
    Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

/// Compute ideal layout from commands and source image dimensions.
///
/// Orientation commands (AutoOrient, Rotate, Flip) are composed into a single
/// net orientation. Crop and Constrain operate in post-orientation coordinates
/// (what the user sees after rotation). The resulting source crop is transformed
/// back to pre-orientation source coordinates for the decoder.
///
/// Only the first `Crop` and first `Constrain` command are used; duplicates are ignored.
///
/// For a friendlier API, see [`Pipeline`].
pub fn plan(
    commands: &[Command],
    source_w: u32,
    source_h: u32,
    limits: Option<&MandatoryConstraints>,
) -> Result<(IdealLayout, DecoderRequest), LayoutError> {
    let mut orientation = Orientation::IDENTITY;
    let mut crop: Option<&SourceCrop> = None;
    let mut constraint: Option<&Constraint> = None;
    let mut padding: Option<Padding> = None;

    for cmd in commands {
        match cmd {
            Command::AutoOrient(exif) => {
                if let Some(o) = Orientation::from_exif(*exif) {
                    orientation = orientation.compose(o);
                }
            }
            Command::Rotate(r) => {
                let o = match r {
                    Rotation::Rotate90 => Orientation::ROTATE_90,
                    Rotation::Rotate180 => Orientation::ROTATE_180,
                    Rotation::Rotate270 => Orientation::ROTATE_270,
                };
                orientation = orientation.compose(o);
            }
            Command::Flip(axis) => {
                let o = match axis {
                    FlipAxis::Horizontal => Orientation::FLIP_H,
                    FlipAxis::Vertical => Orientation::FLIP_V,
                };
                orientation = orientation.compose(o);
            }
            Command::Crop(c) => {
                if crop.is_none() {
                    crop = Some(c);
                }
            }
            Command::Constrain { constraint: c } => {
                if constraint.is_none() {
                    constraint = Some(c);
                }
            }
            Command::Pad {
                top,
                right,
                bottom,
                left,
                color,
            } => {
                if padding.is_none() {
                    padding = Some(Padding {
                        top: *top,
                        right: *right,
                        bottom: *bottom,
                        left: *left,
                        color: *color,
                    });
                }
            }
        }
    }

    plan_from_parts(source_w, source_h, orientation, crop, constraint, padding, limits)
}

/// Core layout computation shared by [`plan()`] and [`Pipeline::plan()`].
fn plan_from_parts(
    source_w: u32,
    source_h: u32,
    orientation: Orientation,
    crop: Option<&SourceCrop>,
    constraint: Option<&Constraint>,
    padding: Option<Padding>,
    limits: Option<&MandatoryConstraints>,
) -> Result<(IdealLayout, DecoderRequest), LayoutError> {
    if source_w == 0 || source_h == 0 {
        return Err(LayoutError::ZeroSourceDimension);
    }

    // 1. Transform source dimensions to post-orientation space.
    let (ow, oh) = orientation.transform_dimensions(source_w, source_h);

    // 2. Compute layout in post-orientation space.
    let layout = if let Some(c) = constraint {
        let mut builder = c.clone();
        if let Some(sc) = crop {
            builder = builder.source_crop(*sc);
        }
        builder.compute(ow, oh)?
    } else if let Some(sc) = crop {
        let rect = sc.resolve(ow, oh);
        Layout {
            source: (ow, oh),
            source_crop: Some(rect),
            resize_to: (rect.width, rect.height),
            canvas: (rect.width, rect.height),
            placement: (0, 0),
            canvas_color: CanvasColor::default(),
        }
    } else {
        Layout {
            source: (ow, oh),
            source_crop: None,
            resize_to: (ow, oh),
            canvas: (ow, oh),
            placement: (0, 0),
            canvas_color: CanvasColor::default(),
        }
    };

    // 3. Apply explicit padding if present (additive on existing canvas).
    let layout = if let Some(pad) = &padding {
        Layout {
            canvas: (
                layout.canvas.0 + pad.left + pad.right,
                layout.canvas.1 + pad.top + pad.bottom,
            ),
            placement: (
                layout.placement.0 + pad.left,
                layout.placement.1 + pad.top,
            ),
            canvas_color: pad.color,
            ..layout
        }
    } else {
        layout
    };

    // 4. Apply mandatory constraints (max/min/align).
    let (layout, encode_crop) = if let Some(mc) = limits {
        mc.apply(layout)
    } else {
        (layout, None)
    };

    // 5. Transform source crop back to pre-orientation source coordinates.
    let source_crop_in_source = layout
        .source_crop
        .map(|r| orientation.transform_rect_to_source(r, source_w, source_h));

    let ideal = IdealLayout {
        orientation,
        layout: layout.clone(),
        source_crop: source_crop_in_source,
        padding,
        encode_crop,
    };

    let request = DecoderRequest {
        crop: source_crop_in_source,
        prescale_target: layout.resize_to,
        orientation,
    };

    Ok((ideal, request))
}

/// Finalize layout after decoder reports what it actually did.
///
/// Given the ideal layout from [`plan()`] and the decoder's [`DecoderOffer`],
/// compute the remaining work: trim, resize, orientation, and canvas placement.
pub fn finalize(ideal: &IdealLayout, request: &DecoderRequest, offer: &DecoderOffer) -> LayoutPlan {
    // 1. Remaining orientation = undo what decoder did, then apply full orientation.
    let remaining_orientation = offer
        .orientation_applied
        .inverse()
        .compose(ideal.orientation);

    // 2. Compute trim rect if decoder didn't crop exactly what we asked.
    let (decoder_w, decoder_h) = offer.dimensions;
    let trim = compute_trim(&request.crop, &offer.crop_applied, decoder_w, decoder_h);

    // 3. Dimensions after trimming.
    let (after_trim_w, after_trim_h) = match &trim {
        Some(r) => (r.width, r.height),
        None => (decoder_w, decoder_h),
    };

    // 4. Dimensions after remaining orientation.
    let (after_orient_w, after_orient_h) =
        remaining_orientation.transform_dimensions(after_trim_w, after_trim_h);

    // 5. Target resize dimensions from the ideal layout.
    let (target_w, target_h) = ideal.layout.resize_to;

    // 6. Determine if resize is identity.
    let resize_is_identity = after_orient_w == target_w && after_orient_h == target_h;

    LayoutPlan {
        decoder_request: request.clone(),
        trim,
        resize_to: (target_w, target_h),
        remaining_orientation,
        canvas: ideal.layout.canvas,
        placement: ideal.layout.placement,
        canvas_color: ideal.layout.canvas_color,
        resize_is_identity,
        encode_crop: ideal.encode_crop,
    }
}

/// Compute trim rect when decoder crop doesn't exactly match request.
fn compute_trim(
    requested_crop: &Option<Rect>,
    applied_crop: &Option<Rect>,
    decoder_w: u32,
    decoder_h: u32,
) -> Option<Rect> {
    match (requested_crop, applied_crop) {
        // We asked for crop, decoder did nothing → trim the full decode to the requested region.
        (Some(req_crop), None) => Some(*req_crop),
        // We asked for crop, decoder cropped but not exactly → compute offset within decoder output.
        (Some(req_crop), Some(applied)) => {
            if req_crop == applied {
                // Exact match — no trim needed.
                None
            } else {
                // Decoder cropped a superset (e.g., block-aligned).
                // Trim within the decoder's output to get just the region we wanted.
                let dx = req_crop.x.saturating_sub(applied.x);
                let dy = req_crop.y.saturating_sub(applied.y);
                let tw = req_crop.width.min(decoder_w.saturating_sub(dx));
                let th = req_crop.height.min(decoder_h.saturating_sub(dy));
                if dx == 0 && dy == 0 && tw == decoder_w && th == decoder_h {
                    None
                } else {
                    Some(Rect::new(dx, dy, tw, th))
                }
            }
        }
        // No crop requested — no trim needed.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint::Gravity;

    // ── No commands ──────────────────────────────────────────────────────

    #[test]
    fn empty_commands_passthrough() {
        let (ideal, req) = plan(&[], 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::IDENTITY);
        assert_eq!(ideal.layout.resize_to, (800, 600));
        assert_eq!(ideal.layout.canvas, (800, 600));
        assert!(ideal.source_crop.is_none());
        assert!(ideal.padding.is_none());
        assert!(req.crop.is_none());
        assert_eq!(req.prescale_target, (800, 600));
    }

    #[test]
    fn zero_source_rejected() {
        assert!(plan(&[], 0, 600, None).is_err());
        assert!(plan(&[], 800, 0, None).is_err());
    }

    // ── Orientation only ─────────────────────────────────────────────────

    #[test]
    fn auto_orient_90_swaps_dims() {
        let commands = [Command::AutoOrient(6)]; // EXIF 6 = Rotate90
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_90);
        // Post-orientation: 800×600 rotated 90° → 600×800
        assert_eq!(ideal.layout.resize_to, (600, 800));
        assert_eq!(ideal.layout.canvas, (600, 800));
        assert_eq!(req.orientation, Orientation::ROTATE_90);
    }

    #[test]
    fn auto_orient_180_preserves_dims() {
        let commands = [Command::AutoOrient(3)]; // EXIF 3 = Rotate180
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn stacked_orientation() {
        // EXIF 6 (Rotate90) + manual Rotate90 = Rotate180
        let commands = [Command::AutoOrient(6), Command::Rotate(Rotation::Rotate90)];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        // 180° doesn't swap: still 800×600
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn flip_horizontal() {
        let commands = [Command::Flip(FlipAxis::Horizontal)];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::FLIP_H);
        // FlipH doesn't change dimensions
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn invalid_exif_ignored() {
        let commands = [Command::AutoOrient(0), Command::AutoOrient(9)];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::IDENTITY);
    }

    // ── Crop in oriented space ───────────────────────────────────────────

    #[test]
    fn crop_in_oriented_space() {
        // Rotate 90°: 800×600 → oriented 600×800
        // Crop 100,100,400,600 in oriented space
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(100, 100, 400, 600)),
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        // Layout crop is in oriented space
        let layout_crop = ideal.layout.source_crop.unwrap();
        assert_eq!(layout_crop, Rect::new(100, 100, 400, 600));

        // Source crop is transformed back to source coordinates
        let source_crop = ideal.source_crop.unwrap();
        assert_eq!(source_crop, req.crop.unwrap());
        // Verify dimensions make sense — rotated rect should have swapped w/h
        assert_eq!(source_crop.width, 600);
        assert_eq!(source_crop.height, 400);
    }

    #[test]
    fn crop_only_no_constraint() {
        let commands = [Command::Crop(SourceCrop::pixels(10, 20, 100, 200))];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (100, 200));
        assert_eq!(ideal.layout.canvas, (100, 200));
        let crop = ideal.source_crop.unwrap();
        assert_eq!(crop, Rect::new(10, 20, 100, 200));
    }

    // ── Constrain after orientation ──────────────────────────────────────

    #[test]
    fn constrain_after_rotate90() {
        // 800×600 rotated 90° → 600×800 oriented, then fit to 300×300
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 300, 300),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        // Fit 600×800 into 300×300 → 225×300
        assert_eq!(ideal.layout.resize_to, (225, 300));
        assert_eq!(req.prescale_target, (225, 300));
    }

    #[test]
    fn constrain_with_crop() {
        // Crop to 400×400, then fit to 200×200
        let commands = [
            Command::Crop(SourceCrop::pixels(100, 50, 400, 400)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
        ];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (200, 200));
        // Source crop should be present (from the explicit crop)
        assert!(ideal.source_crop.is_some());
    }

    // ── Pad command ──────────────────────────────────────────────────────

    #[test]
    fn pad_expands_canvas() {
        let commands = [Command::Pad {
            top: 10,
            right: 20,
            bottom: 10,
            left: 20,
            color: CanvasColor::white(),
        }];
        let (ideal, _) = plan(&commands, 400, 300, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 300));
        assert_eq!(ideal.layout.canvas, (440, 320));
        assert_eq!(ideal.layout.placement, (20, 10));
        assert!(ideal.padding.is_some());
        let pad = ideal.padding.unwrap();
        assert_eq!(pad.top, 10);
        assert_eq!(pad.left, 20);
    }

    #[test]
    fn pad_after_constrain() {
        let commands = [
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
            Command::Pad {
                top: 5,
                right: 5,
                bottom: 5,
                left: 5,
                color: CanvasColor::black(),
            },
        ];
        let (ideal, _) = plan(&commands, 800, 400, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (200, 100));
        assert_eq!(ideal.layout.canvas, (210, 110));
        assert_eq!(ideal.layout.placement, (5, 5));
    }

    // ── finalize with full_decode ────────────────────────────────────────

    #[test]
    fn finalize_full_decode_no_orientation() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 400, 300),
        }];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);

        assert!(plan.trim.is_none());
        assert_eq!(plan.resize_to, (400, 300));
        assert_eq!(plan.remaining_orientation, Orientation::IDENTITY);
        assert_eq!(plan.canvas, (400, 300));
        assert!(!plan.resize_is_identity);
    }

    #[test]
    fn finalize_full_decode_with_orientation() {
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 300, 300),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);

        assert_eq!(plan.remaining_orientation, Orientation::ROTATE_90);
        assert!(plan.trim.is_none());
    }

    #[test]
    fn finalize_decoder_handles_orientation() {
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 300, 300),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        // Decoder applied the rotation itself
        let offer = DecoderOffer {
            dimensions: (600, 800),
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_90,
        };
        let plan = finalize(&ideal, &req, &offer);

        assert_eq!(plan.remaining_orientation, Orientation::IDENTITY);
    }

    #[test]
    fn finalize_decoder_partial_crop() {
        // Request crop of 100,100,200,200, decoder cropped wider (block-aligned)
        let commands = [Command::Crop(SourceCrop::pixels(100, 100, 200, 200))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        assert_eq!(req.crop, Some(Rect::new(100, 100, 200, 200)));

        let offer = DecoderOffer {
            dimensions: (208, 208),
            crop_applied: Some(Rect::new(96, 96, 208, 208)),
            orientation_applied: Orientation::IDENTITY,
        };
        let plan = finalize(&ideal, &req, &offer);

        // Should trim to get the exact region we wanted
        let trim = plan.trim.unwrap();
        assert_eq!(trim.x, 4); // 100 - 96
        assert_eq!(trim.y, 4); // 100 - 96
        assert_eq!(trim.width, 200);
        assert_eq!(trim.height, 200);
    }

    #[test]
    fn finalize_decoder_no_crop_when_requested() {
        // We asked for crop, decoder gave full image → trim = crop rect
        let commands = [Command::Crop(SourceCrop::pixels(100, 100, 200, 200))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);

        let trim = plan.trim.unwrap();
        assert_eq!(trim, Rect::new(100, 100, 200, 200));
    }

    // ── resize_is_identity ───────────────────────────────────────────────

    #[test]
    fn resize_identity_crop_only() {
        let commands = [Command::Crop(SourceCrop::pixels(0, 0, 400, 300))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer {
            dimensions: (400, 300),
            crop_applied: Some(Rect::new(0, 0, 400, 300)),
            orientation_applied: Orientation::IDENTITY,
        };
        let plan = finalize(&ideal, &req, &offer);
        assert!(plan.resize_is_identity);
    }

    #[test]
    fn resize_identity_rotate_only() {
        // Just rotate, no resize
        let commands = [Command::AutoOrient(6)];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);

        // After orientation (90°): 800×600 → 600×800
        // Target resize_to is (600, 800)
        // Decoder output is 800×600, remaining_orientation is Rotate90
        // After orient: 600×800 == target → identity
        assert!(plan.resize_is_identity);
    }

    #[test]
    fn resize_not_identity_when_scaling() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 400, 300),
        }];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);
        assert!(!plan.resize_is_identity);
    }

    // ── Lossless scenario ────────────────────────────────────────────────

    #[test]
    fn lossless_rotate_and_crop() {
        // JPEG lossless scenario: rotate 90° + crop, decoder handles both
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(0, 0, 300, 400)),
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        // oriented = 600×800, crop 0,0,300,400 in oriented space
        assert_eq!(ideal.layout.resize_to, (300, 400));

        // Decoder handles orientation and crop
        let offer = DecoderOffer {
            dimensions: (300, 400),
            crop_applied: req.crop,
            orientation_applied: Orientation::ROTATE_90,
        };
        let plan = finalize(&ideal, &req, &offer);
        assert!(plan.resize_is_identity);
        assert_eq!(plan.remaining_orientation, Orientation::IDENTITY);
        assert!(plan.trim.is_none());
    }

    // ── Only first crop/constraint used ──────────────────────────────────

    #[test]
    fn duplicate_commands_use_first() {
        let commands = [
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
            },
        ];
        let (ideal, _) = plan(&commands, 800, 600, None).unwrap();
        // First constraint wins: Fit to 200×200
        assert_eq!(ideal.layout.resize_to, (200, 150));
    }

    // ════════════════════════════════════════════════════════════════════
    // Weird decoder behavior
    // ════════════════════════════════════════════════════════════════════

    /// Helper: plan + finalize in one step for concise tests.
    fn plan_finalize(
        commands: &[Command],
        source_w: u32,
        source_h: u32,
        offer: &DecoderOffer,
    ) -> (IdealLayout, LayoutPlan) {
        let (ideal, req) = plan(commands, source_w, source_h, None).unwrap();
        let lp = finalize(&ideal, &req, offer);
        (ideal, lp)
    }

    // ── Decoder prescaling (JPEG 1/2, 1/4, 1/8) ─────────────────────

    #[test]
    fn decoder_prescale_half() {
        // Request: fit 4000×3000 to 500×500, decoder prescales to 2000×1500
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
        }];
        let (ideal, req) = plan(&commands, 4000, 3000, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (500, 375));

        let offer = DecoderOffer {
            dimensions: (2000, 1500),
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert!(lp.trim.is_none());
        assert_eq!(lp.resize_to, (500, 375));
        // 2000×1500 → 500×375: still needs resize
        assert!(!lp.resize_is_identity);
    }

    #[test]
    fn decoder_prescale_to_exact_target() {
        // JPEG decoder prescales to exactly the target size — no resize needed
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 500, 375),
        }];
        let (ideal, req) = plan(&commands, 4000, 3000, None).unwrap();
        let offer = DecoderOffer {
            dimensions: (500, 375),
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);
        assert!(lp.resize_is_identity);
    }

    #[test]
    fn decoder_prescale_eighth() {
        // 1/8 prescale: 4000×3000 → 500×375, matches target exactly
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
        }];
        let (_, req) = plan(&commands, 4000, 3000, None).unwrap();
        // Decoder only managed 1/8 but dimensions don't match target
        let offer = DecoderOffer {
            dimensions: (500, 375),
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        };
        let (_, lp) = plan_finalize(
            &[Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
            }],
            4000,
            3000,
            &offer,
        );
        // target is 500×375, decoder output is 500×375 → identity
        assert!(lp.resize_is_identity);
        assert_eq!(lp.resize_to, (500, 375));
        let _ = req; // used above
    }

    // ── Block-aligned crop overshoot ─────────────────────────────────

    #[test]
    fn decoder_crop_mcu_aligned_16x16() {
        // JPEG MCU is 16×16. Request crop at (103,47,200,200).
        // Decoder aligns to (96,32,224,224).
        let commands = [Command::Crop(SourceCrop::pixels(103, 47, 200, 200))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(req.crop.unwrap(), Rect::new(103, 47, 200, 200));

        let offer = DecoderOffer {
            dimensions: (224, 224),
            crop_applied: Some(Rect::new(96, 32, 224, 224)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        let trim = lp.trim.unwrap();
        assert_eq!(trim.x, 7); // 103 - 96
        assert_eq!(trim.y, 15); // 47 - 32
        assert_eq!(trim.width, 200);
        assert_eq!(trim.height, 200);
        assert!(lp.resize_is_identity); // crop-only = no resize
    }

    #[test]
    fn decoder_crop_mcu_aligned_8x8() {
        // 8×8 MCU alignment: request (50,50,100,100), decoder gives (48,48,104,104)
        let commands = [Command::Crop(SourceCrop::pixels(50, 50, 100, 100))];
        let (ideal, req) = plan(&commands, 400, 300, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (104, 104),
            crop_applied: Some(Rect::new(48, 48, 104, 104)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        let trim = lp.trim.unwrap();
        assert_eq!(trim, Rect::new(2, 2, 100, 100));
        assert!(lp.resize_is_identity);
    }

    #[test]
    fn decoder_crop_at_image_edge_truncated() {
        // Request crop near edge: (700,500,200,200) in 800×600.
        // Decoder crops (696,496,104,104) — truncated at image boundary.
        let commands = [Command::Crop(SourceCrop::pixels(700, 500, 100, 100))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (104, 104),
            crop_applied: Some(Rect::new(696, 496, 104, 104)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        let trim = lp.trim.unwrap();
        assert_eq!(trim.x, 4); // 700 - 696
        assert_eq!(trim.y, 4); // 500 - 496
        assert_eq!(trim.width, 100);
        assert_eq!(trim.height, 100);
    }

    // ── Decoder applies wrong orientation ────────────────────────────

    #[test]
    fn decoder_applies_wrong_orientation() {
        // We want Rotate90 (EXIF 6), decoder applied Rotate180 instead
        let commands = [Command::AutoOrient(6)];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_90);

        let offer = DecoderOffer {
            dimensions: (800, 600), // 180° doesn't swap
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_180,
        };
        let lp = finalize(&ideal, &req, &offer);

        // remaining = inverse(180°) ∘ 90° = 180° ∘ 90° = 270°
        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_270);
        // After remaining 270° on 800×600 → 600×800
        // Target was 600×800 (from 90° of 800×600)
        assert_eq!(lp.resize_to, (600, 800));
        assert!(lp.resize_is_identity);
    }

    #[test]
    fn decoder_applies_flip_instead_of_rotate() {
        // We want Rotate90, decoder applied FlipH
        let commands = [Command::AutoOrient(6)];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (800, 600), // FlipH doesn't swap
            crop_applied: None,
            orientation_applied: Orientation::FLIP_H,
        };
        let lp = finalize(&ideal, &req, &offer);

        // remaining = inverse(FlipH) ∘ Rotate90 = FlipH ∘ Rotate90 = Transverse
        assert_eq!(lp.remaining_orientation, Orientation::TRANSVERSE);
        // Transpose swaps axes: 800×600 → 600×800 = target
        assert!(lp.resize_is_identity);
    }

    // ── Decoder applies partial orientation ──────────────────────────

    #[test]
    fn decoder_partial_orientation_flip_only() {
        // We want Transverse (EXIF 7 = rot270 + flip), decoder only flipped
        let commands = [Command::AutoOrient(7)];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.orientation, Orientation::TRANSVERSE);

        let offer = DecoderOffer {
            dimensions: (800, 600),
            crop_applied: None,
            orientation_applied: Orientation::FLIP_H,
        };
        let lp = finalize(&ideal, &req, &offer);

        // remaining = inverse(FlipH) ∘ Transverse = FlipH ∘ Transverse
        let expected = Orientation::FLIP_H.compose(Orientation::TRANSVERSE);
        assert_eq!(lp.remaining_orientation, expected);
    }

    // ── Decoder crops AND orients simultaneously ─────────────────────

    #[test]
    fn decoder_crop_and_orient_simultaneously() {
        // Rotate90 + crop in oriented space → decoder handles both
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(50, 50, 200, 300)),
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        // Decoder did everything: oriented + cropped
        let offer = DecoderOffer {
            dimensions: (200, 300),
            crop_applied: req.crop,
            orientation_applied: Orientation::ROTATE_90,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert!(lp.trim.is_none());
        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        assert!(lp.resize_is_identity);
        assert_eq!(lp.resize_to, (200, 300));
    }

    #[test]
    fn decoder_orients_but_not_crops() {
        // Rotate90 + crop. Decoder handles rotation but ignores crop.
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(50, 50, 200, 300)),
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        // Decoder rotated (swapped dims) but didn't crop
        let offer = DecoderOffer {
            dimensions: (600, 800),
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_90,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        // Should still have a trim for the requested crop (now in source coords)
        assert!(lp.trim.is_some());
        let trim = lp.trim.unwrap();
        let rc = req.crop.unwrap();
        assert_eq!((trim.width, trim.height), (rc.width, rc.height));
    }

    #[test]
    fn decoder_crops_but_not_orients() {
        // Rotate90 + crop. Decoder crops (in source coords) but doesn't rotate.
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(50, 50, 200, 300)),
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let source_crop = req.crop.unwrap();

        // Decoder cropped exactly in source coords but didn't orient
        let offer = DecoderOffer {
            dimensions: (source_crop.width, source_crop.height),
            crop_applied: Some(source_crop),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert!(lp.trim.is_none()); // crop was exact
        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_90);
        // After remaining 90° on cropped dims → should match target
        let (after_w, after_h) = lp
            .remaining_orientation
            .transform_dimensions(source_crop.width, source_crop.height);
        assert_eq!((after_w, after_h), lp.resize_to);
        assert!(lp.resize_is_identity);
    }

    // ── Decoder ignores everything ───────────────────────────────────

    #[test]
    fn decoder_ignores_everything_complex_pipeline() {
        // Full pipeline: EXIF 5 (Transpose) + crop + constrain + pad
        // Decoder does nothing.
        let commands = [
            Command::AutoOrient(5),
            Command::Crop(SourceCrop::pixels(10, 10, 200, 300)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
            },
            Command::Pad {
                top: 5,
                right: 5,
                bottom: 5,
                left: 5,
                color: CanvasColor::black(),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let lp = finalize(&ideal, &req, &offer);

        // Full orientation remains
        assert_eq!(lp.remaining_orientation, Orientation::TRANSPOSE);
        // Decoder output is full 800×600, needs crop → trim present
        assert!(lp.trim.is_some());
        assert!(!lp.resize_is_identity);
        // Canvas includes padding
        assert!(lp.canvas.0 > lp.resize_to.0);
        assert!(lp.canvas.1 > lp.resize_to.1);
    }

    // ── All 8 EXIF orientations: decoder handles vs doesn't ──────────

    #[test]
    fn all_8_orientations_decoder_handles() {
        for exif in 1..=8u8 {
            let orientation = Orientation::from_exif(exif).unwrap();
            let commands = [
                Command::AutoOrient(exif),
                Command::Constrain {
                    constraint: Constraint::new(ConstraintMode::Fit, 300, 300),
                },
            ];
            let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

            // Decoder applied the orientation
            let (dw, dh) = orientation.transform_dimensions(800, 600);
            let offer = DecoderOffer {
                dimensions: (dw, dh),
                crop_applied: None,
                orientation_applied: orientation,
            };
            let lp = finalize(&ideal, &req, &offer);

            assert_eq!(
                lp.remaining_orientation,
                Orientation::IDENTITY,
                "EXIF {exif}: remaining should be identity when decoder handled it"
            );
            assert!(lp.trim.is_none());
        }
    }

    #[test]
    fn all_8_orientations_decoder_ignores() {
        for exif in 1..=8u8 {
            let orientation = Orientation::from_exif(exif).unwrap();
            let commands = [Command::AutoOrient(exif)];
            let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

            // Decoder did nothing
            let offer = DecoderOffer::full_decode(800, 600);
            let lp = finalize(&ideal, &req, &offer);

            assert_eq!(
                lp.remaining_orientation, orientation,
                "EXIF {exif}: remaining should be the full orientation"
            );
            // For orient-only, after remaining orient the dims should match
            let (after_w, after_h) = lp.remaining_orientation.transform_dimensions(800, 600);
            assert_eq!(
                (after_w, after_h),
                lp.resize_to,
                "EXIF {exif}: post-orient dims should match resize target"
            );
            assert!(
                lp.resize_is_identity,
                "EXIF {exif}: orient-only is identity"
            );
        }
    }

    // ── 1×1 pixel edge cases ─────────────────────────────────────────

    #[test]
    fn one_pixel_image_passthrough() {
        let (_, lp) = plan_finalize(&[], 1, 1, &DecoderOffer::full_decode(1, 1));
        assert!(lp.resize_is_identity);
        assert_eq!(lp.resize_to, (1, 1));
        assert_eq!(lp.canvas, (1, 1));
    }

    #[test]
    fn one_pixel_image_with_rotation() {
        let commands = [Command::AutoOrient(6)]; // Rotate90
        let (_, lp) = plan_finalize(&commands, 1, 1, &DecoderOffer::full_decode(1, 1));
        // 1×1 rotated is still 1×1
        assert!(lp.resize_is_identity);
        assert_eq!(lp.resize_to, (1, 1));
    }

    #[test]
    fn one_pixel_image_with_fit() {
        // Fit upscales: 1×1 → 100×100
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
        }];
        let (_, lp) = plan_finalize(&commands, 1, 1, &DecoderOffer::full_decode(1, 1));
        assert_eq!(lp.resize_to, (100, 100));
        assert!(!lp.resize_is_identity);
    }

    #[test]
    fn one_pixel_image_with_within() {
        // Within never upscales: 1×1 stays 1×1
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Within, 100, 100),
        }];
        let (_, lp) = plan_finalize(&commands, 1, 1, &DecoderOffer::full_decode(1, 1));
        assert_eq!(lp.resize_to, (1, 1));
        assert!(lp.resize_is_identity);
    }

    // ── Decoder prescales with orientation ────────────────────────────

    #[test]
    fn decoder_prescale_with_orientation_handled() {
        // 4000×3000, EXIF 6 (Rotate90), fit to 500×500
        // Decoder prescales 1/4 AND handles rotation → delivers 750×1000
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
            },
        ];
        let (ideal, req) = plan(&commands, 4000, 3000, None).unwrap();
        // Oriented: 3000×4000, fit to 500×500 → 375×500
        assert_eq!(ideal.layout.resize_to, (375, 500));

        let offer = DecoderOffer {
            dimensions: (750, 1000), // 1/4 prescale + rotation
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_90,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        assert_eq!(lp.resize_to, (375, 500));
        assert!(!lp.resize_is_identity); // 750×1000 → 375×500
    }

    #[test]
    fn decoder_prescale_without_orientation() {
        // 4000×3000, EXIF 6 (Rotate90), fit to 500×500
        // Decoder prescales 1/4 but doesn't rotate → delivers 1000×750
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
            },
        ];
        let (ideal, req) = plan(&commands, 4000, 3000, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (1000, 750), // 1/4 prescale, no rotation
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_90);
        // After 90° on 1000×750 → 750×1000
        // Target is 375×500 → not identity
        assert!(!lp.resize_is_identity);
    }

    // ── Decoder crop + prescale combo ────────────────────────────────

    #[test]
    fn decoder_crop_then_prescale() {
        // Request crop 200×200, decoder crops to 208×208 (MCU) then prescales 1/2 → 104×104
        let commands = [
            Command::Crop(SourceCrop::pixels(100, 100, 200, 200)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (100, 100));

        let offer = DecoderOffer {
            dimensions: (104, 104), // MCU-aligned crop, then 1/2 prescale
            crop_applied: Some(Rect::new(96, 96, 208, 208)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        // Trim needed: within the 104×104 output, offset (4/2, 4/2) for 100×100?
        // Actually, the trim is computed from requested vs applied crop in source coords,
        // not accounting for prescale. The trim rect is in decoder-output coords.
        let trim = lp.trim.unwrap();
        assert_eq!(trim.x, 4); // 100 - 96 in source coords
        assert_eq!(trim.y, 4);
        // Width/height capped at decoder_w - dx
        assert_eq!(trim.width, 100); // min(200, 104-4) = 100
        assert_eq!(trim.height, 100);
    }

    // ── Canvas / placement preserved through finalize ────────────────

    #[test]
    fn finalize_preserves_canvas_from_fit_pad() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::FitPad, 400, 400)
                .canvas_color(CanvasColor::white()),
        }];
        let (ideal, req) = plan(&commands, 1000, 500, None).unwrap();
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.layout.resize_to, (400, 200));
        assert_eq!(ideal.layout.placement, (0, 100));

        let offer = DecoderOffer::full_decode(1000, 500);
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.canvas, (400, 400));
        assert_eq!(lp.placement, (0, 100));
        assert_eq!(lp.canvas_color, CanvasColor::white());
    }

    #[test]
    fn finalize_preserves_canvas_from_fit_crop() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::FitCrop, 400, 400),
        }];
        let (ideal, req) = plan(&commands, 1000, 500, None).unwrap();
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.layout.resize_to, (400, 400));

        let offer = DecoderOffer::full_decode(1000, 500);
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.canvas, (400, 400));
        assert_eq!(lp.resize_to, (400, 400));
    }

    // ── Decoder applies unrequested crop ─────────────────────────────

    #[test]
    fn decoder_crops_unrequested() {
        // No crop in commands, but decoder crops anyway (weird but possible)
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 400, 300),
        }];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        assert!(req.crop.is_none());

        // Decoder randomly crops to 700×500
        let offer = DecoderOffer {
            dimensions: (700, 500),
            crop_applied: Some(Rect::new(50, 50, 700, 500)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        // No trim (we didn't request a crop, so no trim logic fires)
        assert!(lp.trim.is_none());
        // Resize target is still what the layout computed
        assert_eq!(lp.resize_to, (400, 300));
        // But resize_is_identity will be false (700×500 ≠ 400×300)
        assert!(!lp.resize_is_identity);
    }

    // ── Orientation composition edge cases with finalize ─────────────

    #[test]
    fn decoder_applies_inverse_of_requested() {
        // We want Rotate90, decoder applies Rotate270 (the inverse)
        let commands = [Command::AutoOrient(6)]; // Rotate90
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (600, 800), // 270° swaps
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_270,
        };
        let lp = finalize(&ideal, &req, &offer);

        // remaining = inverse(270°) ∘ 90° = 90° ∘ 90° = 180°
        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_180);
        // After 180° on 600×800 → 600×800 = target
        assert!(lp.resize_is_identity);
    }

    #[test]
    fn decoder_double_applies_orientation() {
        // We want Rotate90, decoder applies Rotate90 twice (=180°)
        // This is a weird edge case: decoder composed with itself
        let commands = [Command::AutoOrient(6)]; // Rotate90
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (800, 600), // 180° doesn't swap
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_180,
        };
        let lp = finalize(&ideal, &req, &offer);

        // remaining = inverse(180°) ∘ 90° = 180° ∘ 90° = 270°
        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_270);
        // 270° on 800×600 → 600×800 = target
        assert!(lp.resize_is_identity);
    }

    // ── Asymmetric images with orientation ────────────────────────────

    #[test]
    fn tall_image_rotate90_decoder_handles() {
        // 100×1000 (very tall), rotate 90° → 1000×100, fit to 500×500
        let commands = [
            Command::AutoOrient(6),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 500, 500),
            },
        ];
        let (ideal, req) = plan(&commands, 100, 1000, None).unwrap();
        // oriented: 1000×100, fit to 500×500 → 500×50
        assert_eq!(ideal.layout.resize_to, (500, 50));

        let offer = DecoderOffer {
            dimensions: (1000, 100),
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_90,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        assert!(!lp.resize_is_identity);
        assert_eq!(lp.resize_to, (500, 50));
    }

    #[test]
    fn square_image_all_orientations_are_identity() {
        // Square image: all orientations produce same dimensions
        for exif in 1..=8u8 {
            let commands = [Command::AutoOrient(exif)];
            let (_, lp) = plan_finalize(&commands, 500, 500, &DecoderOffer::full_decode(500, 500));
            assert_eq!(lp.resize_to, (500, 500), "EXIF {exif}");
            assert!(lp.resize_is_identity, "EXIF {exif}");
        }
    }

    // ── Crop + constraint + orient + decoder partial ─────────────────

    #[test]
    fn full_pipeline_decoder_handles_only_orient() {
        // EXIF 8 (Rotate270) + crop + fit
        // 800×600 → oriented 600×800 → crop(50,50,400,600) → fit(200,200) → 150×200
        let commands = [
            Command::AutoOrient(8),
            Command::Crop(SourceCrop::pixels(50, 50, 400, 600)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        // Decoder handles rotation but not crop
        let offer = DecoderOffer {
            dimensions: (600, 800), // 270° swaps
            crop_applied: None,
            orientation_applied: Orientation::ROTATE_270,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        // Crop was in source coords; decoder didn't crop → trim = source crop
        assert!(lp.trim.is_some());
        assert!(!lp.resize_is_identity);
    }

    #[test]
    fn full_pipeline_decoder_handles_nothing() {
        // Same pipeline, decoder does absolutely nothing
        let commands = [
            Command::AutoOrient(8),
            Command::Crop(SourceCrop::pixels(50, 50, 400, 600)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_270);
        assert!(lp.trim.is_some()); // crop not handled
        assert!(!lp.resize_is_identity);
    }

    #[test]
    fn full_pipeline_decoder_handles_everything() {
        // Decoder handles orient + crop + prescale to exact target
        let commands = [
            Command::AutoOrient(8),
            Command::Crop(SourceCrop::pixels(50, 50, 400, 600)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
        ];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();
        let target = ideal.layout.resize_to;

        let offer = DecoderOffer {
            dimensions: target,
            crop_applied: req.crop,
            orientation_applied: Orientation::ROTATE_270,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
        assert!(lp.trim.is_none());
        assert!(lp.resize_is_identity);
    }

    // ── Narrow / extreme aspect ratios ───────────────────────────────

    #[test]
    fn extreme_aspect_ratio_1x10000() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
        }];
        let (ideal, req) = plan(&commands, 1, 10000, None).unwrap();
        // Fit 1×10000 into 100×100 → 1×100
        assert_eq!(ideal.layout.resize_to, (1, 100));

        let offer = DecoderOffer::full_decode(1, 10000);
        let lp = finalize(&ideal, &req, &offer);
        assert!(!lp.resize_is_identity);
        assert_eq!(lp.resize_to, (1, 100));
    }

    #[test]
    fn extreme_aspect_ratio_10000x1() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 100, 100),
        }];
        let (ideal, _) = plan(&commands, 10000, 1, None).unwrap();
        assert_eq!(ideal.layout.resize_to, (100, 1));
    }

    // ── Exact match decoder behavior ─────────────────────────────────

    #[test]
    fn decoder_exact_crop_no_trim() {
        let commands = [Command::Crop(SourceCrop::pixels(100, 100, 200, 200))];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (200, 200),
            crop_applied: Some(Rect::new(100, 100, 200, 200)),
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert!(lp.trim.is_none());
        assert!(lp.resize_is_identity);
    }

    // ── Flips are self-inverse ───────────────────────────────────────

    #[test]
    fn decoder_applies_same_flip_twice_is_identity() {
        // User wants FlipH, decoder also applies FlipH → remaining = identity
        let commands = [Command::Flip(FlipAxis::Horizontal)];
        let (ideal, req) = plan(&commands, 800, 600, None).unwrap();

        let offer = DecoderOffer {
            dimensions: (800, 600),
            crop_applied: None,
            orientation_applied: Orientation::FLIP_H,
        };
        let lp = finalize(&ideal, &req, &offer);
        assert_eq!(lp.remaining_orientation, Orientation::IDENTITY);
    }

    // ── FitPad with decoder prescale ─────────────────────────────────

    #[test]
    fn fit_pad_with_prescaled_decoder() {
        let commands = [Command::Constrain {
            constraint: Constraint::new(ConstraintMode::FitPad, 400, 400)
                .canvas_color(CanvasColor::white()),
        }];
        let (ideal, req) = plan(&commands, 4000, 2000, None).unwrap();
        // Fit 4000×2000 into 400×400 → 400×200, canvas 400×400, placement (0,100)
        assert_eq!(ideal.layout.resize_to, (400, 200));
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.layout.placement, (0, 100));

        // Decoder prescales to 1000×500
        let offer = DecoderOffer {
            dimensions: (1000, 500),
            crop_applied: None,
            orientation_applied: Orientation::IDENTITY,
        };
        let lp = finalize(&ideal, &req, &offer);

        assert_eq!(lp.resize_to, (400, 200));
        assert_eq!(lp.canvas, (400, 400));
        assert_eq!(lp.placement, (0, 100));
        assert!(!lp.resize_is_identity);
    }

    // ════════════════════════════════════════════════════════════════════
    // Pipeline builder API
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn pipeline_basic_fit() {
        let (ideal, _) = Pipeline::new(800, 600).fit(400, 300).plan().unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 300));
    }

    #[test]
    fn pipeline_within() {
        let (ideal, _) = Pipeline::new(200, 100).within(400, 300).plan().unwrap();
        // Source smaller than target → no upscale
        assert_eq!(ideal.layout.resize_to, (200, 100));
    }

    #[test]
    fn pipeline_orient_then_fit() {
        let (ideal, _) = Pipeline::new(800, 600)
            .auto_orient(6) // Rotate90
            .fit(300, 300)
            .plan()
            .unwrap();
        // 800×600 → oriented 600×800 → fit 300×300 → 225×300
        assert_eq!(ideal.layout.resize_to, (225, 300));
    }

    #[test]
    fn pipeline_matches_command_api() {
        // Same operation via both APIs should produce identical results
        let commands = [
            Command::AutoOrient(6),
            Command::Crop(SourceCrop::pixels(50, 50, 400, 600)),
            Command::Constrain {
                constraint: Constraint::new(ConstraintMode::Fit, 200, 200),
            },
        ];
        let (ideal_cmd, req_cmd) = plan(&commands, 800, 600, None).unwrap();

        let (ideal_pipe, req_pipe) = Pipeline::new(800, 600)
            .auto_orient(6)
            .crop_pixels(50, 50, 400, 600)
            .fit(200, 200)
            .plan()
            .unwrap();

        assert_eq!(ideal_cmd.orientation, ideal_pipe.orientation);
        assert_eq!(ideal_cmd.layout, ideal_pipe.layout);
        assert_eq!(ideal_cmd.source_crop, ideal_pipe.source_crop);
        assert_eq!(req_cmd, req_pipe);
    }

    #[test]
    fn pipeline_stacked_rotations() {
        let (ideal, _) = Pipeline::new(800, 600)
            .auto_orient(6) // Rotate90
            .rotate_90() // +90 = 180 total
            .plan()
            .unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn pipeline_flip_h_and_v() {
        let (ideal, _) = Pipeline::new(800, 600).flip_h().flip_v().plan().unwrap();
        // FlipH then FlipV = Rotate180
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
    }

    #[test]
    fn pipeline_crop_percent() {
        let (ideal, _) = Pipeline::new(1000, 1000)
            .crop_percent(0.1, 0.1, 0.8, 0.8)
            .plan()
            .unwrap();
        let crop = ideal.layout.source_crop.unwrap();
        assert_eq!(crop, Rect::new(100, 100, 800, 800));
    }

    #[test]
    fn pipeline_fit_crop() {
        let (ideal, _) = Pipeline::new(1000, 500).fit_crop(400, 400).plan().unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 400));
        assert!(ideal.layout.source_crop.is_some());
    }

    #[test]
    fn pipeline_fit_pad() {
        let (ideal, _) = Pipeline::new(1000, 500).fit_pad(400, 400).plan().unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 200));
        assert_eq!(ideal.layout.canvas, (400, 400));
    }

    #[test]
    fn pipeline_distort() {
        let (ideal, _) = Pipeline::new(800, 600).distort(100, 100).plan().unwrap();
        assert_eq!(ideal.layout.resize_to, (100, 100));
    }

    #[test]
    fn pipeline_aspect_crop() {
        let (ideal, _) = Pipeline::new(1000, 500)
            .aspect_crop(400, 400)
            .plan()
            .unwrap();
        // Crop to 1:1 aspect, no scaling
        let crop = ideal.layout.source_crop.unwrap();
        assert_eq!(crop.width, crop.height);
        assert_eq!(ideal.layout.resize_to, (crop.width, crop.height));
    }

    #[test]
    fn pipeline_pad_uniform() {
        let (ideal, _) = Pipeline::new(400, 300)
            .pad_uniform(10, CanvasColor::white())
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 300));
        assert_eq!(ideal.layout.canvas, (420, 320));
    }

    #[test]
    fn pipeline_pad_asymmetric() {
        let (ideal, _) = Pipeline::new(400, 300)
            .pad(5, 10, 15, 20, CanvasColor::black())
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (430, 320)); // 400+10+20, 300+5+15
        assert_eq!(ideal.layout.placement, (20, 5));
    }

    #[test]
    fn pipeline_constrain_with_gravity() {
        let (ideal, _) = Pipeline::new(1000, 500)
            .constrain(
                Constraint::new(ConstraintMode::FitPad, 400, 400)
                    .gravity(Gravity::Percentage(0.0, 0.0))
                    .canvas_color(CanvasColor::white()),
            )
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 200));
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.layout.placement, (0, 0)); // top-left gravity
    }

    #[test]
    fn pipeline_full_roundtrip() {
        // End-to-end: build pipeline, plan, finalize with full_decode
        let (ideal, req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(100, 100, 2000, 2500)
            .within(800, 800)
            .pad_uniform(5, CanvasColor::black())
            .plan()
            .unwrap();

        let lp = ideal.finalize(&req, &DecoderOffer::full_decode(4000, 3000));
        assert_eq!(lp.remaining_orientation, Orientation::ROTATE_90);
        assert!(lp.trim.is_some());
        assert!(!lp.resize_is_identity);
        // Canvas should be resize_to + 10 each dim
        assert_eq!(lp.canvas.0, lp.resize_to.0 + 10);
        assert_eq!(lp.canvas.1, lp.resize_to.1 + 10);
    }

    #[test]
    fn pipeline_zero_source_rejected() {
        assert!(Pipeline::new(0, 600).fit(100, 100).plan().is_err());
        assert!(Pipeline::new(800, 0).fit(100, 100).plan().is_err());
    }

    #[test]
    fn pipeline_first_constraint_wins() {
        let (ideal, _) = Pipeline::new(800, 600)
            .fit(200, 200)
            .within(100, 100) // ignored
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (200, 150));
    }

    #[test]
    fn pipeline_first_crop_wins() {
        let (ideal, _) = Pipeline::new(800, 600)
            .crop_pixels(0, 0, 100, 100)
            .crop_pixels(200, 200, 50, 50) // ignored
            .plan()
            .unwrap();
        let crop = ideal.source_crop.unwrap();
        assert_eq!(crop, Rect::new(0, 0, 100, 100));
    }

    #[test]
    fn pipeline_within_crop() {
        let (ideal, _) = Pipeline::new(1000, 500)
            .within_crop(400, 400)
            .plan()
            .unwrap();
        // Source larger → crop to aspect + downscale
        assert_eq!(ideal.layout.resize_to, (400, 400));
        assert!(ideal.layout.source_crop.is_some());
    }

    #[test]
    fn pipeline_within_pad() {
        let (ideal, _) = Pipeline::new(200, 100).within_pad(400, 300).plan().unwrap();
        // Source fits within target → identity (imageflow behavior)
        assert_eq!(ideal.layout.resize_to, (200, 100));
        assert_eq!(ideal.layout.canvas, (200, 100));
    }

    #[test]
    fn pipeline_rotate_270() {
        let (ideal, _) = Pipeline::new(800, 600).rotate_270().plan().unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_270);
        assert_eq!(ideal.layout.resize_to, (600, 800));
    }

    #[test]
    fn pipeline_rotate_180() {
        let (ideal, _) = Pipeline::new(800, 600).rotate_180().plan().unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    // ════════════════════════════════════════════════════════════════════
    // Secondary plane / gain map tests
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn secondary_no_crop_quarter_scale() {
        // SDR: 4000×3000, gain map: 1000×750 (exactly 1/4)
        let (sdr, _) = Pipeline::new(4000, 3000).fit(800, 600).plan().unwrap();
        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        assert_eq!(gm.orientation, sdr.orientation);
        assert!(gm.source_crop.is_none()); // no crop → no crop
        assert!(gm_req.crop.is_none());
        // Auto target: 800×600 * 0.25 = 200×150
        assert_eq!(gm.layout.resize_to, (200, 150));
    }

    #[test]
    fn secondary_no_crop_explicit_target() {
        let (sdr, _) = Pipeline::new(4000, 3000).fit(800, 600).plan().unwrap();
        let (gm, _) = sdr.derive_secondary((4000, 3000), (1000, 750), Some((800, 600)));

        // Explicit target: gain map rendered at full SDR size
        assert_eq!(gm.layout.resize_to, (800, 600));
    }

    #[test]
    fn secondary_crop_scales_to_quarter() {
        // SDR crop (100,100,200,200) → gain map at 1/4 → (25,25,50,50)
        let (sdr, _) = Pipeline::new(4000, 3000)
            .crop_pixels(100, 100, 200, 200)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        let crop = gm_req.crop.unwrap();
        assert_eq!(crop, Rect::new(25, 25, 50, 50));
        assert_eq!(gm.source_crop, gm_req.crop);
    }

    #[test]
    fn secondary_crop_rounds_outward() {
        // SDR crop (103,47,200,200). At 1/4:
        //   x: floor(103*0.25) = floor(25.75) = 25
        //   y: floor(47*0.25) = floor(11.75) = 11
        //   x1: ceil(303*0.25) = ceil(75.75) = 76 → w = 76-25 = 51
        //   y1: ceil(247*0.25) = ceil(61.75) = 62 → h = 62-11 = 51
        let (sdr, _) = Pipeline::new(4000, 3000)
            .crop_pixels(103, 47, 200, 200)
            .plan()
            .unwrap();

        let (_, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);
        let crop = gm_req.crop.unwrap();

        assert_eq!(crop.x, 25);
        assert_eq!(crop.y, 11);
        assert_eq!(crop.width, 51); // rounds outward
        assert_eq!(crop.height, 51);
    }

    #[test]
    fn secondary_orientation_preserved() {
        let (sdr, _) = Pipeline::new(4000, 3000)
            .auto_orient(6) // Rotate90
            .fit(800, 800)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        assert_eq!(gm.orientation, Orientation::ROTATE_90);
        assert_eq!(gm_req.orientation, Orientation::ROTATE_90);
        // Oriented secondary: 750×1000 (rotated)
        assert_eq!(gm.layout.source, (750, 1000));
    }

    #[test]
    fn secondary_crop_with_orientation() {
        // SDR: 4000×3000, rotate 90° → oriented 3000×4000
        // Crop 100,100,2000,2000 in oriented space
        let (sdr, sdr_req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(100, 100, 2000, 2000)
            .fit(500, 500)
            .plan()
            .unwrap();

        let (_gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        // Both should have crops in source (pre-orient) space
        assert!(sdr_req.crop.is_some());
        assert!(gm_req.crop.is_some());

        // The gain map crop should be roughly 1/4 of the SDR crop
        let sdr_crop = sdr_req.crop.unwrap();
        let gm_crop = gm_req.crop.unwrap();

        // Spatial coverage should be at least as large (round-outward)
        let sdr_right = sdr_crop.x + sdr_crop.width;
        let sdr_bottom = sdr_crop.y + sdr_crop.height;
        let gm_right = gm_crop.x + gm_crop.width;
        let gm_bottom = gm_crop.y + gm_crop.height;

        // Gain map crop scaled back up should encompass SDR crop
        assert!(gm_crop.x as f64 * 4.0 <= sdr_crop.x as f64 + 0.01);
        assert!(gm_crop.y as f64 * 4.0 <= sdr_crop.y as f64 + 0.01);
        assert!(gm_right as f64 * 4.0 >= sdr_right as f64 - 0.01);
        assert!(gm_bottom as f64 * 4.0 >= sdr_bottom as f64 - 0.01);
    }

    #[test]
    fn secondary_finalize_both_full_decode() {
        // Both decoders do nothing — both finalize independently
        let (sdr, sdr_req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(100, 100, 2000, 2000)
            .fit(500, 500)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        let sdr_plan = sdr.finalize(&sdr_req, &DecoderOffer::full_decode(4000, 3000));
        let gm_plan = gm.finalize(&gm_req, &DecoderOffer::full_decode(1000, 750));

        // Both should need the same remaining orientation
        assert_eq!(sdr_plan.remaining_orientation, Orientation::ROTATE_90);
        assert_eq!(gm_plan.remaining_orientation, Orientation::ROTATE_90);

        // Both need trim (decoder didn't crop)
        assert!(sdr_plan.trim.is_some());
        assert!(gm_plan.trim.is_some());

        // Neither is identity (both need resize)
        assert!(!sdr_plan.resize_is_identity);
        assert!(!gm_plan.resize_is_identity);
    }

    #[test]
    fn secondary_finalize_decoders_differ() {
        // SDR decoder: handles orientation + crop
        // Gain map decoder: does nothing (full decode)
        let (sdr, sdr_req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(100, 100, 2000, 2000)
            .fit(500, 500)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        // SDR decoder handles everything
        let sdr_offer = DecoderOffer {
            dimensions: (2000, 2000),
            crop_applied: sdr_req.crop,
            orientation_applied: Orientation::ROTATE_90,
        };
        let sdr_plan = sdr.finalize(&sdr_req, &sdr_offer);

        // Gain map decoder does nothing
        let gm_offer = DecoderOffer::full_decode(1000, 750);
        let gm_plan = gm.finalize(&gm_req, &gm_offer);

        // SDR: decoder did everything → no trim, no remaining orient
        assert!(sdr_plan.trim.is_none());
        assert_eq!(sdr_plan.remaining_orientation, Orientation::IDENTITY);

        // Gain map: decoder did nothing → trim + orient remain
        assert!(gm_plan.trim.is_some());
        assert_eq!(gm_plan.remaining_orientation, Orientation::ROTATE_90);

        // But both produce spatially-locked output (same orientation effect)
    }

    #[test]
    fn secondary_finalize_gain_map_has_own_mcu_grid() {
        // SDR crop (100,100,200,200), gain map at 1/4 scale
        // SDR decoder: MCU-aligns to (96,96,208,208) → output 208×208
        // GM decoder: MCU-aligns to (24,24,56,56) → output 56×56
        // (GM crop was (25,25,50,50) but decoder aligned differently)
        let (sdr, sdr_req) = Pipeline::new(800, 600)
            .crop_pixels(100, 100, 200, 200)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((800, 600), (200, 150), None);
        let gm_crop_requested = gm_req.crop.unwrap();

        // SDR decoder: MCU-aligned
        let sdr_offer = DecoderOffer {
            dimensions: (208, 208),
            crop_applied: Some(Rect::new(96, 96, 208, 208)),
            orientation_applied: Orientation::IDENTITY,
        };
        let sdr_plan = sdr.finalize(&sdr_req, &sdr_offer);

        // GM decoder: different MCU alignment
        let gm_offer = DecoderOffer {
            dimensions: (56, 56),
            crop_applied: Some(Rect::new(24, 24, 56, 56)),
            orientation_applied: Orientation::IDENTITY,
        };
        let gm_plan = gm.finalize(&gm_req, &gm_offer);

        // SDR trim: offset within decoder output
        let sdr_trim = sdr_plan.trim.unwrap();
        assert_eq!(sdr_trim.x, 4); // 100 - 96
        assert_eq!(sdr_trim.y, 4);
        assert_eq!(sdr_trim.width, 200);
        assert_eq!(sdr_trim.height, 200);

        // GM trim: offset within its decoder output
        let gm_trim = gm_plan.trim.unwrap();
        let expected_dx = gm_crop_requested.x - 24; // requested.x - applied.x
        let expected_dy = gm_crop_requested.y - 24;
        assert_eq!(gm_trim.x, expected_dx);
        assert_eq!(gm_trim.y, expected_dy);
        assert_eq!(gm_trim.width, gm_crop_requested.width);
        assert_eq!(gm_trim.height, gm_crop_requested.height);

        // Both independently correct despite different MCU grids
        assert!(sdr_plan.resize_is_identity);
        assert!(gm_plan.resize_is_identity);
    }

    #[test]
    fn secondary_no_padding() {
        let (sdr, _) = Pipeline::new(800, 600)
            .fit_pad(400, 400)
            .pad_uniform(10, CanvasColor::white())
            .plan()
            .unwrap();

        // SDR has padding
        assert!(sdr.padding.is_some());
        assert_eq!(sdr.layout.canvas, (420, 420)); // 400+20 from explicit pad

        let (gm, _) = sdr.derive_secondary((800, 600), (200, 150), None);

        // Gain map: no padding
        assert!(gm.padding.is_none());
        assert_eq!(gm.layout.canvas, gm.layout.resize_to);
    }

    #[test]
    fn secondary_non_integer_scale() {
        // SDR: 1920×1080, gain map: 480×270 (exactly 1/4)
        // Crop at (100,50,500,300): y0=floor(50*0.25)=12, y1=ceil(350*0.25)=88 → h=76
        // Round-outward because 50/4=12.5 doesn't divide cleanly
        let (sdr, _) = Pipeline::new(1920, 1080)
            .crop_pixels(100, 50, 500, 300)
            .plan()
            .unwrap();

        let (_, gm_req) = sdr.derive_secondary((1920, 1080), (480, 270), None);
        let crop = gm_req.crop.unwrap();
        assert_eq!(crop, Rect::new(25, 12, 125, 76));
    }

    #[test]
    fn secondary_odd_ratio() {
        // SDR: 1000×1000, gain map: 333×333 (1/3.003 — not clean)
        // Crop (100,100,600,600) → scaled: floor(33.3)=33, ceil(233.1)=234 → w=201
        let (sdr, _) = Pipeline::new(1000, 1000)
            .crop_pixels(100, 100, 600, 600)
            .plan()
            .unwrap();

        let (_, gm_req) = sdr.derive_secondary((1000, 1000), (333, 333), None);
        let crop = gm_req.crop.unwrap();

        // Round outward: origin floors, far edge ceils
        let scale: f64 = 333.0 / 1000.0;
        let x0 = (100.0_f64 * scale).floor() as u32;
        let y0 = (100.0_f64 * scale).floor() as u32;
        let x1 = (700.0_f64 * scale).ceil() as u32;
        let y1 = (700.0_f64 * scale).ceil() as u32;
        assert_eq!(crop.x, x0);
        assert_eq!(crop.y, y0);
        assert_eq!(crop.width, x1 - x0);
        assert_eq!(crop.height, y1 - y0);

        // Verify outward: scaled back up should encompass original
        assert!(crop.x as f64 / scale <= 100.0 + 0.01);
        assert!((crop.x + crop.width) as f64 / scale >= 700.0 - 0.01);
    }

    #[test]
    fn secondary_crop_at_edge_clamped() {
        // SDR crop near right edge: (3800,2800,200,200) in 4000×3000
        // GM at 1/4: (950,700,50,50) — right at the edge of 1000×750
        let (sdr, _) = Pipeline::new(4000, 3000)
            .crop_pixels(3800, 2800, 200, 200)
            .plan()
            .unwrap();

        let (_, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);
        let crop = gm_req.crop.unwrap();

        // Should be clamped to gain map bounds
        assert!(crop.x + crop.width <= 1000);
        assert!(crop.y + crop.height <= 750);
    }

    #[test]
    fn secondary_passthrough_no_commands() {
        let (sdr, _) = Pipeline::new(800, 600).plan().unwrap();
        let (gm, gm_req) = sdr.derive_secondary((800, 600), (200, 150), None);

        assert!(gm.source_crop.is_none());
        assert!(gm_req.crop.is_none());
        assert_eq!(gm.orientation, Orientation::IDENTITY);
        assert_eq!(gm.layout.resize_to, (200, 150));
        assert_eq!(gm.layout.source, (200, 150));
    }

    #[test]
    fn secondary_lossless_path() {
        // Both SDR and gain map do rotate+crop, both decoders handle it
        let (sdr, sdr_req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(0, 0, 1000, 1500)
            .plan()
            .unwrap();

        let (gm, gm_req) = sdr.derive_secondary((4000, 3000), (1000, 750), None);

        // SDR decoder handles everything
        let sdr_offer = DecoderOffer {
            dimensions: (1000, 1500),
            crop_applied: sdr_req.crop,
            orientation_applied: Orientation::ROTATE_90,
        };
        let sdr_plan = sdr.finalize(&sdr_req, &sdr_offer);
        assert!(sdr_plan.resize_is_identity);
        assert_eq!(sdr_plan.remaining_orientation, Orientation::IDENTITY);

        // GM decoder handles everything too
        let gm_offer = DecoderOffer {
            dimensions: (gm.layout.resize_to.0, gm.layout.resize_to.1),
            crop_applied: gm_req.crop,
            orientation_applied: Orientation::ROTATE_90,
        };
        let gm_plan = gm.finalize(&gm_req, &gm_offer);
        assert!(gm_plan.resize_is_identity);
        assert_eq!(gm_plan.remaining_orientation, Orientation::IDENTITY);
    }

    #[test]
    fn secondary_all_8_orientations() {
        for exif in 1..=8u8 {
            let (sdr, _) = Pipeline::new(800, 600)
                .auto_orient(exif)
                .plan()
                .unwrap();
            let (gm, gm_req) = sdr.derive_secondary((800, 600), (200, 150), None);

            assert_eq!(
                gm.orientation,
                Orientation::from_exif(exif).unwrap(),
                "EXIF {exif}"
            );
            assert_eq!(gm_req.orientation, gm.orientation);

            // Oriented dims should match
            let expected = Orientation::from_exif(exif)
                .unwrap()
                .transform_dimensions(200, 150);
            assert_eq!(gm.layout.source, expected, "EXIF {exif} oriented dims");
        }
    }

    // ── MandatoryConstraints ────────────────────────────────────────────

    #[test]
    fn limits_max_caps_canvas() {
        // Fit 100×100 into 2000×2000 → resize_to=2000×2000
        // Max 500×500 should cap to 500×500
        let (ideal, _) = Pipeline::new(100, 100)
            .fit(2000, 2000)
            .limits(MandatoryConstraints {
                max: Some((500, 500)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert!(ideal.layout.resize_to.0 <= 500);
        assert!(ideal.layout.resize_to.1 <= 500);
        assert!(ideal.layout.canvas.0 <= 500);
        assert!(ideal.layout.canvas.1 <= 500);
    }

    #[test]
    fn limits_max_preserves_aspect() {
        // 1000×500 fit to 2000×1000 → resize_to=2000×1000
        // Max 600×600 → should scale to 600×300 (2:1 aspect preserved)
        let (ideal, _) = Pipeline::new(1000, 500)
            .fit(2000, 1000)
            .limits(MandatoryConstraints {
                max: Some((600, 600)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (600, 300));
        assert_eq!(ideal.layout.canvas, (600, 300));
    }

    #[test]
    fn limits_max_scales_padded_canvas() {
        // FitPad(400, 400) on 800×600 → resize_to=(400,300), canvas=(400,400)
        // Max 200×200 → scale by 0.5 → resize_to=(200,150), canvas=(200,200)
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(400, 400)
            .limits(MandatoryConstraints {
                max: Some((200, 200)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert!(ideal.layout.canvas.0 <= 200);
        assert!(ideal.layout.canvas.1 <= 200);
        assert_eq!(ideal.layout.resize_to, (200, 150));
        assert_eq!(ideal.layout.canvas, (200, 200));
    }

    #[test]
    fn limits_max_noop_when_within() {
        // Already within max → no change
        let (ideal, _) = Pipeline::new(800, 600)
            .fit(400, 300)
            .limits(MandatoryConstraints {
                max: Some((1000, 1000)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 300));
    }

    #[test]
    fn limits_min_scales_up() {
        // 1000×1000 within 50×50 → resize_to=50×50 (no upscale)
        // Wait, Within doesn't upscale. Use Within so we get small output.
        let (ideal, _) = Pipeline::new(100, 100)
            .within(50, 50)
            .limits(MandatoryConstraints {
                min: Some((200, 200)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        // Min should push resize_to up to at least 200 on the smaller axis
        assert!(ideal.layout.resize_to.0 >= 200);
        assert!(ideal.layout.resize_to.1 >= 200);
    }

    #[test]
    fn limits_min_preserves_aspect() {
        // 1000×500 within 100×50 → resize_to=100×50
        // Min (200, 200) → scale = max(200/100, 200/50) = 4 → 400×200
        let (ideal, _) = Pipeline::new(1000, 500)
            .within(100, 50)
            .limits(MandatoryConstraints {
                min: Some((200, 200)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (400, 200));
    }

    #[test]
    fn limits_max_wins_over_min() {
        // min=500×500, max=200×200 → max wins
        let (ideal, _) = Pipeline::new(1000, 1000)
            .within(100, 100)
            .limits(MandatoryConstraints {
                max: Some((200, 200)),
                min: Some((500, 500)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert!(ideal.layout.resize_to.0 <= 200);
        assert!(ideal.layout.resize_to.1 <= 200);
        assert!(ideal.layout.canvas.0 <= 200);
        assert!(ideal.layout.canvas.1 <= 200);
    }

    #[test]
    fn limits_align_snaps_down() {
        // 1000×667 fit to 1000×667 → resize_to=1000×667
        // Align 16 → 992×656
        let (ideal, _) = Pipeline::new(1000, 667)
            .fit(1000, 667)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to.0 % 16, 0);
        assert_eq!(ideal.layout.resize_to.1 % 16, 0);
        assert_eq!(ideal.layout.resize_to, (992, 656));
    }

    #[test]
    fn limits_align_mod2_for_video() {
        // 801×601 → align 2 → 800×600
        let (ideal, _) = Pipeline::new(801, 601)
            .fit(801, 601)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(2)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn limits_align_preserves_padded_canvas() {
        // FitPad(400, 400) on 800×600 → resize_to=(400,300), canvas=(400,400)
        // Align 16: canvas already 400×400 (mod 16). No change.
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(400, 400)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas.0 % 16, 0);
        assert_eq!(ideal.layout.canvas.1 % 16, 0);
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.layout.resize_to, (400, 300)); // unchanged
    }

    #[test]
    fn limits_align_snaps_padded_canvas() {
        // FitPad(401, 401) on 800×600 → resize_to=(401,301), canvas=(401,401)
        // Align 16: canvas → 400×400, resize_to stays 401→clamped to 400, 301 stays
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(401, 401)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas.0 % 16, 0);
        assert_eq!(ideal.layout.canvas.1 % 16, 0);
        // resize_to clamped to canvas
        assert!(ideal.layout.resize_to.0 <= ideal.layout.canvas.0);
        assert!(ideal.layout.resize_to.1 <= ideal.layout.canvas.1);
    }

    #[test]
    fn limits_align_1_is_noop() {
        let (ideal, _) = Pipeline::new(801, 601)
            .fit(801, 601)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(1)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.resize_to, (801, 601));
    }

    #[test]
    fn limits_all_three_combined() {
        // Start big: 100×100 fit to 10000×10000
        // Max 1920×1080, min 100×100, align 8
        let (ideal, _) = Pipeline::new(100, 100)
            .fit(10000, 10000)
            .limits(MandatoryConstraints {
                max: Some((1920, 1080)),
                min: Some((100, 100)),
                align: Some(Align::RoundDown(8)),
            })
            .plan()
            .unwrap();
        // Max caps to 1080×1080 (square, height constrains)
        assert!(ideal.layout.canvas.0 <= 1920);
        assert!(ideal.layout.canvas.1 <= 1080);
        // Align snaps canvas
        assert_eq!(ideal.layout.canvas.0 % 8, 0);
        assert_eq!(ideal.layout.canvas.1 % 8, 0);
        // Min satisfied (1080 > 100)
        assert!(ideal.layout.canvas.0 >= 100);
    }

    #[test]
    fn limits_max_with_explicit_pad() {
        // Fit(200, 200) on 400×400 + pad 50 all → resize=200×200, canvas=300×300
        // Max 250×250 → scale by 250/300 ≈ 0.833
        let (ideal, _) = Pipeline::new(400, 400)
            .fit(200, 200)
            .pad_uniform(50, CanvasColor::white())
            .limits(MandatoryConstraints {
                max: Some((250, 250)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert!(ideal.layout.canvas.0 <= 250);
        assert!(ideal.layout.canvas.1 <= 250);
    }

    #[test]
    fn limits_default_is_noop() {
        // Default MandatoryConstraints (all None) should be identity
        let (a, _) = Pipeline::new(800, 600).fit(400, 300).plan().unwrap();
        let (b, _) = Pipeline::new(800, 600)
            .fit(400, 300)
            .limits(MandatoryConstraints::default())
            .plan()
            .unwrap();
        assert_eq!(a.layout, b.layout);
    }

    #[test]
    fn limits_tiny_image_align_doesnt_zero() {
        // 3×3 image, align 16 → canvas snaps to 16×16, resize_to stays 3×3
        let (ideal, _) = Pipeline::new(3, 3)
            .fit(3, 3)
            .limits(MandatoryConstraints {
                align: Some(Align::RoundDown(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (16, 16));
        assert_eq!(ideal.layout.resize_to, (3, 3));
    }

    // ── CanvasColor::Linear ─────────────────────────────────────────────

    #[test]
    fn canvas_color_linear_equality() {
        let a = CanvasColor::Linear {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        };
        let b = CanvasColor::Linear {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn canvas_color_linear_ne_srgb() {
        let linear = CanvasColor::Linear {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let srgb = CanvasColor::white();
        assert_ne!(linear, srgb);
    }

    #[test]
    fn canvas_color_linear_in_pipeline() {
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(400, 400)
            .pad_uniform(
                10,
                CanvasColor::Linear {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
            )
            .plan()
            .unwrap();
        assert!(matches!(ideal.layout.canvas_color, CanvasColor::Linear { .. }));
    }

    // ── Align::Extend ───────────────────────────────────────────────────

    #[test]
    fn align_extend_rounds_up() {
        // 801×601, align extend 16 → canvas 816×608, encode_crop = (801, 601)
        let (ideal, _) = Pipeline::new(801, 601)
            .fit(801, 601)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (816, 608));
        assert_eq!(ideal.encode_crop, Some((801, 601)));
        assert_eq!(ideal.layout.placement, (0, 0));
        assert_eq!(ideal.layout.resize_to, (801, 601));
    }

    #[test]
    fn align_extend_already_aligned_noop() {
        // 800×640, already mod-16 → no extension
        let (ideal, _) = Pipeline::new(800, 640)
            .fit(800, 640)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (800, 640));
        assert_eq!(ideal.encode_crop, None);
    }

    #[test]
    fn align_extend_mod2() {
        // 801×601, mod-2 → 802×602
        let (ideal, _) = Pipeline::new(801, 601)
            .fit(801, 601)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(2)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (802, 602));
        assert_eq!(ideal.encode_crop, Some((801, 601)));
    }

    #[test]
    fn align_extend_mcu_8() {
        // 100×100, MCU-8 → 104×104
        let (ideal, _) = Pipeline::new(100, 100)
            .fit(100, 100)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(8)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (104, 104));
        assert_eq!(ideal.encode_crop, Some((100, 100)));
        assert_eq!(ideal.layout.resize_to, (100, 100));
    }

    #[test]
    fn align_extend_with_pad() {
        // FitPad(400, 400) on 800×600 → canvas=400×400 (already mod-16)
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(400, 400)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        // 400 is mod-16, so no extension
        assert_eq!(ideal.layout.canvas, (400, 400));
        assert_eq!(ideal.encode_crop, None);
    }

    #[test]
    fn align_extend_with_unaligned_pad() {
        // FitPad(401, 401) → canvas=401×401, extend to 416×416
        let (ideal, _) = Pipeline::new(800, 600)
            .fit_pad(401, 401)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert_eq!(ideal.layout.canvas, (416, 416));
        assert_eq!(ideal.encode_crop, Some((401, 401)));
        assert_eq!(ideal.layout.placement, (0, 0)); // moved to origin
    }

    #[test]
    fn align_extend_finalize_carries_through() {
        // encode_crop passes through finalize
        let (ideal, req) = Pipeline::new(801, 601)
            .fit(801, 601)
            .limits(MandatoryConstraints {
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        let offer = DecoderOffer::full_decode(801, 601);
        let lp = ideal.finalize(&req, &offer);
        assert_eq!(lp.canvas, (816, 608));
        assert_eq!(lp.encode_crop, Some((801, 601)));
    }

    #[test]
    fn align_extend_max_then_extend() {
        // 4000×3000 fit to 4000×3000, max 1920×1080 → 1440×1080
        // Then extend mod-16: 1440 is mod-16, 1080 is not → 1440×1088
        let (ideal, _) = Pipeline::new(4000, 3000)
            .fit(4000, 3000)
            .limits(MandatoryConstraints {
                max: Some((1920, 1080)),
                align: Some(Align::Extend(16)),
                ..Default::default()
            })
            .plan()
            .unwrap();
        assert!(ideal.layout.canvas.0 % 16 == 0);
        assert!(ideal.layout.canvas.1 % 16 == 0);
        // Max applied first, then extend only adds a few pixels
        assert!(ideal.layout.canvas.0 <= 1920 + 15);
        assert!(ideal.layout.canvas.1 <= 1080 + 15);
        if let Some((cw, ch)) = ideal.encode_crop {
            assert!(cw <= 1920);
            assert!(ch <= 1080);
        }
    }
}
