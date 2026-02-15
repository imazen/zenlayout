//! Command pipeline and decoder negotiation.
//!
//! Two-phase layout planning:
//! 1. [`plan()`] — compute ideal layout from commands + source dimensions → [`IdealLayout`] + [`DecoderRequest`]
//! 2. [`finalize()`] — given what the decoder actually did ([`DecoderOffer`]), compute remaining work → [`LayoutPlan`]

use crate::constraint::{CanvasColor, Constraint, Layout, LayoutError, Rect, SourceCrop};
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
    /// Final canvas dimensions.
    pub canvas: (u32, u32),
    /// Placement offset on canvas.
    pub placement: (u32, u32),
    /// Canvas background color.
    pub canvas_color: CanvasColor,
    /// True when no resize is needed (enables lossless path).
    pub resize_is_identity: bool,
}

/// Compute ideal layout from commands and source image dimensions.
///
/// Orientation commands (AutoOrient, Rotate, Flip) are composed into a single
/// net orientation. Crop and Constrain operate in post-orientation coordinates
/// (what the user sees after rotation). The resulting source crop is transformed
/// back to pre-orientation source coordinates for the decoder.
///
/// Only the first `Crop` and first `Constrain` command are used; duplicates are ignored.
pub fn plan(
    commands: &[Command],
    source_w: u32,
    source_h: u32,
) -> Result<(IdealLayout, DecoderRequest), LayoutError> {
    if source_w == 0 || source_h == 0 {
        return Err(LayoutError::ZeroSourceDimension);
    }

    // 1. Compose all orientation commands into a net orientation.
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

    // 2. Transform source dimensions to post-orientation space.
    let (ow, oh) = orientation.transform_dimensions(source_w, source_h);

    // 3. Compute layout in post-orientation space.
    let layout = if let Some(c) = constraint {
        // Merge any explicit crop into the constraint.
        let mut builder = c.clone();
        if let Some(sc) = crop {
            builder = builder.source_crop(*sc);
        }
        builder.compute(ow, oh)?
    } else if let Some(sc) = crop {
        // Crop only, no constraint — resolve crop rect, no resize.
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
        // No constraint, no crop — passthrough.
        Layout {
            source: (ow, oh),
            source_crop: None,
            resize_to: (ow, oh),
            canvas: (ow, oh),
            placement: (0, 0),
            canvas_color: CanvasColor::default(),
        }
    };

    // 4. Apply explicit padding if present.
    let layout = if let Some(pad) = &padding {
        let (rw, rh) = layout.resize_to;
        let cw = rw + pad.left + pad.right;
        let ch = rh + pad.top + pad.bottom;
        Layout {
            canvas: (cw, ch),
            placement: (pad.left, pad.top),
            canvas_color: pad.color,
            ..layout
        }
    } else {
        layout
    };

    // 5. Transform the source crop from post-orientation space back to
    //    pre-orientation source coordinates.
    let source_crop_in_source = layout
        .source_crop
        .map(|r| orientation.transform_rect_to_source(r, source_w, source_h));

    let ideal = IdealLayout {
        orientation,
        layout: layout.clone(),
        source_crop: source_crop_in_source,
        padding,
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
    let trim = compute_trim(request, offer, decoder_w, decoder_h);

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
    }
}

/// Compute trim rect when decoder crop doesn't exactly match request.
fn compute_trim(
    request: &DecoderRequest,
    offer: &DecoderOffer,
    decoder_w: u32,
    decoder_h: u32,
) -> Option<Rect> {
    match (&request.crop, &offer.crop_applied) {
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
    use crate::constraint::ConstraintMode;

    // ── No commands ──────────────────────────────────────────────────────

    #[test]
    fn empty_commands_passthrough() {
        let (ideal, req) = plan(&[], 800, 600).unwrap();
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
        assert!(plan(&[], 0, 600).is_err());
        assert!(plan(&[], 800, 0).is_err());
    }

    // ── Orientation only ─────────────────────────────────────────────────

    #[test]
    fn auto_orient_90_swaps_dims() {
        let commands = [Command::AutoOrient(6)]; // EXIF 6 = Rotate90
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_90);
        // Post-orientation: 800×600 rotated 90° → 600×800
        assert_eq!(ideal.layout.resize_to, (600, 800));
        assert_eq!(ideal.layout.canvas, (600, 800));
        assert_eq!(req.orientation, Orientation::ROTATE_90);
    }

    #[test]
    fn auto_orient_180_preserves_dims() {
        let commands = [Command::AutoOrient(3)]; // EXIF 3 = Rotate180
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn stacked_orientation() {
        // EXIF 6 (Rotate90) + manual Rotate90 = Rotate180
        let commands = [Command::AutoOrient(6), Command::Rotate(Rotation::Rotate90)];
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
        assert_eq!(ideal.orientation, Orientation::ROTATE_180);
        // 180° doesn't swap: still 800×600
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn flip_horizontal() {
        let commands = [Command::Flip(FlipAxis::Horizontal)];
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
        assert_eq!(ideal.orientation, Orientation::FLIP_H);
        // FlipH doesn't change dimensions
        assert_eq!(ideal.layout.resize_to, (800, 600));
    }

    #[test]
    fn invalid_exif_ignored() {
        let commands = [Command::AutoOrient(0), Command::AutoOrient(9)];
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();

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
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, _) = plan(&commands, 400, 300).unwrap();
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
        let (ideal, _) = plan(&commands, 800, 400).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();

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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
        let offer = DecoderOffer::full_decode(800, 600);
        let plan = finalize(&ideal, &req, &offer);

        let trim = plan.trim.unwrap();
        assert_eq!(trim, Rect::new(100, 100, 200, 200));
    }

    // ── resize_is_identity ───────────────────────────────────────────────

    #[test]
    fn resize_identity_crop_only() {
        let commands = [Command::Crop(SourceCrop::pixels(0, 0, 400, 300))];
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, req) = plan(&commands, 800, 600).unwrap();
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
        let (ideal, _) = plan(&commands, 800, 600).unwrap();
        // First constraint wins: Fit to 200×200
        assert_eq!(ideal.layout.resize_to, (200, 150));
    }
}
