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
    CanvasColor, Constraint, ConstraintMode, Gravity, Layout, LayoutError, Rect, SourceCrop,
};
pub use orientation::Orientation;
pub use plan::{
    Command, DecoderOffer, DecoderRequest, FlipAxis, IdealLayout, LayoutPlan, Padding, Pipeline,
    Rotation, finalize, plan,
};
