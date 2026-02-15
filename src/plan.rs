//! Command pipeline and decoder negotiation.

use crate::constraint::{CanvasColor, Constraint, Layout, Rect, SourceCrop};
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
