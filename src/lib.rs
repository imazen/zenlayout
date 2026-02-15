//! Image layout computation with constraint modes, orientation, and decoder negotiation.
//!
//! Pure geometry — no pixel operations, no allocations, `no_std` compatible.
//!
//! # Modules
//!
//! - [`constraint`] — Constraint modes (Fit, Within, FitCrop, etc.) and layout computation
//! - [`orientation`] — EXIF orientation, D4 dihedral group, coordinate transforms
//! - [`plan`] — Command pipeline, decoder negotiation, two-phase layout planning

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

pub mod constraint;
pub mod orientation;
pub mod plan;

// Re-exports: core types from constraint module
pub use constraint::{
    CanvasColor, Constraint, ConstraintMode, Gravity, Layout, LayoutError, Rect, Size, SourceCrop,
};
pub use orientation::Orientation;
pub use plan::{
    Align, CodecLayout, Command, DecoderOffer, DecoderRequest, FlipAxis, IdealLayout, LayoutPlan,
    OutputLimits, Padding, Pipeline, PlaneLayout, Region, RegionCoord, Rotation, Subsampling,
    compute_layout, compute_layout_sequential,
};
