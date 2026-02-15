# zenlayout

Image layout computation with constraint modes, orientation, and decoder negotiation.

Pure geometry — no pixel operations, no allocations, `no_std` compatible.

## What it does

Given source dimensions and a set of commands (orient, crop, constrain, pad), zenlayout computes every dimension, crop rect, and placement offset needed to produce the output. It handles EXIF orientation, aspect-ratio-aware scaling, codec alignment (JPEG MCU boundaries), and gain map / secondary plane spatial locking.

What it doesn't do: touch pixels. That's your resize engine's job.

## Quick start

```rust
use zenlayout::{Pipeline, DecoderOffer, OutputLimits, Subsampling};

let (ideal, request) = Pipeline::new(4000, 3000)
    .auto_orient(6)            // EXIF orientation 6 = 90° CW
    .fit(800, 600)             // fit within 800×600
    .output_limits(OutputLimits {
        align: Some(Subsampling::S420.mcu_align()),
        ..Default::default()
    })
    .plan()
    .unwrap();

// Pass `request` to your decoder, get back what it actually did
let offer = DecoderOffer::full_decode(4000, 3000);
let plan = ideal.finalize(&request, &offer);

// plan.resize_to, plan.canvas, plan.remaining_orientation, etc.
// contain everything the resize engine needs
```

## Two-phase layout

Layout computation splits into two phases to support decoder negotiation (JPEG prescaling, partial decode, hardware orientation):

```text
    Commands + Source
          │
          ▼
    ┌──────────────┐     ┌──────────────┐
    │compute_layout│────►│DecoderRequest│───► Decoder
    └──────────────┘     └──────────────┘        │
          │                                      │
          ▼                                      ▼
    ┌───────────┐       ┌─────────────┐    ┌───────────┐
    │IdealLayout│──────►│ finalize()  │◄───│DecoderOffer│
    └───────────┘       └─────────────┘    └───────────┘
                              │
                              ▼
                        ┌──────────┐
                        │LayoutPlan│ ── final operations
                        └──────────┘
```

**Phase 1** (`Pipeline::plan()` or `compute_layout()`) computes the ideal layout assuming a full decode. It returns an `IdealLayout` (what the output should look like) and a `DecoderRequest` (hints for the decoder — crop region, target size, orientation).

**Phase 2** (`IdealLayout::finalize()`) takes a `DecoderOffer` describing what the decoder actually did (maybe it prescaled to 1/8, applied orientation, or cropped to MCU boundaries). It compensates for the difference and returns a `LayoutPlan` with the remaining work: what to trim, resize, orient, and place on the canvas.

If your decoder doesn't support any of that, pass `DecoderOffer::full_decode(w, h)`.

## Constraint modes

Eight modes control how source dimensions map to target dimensions:

| Mode | Behavior | Aspect ratio | May upscale |
|------|----------|-------------|-------------|
| `Fit` | Scale to fit within target | Preserved | Yes |
| `Within` | Like Fit, but never upscales | Preserved | No |
| `FitCrop` | Scale to fill target, crop overflow | Preserved | Yes |
| `WithinCrop` | Like FitCrop, but never upscales | Preserved | No |
| `FitPad` | Scale to fit, pad to exact target | Preserved | Yes |
| `WithinPad` | Like FitPad, but never upscales | Preserved | No |
| `Distort` | Scale to exact target dimensions | Stretched | Yes |
| `AspectCrop` | Crop to target aspect ratio, no scaling | Preserved | No |

```text
    Source 4:3, Target 1:1 (square):

    Fit           Within         FitCrop       FitPad
    ┌───┐         ┌───┐          ┌───┐         ┌─────┐
    │   │         │   │          │ █ │         │     │
    │   │         │   │          │ █ │         │ ███ │
    │   │         │   │(smaller) │ █ │         │     │
    └───┘         └───┘          └───┘         └─────┘
    exact size    ≤ source       fills+crops    fits+pads
```

Single-axis constraints are supported: `Constraint::width_only()` and `Constraint::height_only()` derive the other dimension from the source aspect ratio.

## Full API reference

### `Size`

Width × height dimensions in pixels.

```rust
pub struct Size {
    pub width: u32,
    pub height: u32,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `const fn new(width: u32, height: u32) -> Self` | Create a new size |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Rect`

Axis-aligned rectangle in pixel coordinates.

```rust
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `const fn new(x: u32, y: u32, width: u32, height: u32) -> Self` | Create a new rect |
| `clamp_to` | `fn clamp_to(self, max_w: u32, max_h: u32) -> Self` | Clamp to fit within bounds (width/height ≥ 1) |
| `is_full` | `fn is_full(&self, source_w: u32, source_h: u32) -> bool` | Whether this rect covers the full source |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `ConstraintMode`

How to fit a source image into target dimensions. `#[non_exhaustive]`.

| Variant | Description |
|---------|-------------|
| `Distort` | Scale to exact target, distorting aspect ratio |
| `Fit` | Scale to fit within target, preserving aspect ratio. May upscale. |
| `Within` | Like `Fit`, but never upscales |
| `FitCrop` | Scale to fill target, crop overflow |
| `WithinCrop` | Like `FitCrop`, but never upscales |
| `FitPad` | Scale to fit within target, pad to exact target |
| `WithinPad` | Like `FitPad`, but never upscales |
| `AspectCrop` | Crop to target aspect ratio without scaling |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Gravity`

Where to position the image when cropping or padding.

| Variant | Description |
|---------|-------------|
| `Center` | Center on both axes (default) |
| `Percentage(f32, f32)` | Position by `(x, y)` percentage. `(0.0, 0.0)` = top-left, `(1.0, 1.0)` = bottom-right |

Derives: `Copy`, `Clone`, `Debug`, `Default` (`Center`), `PartialEq`

### `CanvasColor`

Canvas background color for pad modes.

| Variant | Description |
|---------|-------------|
| `Transparent` | Transparent black `[0, 0, 0, 0]` (default) |
| `Srgb { r: u8, g: u8, b: u8, a: u8 }` | sRGB color with alpha |
| `Linear { r: f32, g: f32, b: f32, a: f32 }` | Linear RGB color with alpha |

| Method | Signature | Description |
|--------|-----------|-------------|
| `white` | `const fn white() -> Self` | White, fully opaque (`Srgb { 255, 255, 255, 255 }`) |
| `black` | `const fn black() -> Self` | Black, fully opaque (`Srgb { 0, 0, 0, 255 }`) |

Derives: `Copy`, `Clone`, `Debug`, `Default` (`Transparent`), `PartialEq`, `Eq`, `Hash`

### `SourceCrop`

Region of source image to use before applying the constraint.

| Variant | Description |
|---------|-------------|
| `Pixels(Rect)` | Absolute pixel coordinates |
| `Percent { x: f32, y: f32, width: f32, height: f32 }` | Percentage of source dimensions (all `0.0..=1.0`) |

| Method | Signature | Description |
|--------|-----------|-------------|
| `pixels` | `fn pixels(x: u32, y: u32, width: u32, height: u32) -> Self` | Create pixel-based crop |
| `percent` | `fn percent(x: f32, y: f32, width: f32, height: f32) -> Self` | Create percentage-based crop |
| `margin_percent` | `fn margin_percent(margin: f32) -> Self` | Crop equal margins from all edges |
| `margins_percent` | `fn margins_percent(top: f32, right: f32, bottom: f32, left: f32) -> Self` | Crop specific margins (CSS order) |
| `resolve` | `fn resolve(&self, source_w: u32, source_h: u32) -> Rect` | Resolve to pixel coordinates for a given source size |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`

### `Constraint`

Layout constraint specification — how to fit source into target dimensions.

```rust
pub struct Constraint {
    pub mode: ConstraintMode,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub gravity: Gravity,
    pub canvas_color: CanvasColor,
    pub source_crop: Option<SourceCrop>,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new(mode: ConstraintMode, width: u32, height: u32) -> Self` | Constraint with both target dimensions |
| `width_only` | `fn width_only(mode: ConstraintMode, width: u32) -> Self` | Constrain only width (height from aspect ratio) |
| `height_only` | `fn height_only(mode: ConstraintMode, height: u32) -> Self` | Constrain only height (width from aspect ratio) |
| `gravity` | `fn gravity(self, gravity: Gravity) -> Self` | Set crop/pad positioning (builder) |
| `canvas_color` | `fn canvas_color(self, color: CanvasColor) -> Self` | Set pad background color (builder) |
| `source_crop` | `fn source_crop(self, crop: SourceCrop) -> Self` | Set explicit source crop (builder) |
| `compute` | `fn compute(&self, source_w: u32, source_h: u32) -> Result<Layout, LayoutError>` | Compute layout for given source dimensions |

Derives: `Clone`, `Debug`, `PartialEq`

### `Layout`

Computed layout result from applying a `Constraint` to source dimensions.

```text
    ┌─────────────── canvas ───────────────┐
    │                                       │
    │    placement ──┐                      │
    │    (x offset)  │                      │
    │                ▼                      │
    │         ┌── resize_to ──┐             │
    │         │               │             │
    │         │    image      │             │
    │         │               │             │
    │         └───────────────┘             │
    │                                       │
    └───────────────────────────────────────┘

    source_crop ──► resize_to ──► placed on canvas
```

```rust
pub struct Layout {
    pub source: Size,                    // Original source dimensions
    pub source_crop: Option<Rect>,       // Region of source to use (None = full)
    pub resize_to: Size,                 // Dimensions to resize cropped source to
    pub canvas: Size,                    // Final output canvas (≥ resize_to)
    pub placement: (u32, u32),           // Top-left offset of image on canvas
    pub canvas_color: CanvasColor,       // Background color for padding areas
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `needs_resize` | `fn needs_resize(&self) -> bool` | Whether resampling is needed (dimensions change) |
| `needs_padding` | `fn needs_padding(&self) -> bool` | Whether canvas is larger than resized image |
| `needs_crop` | `fn needs_crop(&self) -> bool` | Whether a source crop is applied |
| `effective_source` | `fn effective_source(&self) -> Size` | Source dimensions after crop |

Derives: `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `LayoutError`

Layout computation error.

| Variant | Display message |
|---------|-----------------|
| `ZeroSourceDimension` | "source image has zero width or height" |
| `ZeroTargetDimension` | "target width or height is zero" |

Implements: `Display` (always), `Error` (behind `std` feature)

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`

### `Pipeline`

Builder for image processing pipelines. All operations are in post-orientation coordinates (what the user sees after rotation).

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new(source_w: u32, source_h: u32) -> Self` | Create pipeline for source image |
| `auto_orient` | `fn auto_orient(self, exif: u8) -> Self` | Apply EXIF orientation (1-8). Invalid values ignored. |
| `rotate_90` | `fn rotate_90(self) -> Self` | Rotate 90° CW. Stacks with other orientations. |
| `rotate_180` | `fn rotate_180(self) -> Self` | Rotate 180°. Stacks. |
| `rotate_270` | `fn rotate_270(self) -> Self` | Rotate 270° CW. Stacks. |
| `flip_h` | `fn flip_h(self) -> Self` | Flip horizontally. Stacks. |
| `flip_v` | `fn flip_v(self) -> Self` | Flip vertically. Stacks. |
| `crop_pixels` | `fn crop_pixels(self, x: u32, y: u32, width: u32, height: u32) -> Self` | Crop to pixel coords (first crop wins) |
| `crop_percent` | `fn crop_percent(self, x: f32, y: f32, width: f32, height: f32) -> Self` | Crop using percentages (first crop wins) |
| `crop` | `fn crop(self, source_crop: SourceCrop) -> Self` | Crop with pre-built `SourceCrop` (first crop wins) |
| `fit` | `fn fit(self, width: u32, height: u32) -> Self` | Fit within target (may upscale) |
| `within` | `fn within(self, width: u32, height: u32) -> Self` | Fit within target (never upscales) |
| `fit_crop` | `fn fit_crop(self, width: u32, height: u32) -> Self` | Fill target, crop overflow |
| `within_crop` | `fn within_crop(self, width: u32, height: u32) -> Self` | Fill target, crop overflow, never upscales |
| `fit_pad` | `fn fit_pad(self, width: u32, height: u32) -> Self` | Fit within target, pad to exact |
| `within_pad` | `fn within_pad(self, width: u32, height: u32) -> Self` | Fit within target, pad, never upscales |
| `distort` | `fn distort(self, width: u32, height: u32) -> Self` | Scale to exact target (stretches) |
| `aspect_crop` | `fn aspect_crop(self, width: u32, height: u32) -> Self` | Crop to target aspect ratio, no scaling |
| `constrain` | `fn constrain(self, constraint: Constraint) -> Self` | Apply pre-built `Constraint` (first constraint wins) |
| `pad_uniform` | `fn pad_uniform(self, amount: u32, color: CanvasColor) -> Self` | Add uniform padding on all sides |
| `pad` | `fn pad(self, top: u32, right: u32, bottom: u32, left: u32, color: CanvasColor) -> Self` | Add asymmetric padding (first pad wins) |
| `output_limits` | `fn output_limits(self, limits: OutputLimits) -> Self` | Apply safety limits after layout computation |
| `plan` | `fn plan(self) -> Result<(IdealLayout, DecoderRequest), LayoutError>` | Compute ideal layout (phase 1) |

Derives: `Clone`, `Debug`

### `Command`

Individual processing command for programmatic construction (alternative to `Pipeline`).

| Variant | Fields | Description |
|---------|--------|-------------|
| `AutoOrient(u8)` | EXIF value (1-8) | Apply EXIF orientation correction |
| `Rotate(Rotation)` | | Manual rotation |
| `Flip(FlipAxis)` | | Manual flip |
| `Crop(SourceCrop)` | | Crop in post-orientation coordinates |
| `Constrain` | `{ constraint: Constraint }` | Constrain dimensions |
| `Pad` | `{ top, right, bottom, left: u32, color: CanvasColor }` | Add padding |

Only the first `Crop`, first `Constrain`, and first `Pad` are used; duplicates ignored.

Derives: `Clone`, `Debug`, `PartialEq`

### `Rotation`

| Variant | Description |
|---------|-------------|
| `Rotate90` | 90° clockwise |
| `Rotate180` | 180° |
| `Rotate270` | 270° clockwise (90° counter-clockwise) |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `FlipAxis`

| Variant | Description |
|---------|-------------|
| `Horizontal` | Flip left-right |
| `Vertical` | Flip top-bottom |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `IdealLayout`

Phase 1 result (pre-negotiation).

```rust
pub struct IdealLayout {
    pub orientation: Orientation,         // Net orientation to apply
    pub layout: Layout,                   // Layout in post-orientation space
    pub source_crop: Option<Rect>,        // Source crop in pre-orientation source coords
    pub padding: Option<Padding>,         // Padding to add
    pub content_size: Option<Size>,       // Real content size if Align::Extend was used
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `finalize` | `fn finalize(&self, request: &DecoderRequest, offer: &DecoderOffer) -> LayoutPlan` | Phase 2: compute remaining work after decoder reports |
| `derive_secondary` | `fn derive_secondary(&self, primary_source: Size, secondary_source: Size, secondary_target: Option<Size>) -> (IdealLayout, DecoderRequest)` | Derive spatially-locked plan for gain map / depth map / alpha plane |

Derives: `Clone`, `Debug`, `PartialEq`

### `DecoderRequest`

What the layout engine wants the decoder to do.

```rust
pub struct DecoderRequest {
    pub crop: Option<Rect>,           // Crop region in pre-orientation source coords
    pub target_size: Size,            // Hint for prescale target
    pub orientation: Orientation,     // Orientation to handle
}
```

Derives: `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `DecoderOffer`

What the decoder actually did.

```rust
pub struct DecoderOffer {
    pub dimensions: Size,                         // Decoded output dimensions
    pub crop_applied: Option<Rect>,               // Crop applied (in source coords)
    pub orientation_applied: Orientation,          // Orientation applied
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `full_decode` | `fn full_decode(w: u32, h: u32) -> Self` | Decoder did nothing special, just decoded at full size |

Derives: `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `LayoutPlan`

Final layout plan after decoder negotiation (phase 2 result).

```rust
pub struct LayoutPlan {
    pub decoder_request: DecoderRequest,      // What was requested
    pub trim: Option<Rect>,                   // Trim rect for decoder overshoot
    pub resize_to: Size,                      // Target resize dimensions
    pub remaining_orientation: Orientation,    // Orientation remaining after decoder
    pub canvas: Size,                         // Final canvas dimensions
    pub placement: (u32, u32),                // Image placement on canvas
    pub canvas_color: CanvasColor,            // Background color
    pub resize_is_identity: bool,             // True when no resize needed
    pub content_size: Option<Size>,           // Real content if Align::Extend was used
}
```

Derives: `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Padding`

Explicit padding specification.

```rust
pub struct Padding {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
    pub color: CanvasColor,
}
```

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `OutputLimits`

Post-computation safety limits applied after all layout computation.

```text
    Layout from constraint
          │
          ▼
    ┌─── max ───┐   Scale down proportionally if canvas > max
    │            │
    ▼            │
    ┌─── min ───┐   Scale up proportionally if canvas < min
    │            │   (re-applies max if min overshot -- max wins)
    ▼            │
    ┌── align ──┐   Snap to codec multiples (Crop/Extend/Distort)
    │            │   NOTE: may slightly exceed max or drop below min
    ▼
    Final Layout
```

```rust
pub struct OutputLimits {
    pub max: Option<Size>,       // Security cap — proportional downscale if exceeded
    pub min: Option<Size>,       // Quality floor — proportional upscale if below
    pub align: Option<Align>,    // Snap to codec-required multiples
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `apply` | `fn apply(&self, layout: Layout) -> (Layout, Option<Size>)` | Apply limits. Returns modified layout + optional content_size for `Extend`. |

Implements: `Default` (all `None`)

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Align`

How to align output dimensions to codec-required multiples.

```text
    Source: 801×601, align to mod-16

    Crop:     800×592  --  round down, lose edge pixels
    Extend:   816×608  --  round up, replicate edges, content_size=(801,601)
    Distort:  800×608  --  round to nearest, slight stretch
```

| Variant | Description |
|---------|-------------|
| `Crop(u32, u32)` | Round canvas down per axis. Loses up to `n-1` edge pixels. |
| `Extend(u32, u32)` | Round canvas up per axis. No content loss. Renderer replicates edges. |
| `Distort(u32, u32)` | Round to nearest multiple per axis. Minimal stretch. |

| Method | Signature | Description |
|--------|-----------|-------------|
| `uniform_crop` | `const fn uniform_crop(n: u32) -> Self` | Same alignment for both axes (crop) |
| `uniform_extend` | `const fn uniform_extend(n: u32) -> Self` | Same alignment for both axes (extend) |
| `uniform_distort` | `const fn uniform_distort(n: u32) -> Self` | Same alignment for both axes (distort) |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Subsampling`

Chroma subsampling scheme for JPEG/video codecs.

| Variant | Description | MCU size |
|---------|-------------|----------|
| `S444` | No subsampling. Chroma same as luma. | 8×8 |
| `S422` | Half-width chroma, full height. | 16×8 |
| `S420` | Quarter chroma (half width and height). | 16×16 |

| Method | Signature | Description |
|--------|-----------|-------------|
| `factors` | `const fn factors(self) -> (u32, u32)` | `(h, v)` subsampling factors (ratios, not dimensions) |
| `mcu_size` | `const fn mcu_size(self) -> Size` | MCU dimensions in luma pixels |
| `mcu_align` | `const fn mcu_align(self) -> Align` | `Align::Extend` for JPEG MCU alignment |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `CodecLayout`

Codec-ready geometry for a YCbCr image. Per-plane dimensions, block/MCU grid, and row group size.

```rust
pub struct CodecLayout {
    pub luma: PlaneLayout,           // Luma (Y) plane layout
    pub chroma: PlaneLayout,         // Chroma (Cb, Cr) plane layout (shared geometry)
    pub subsampling: Subsampling,    // Subsampling scheme
    pub mcu_size: Size,              // MCU dimensions in luma pixels
    pub mcu_cols: u32,               // MCUs per row
    pub mcu_rows: u32,               // MCU rows
    pub luma_rows_per_mcu: u32,      // Feed this many rows at a time to the encoder
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new(canvas: Size, subsampling: Subsampling) -> Self` | Compute codec geometry. Canvas should already be MCU-aligned. |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `PlaneLayout`

Geometry for a single image plane (luma or chroma). Block size is always 8×8 (DCT block).

```rust
pub struct PlaneLayout {
    pub content: Size,     // Content dimensions in pixels
    pub extended: Size,    // Allocated/encoded dimensions (extended to block boundary)
    pub blocks_w: u32,     // Number of 8×8 blocks per row
    pub blocks_h: u32,     // Number of 8×8 blocks per column
}
```

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### `Orientation`

Image orientation as an element of the D4 dihedral group — 4 rotations × 2 flip states = 8 elements, matching EXIF orientations 1-8.

```rust
pub struct Orientation {
    pub rotation: u8,    // 0-3: 0°, 90°, 180°, 270° clockwise
    pub flip: bool,      // Horizontal flip applied after rotation
}
```

**Constants:**

| Constant | EXIF | Description |
|----------|------|-------------|
| `IDENTITY` | 1 | No transformation |
| `FLIP_H` | 2 | Horizontal flip |
| `ROTATE_180` | 3 | 180° rotation |
| `FLIP_V` | 4 | Vertical flip |
| `TRANSPOSE` | 5 | Reflect over main diagonal |
| `ROTATE_90` | 6 | 90° clockwise |
| `TRANSVERSE` | 7 | Reflect over anti-diagonal |
| `ROTATE_270` | 8 | 270° clockwise |

| Method | Signature | Description |
|--------|-----------|-------------|
| `from_exif` | `fn from_exif(value: u8) -> Option<Self>` | Create from EXIF tag (1-8). `None` for invalid. |
| `to_exif` | `fn to_exif(self) -> u8` | Convert to EXIF tag (1-8) |
| `is_identity` | `fn is_identity(self) -> bool` | Whether this is the identity |
| `swaps_axes` | `fn swaps_axes(self) -> bool` | Whether width and height swap |
| `compose` | `fn compose(self, other: Self) -> Self` | Apply `self` then `other` (D4 group multiplication) |
| `inverse` | `fn inverse(self) -> Self` | Inverse: `self.compose(self.inverse()) == IDENTITY` |
| `transform_dimensions` | `fn transform_dimensions(self, w: u32, h: u32) -> Size` | Transform source dimensions to display dimensions |
| `transform_rect_to_source` | `fn transform_rect_to_source(self, rect: Rect, source_w: u32, source_h: u32) -> Rect` | Transform display rect back to source coordinates |

Derives: `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`

### Free functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `compute_layout` | `fn compute_layout(commands: &[Command], source_w: u32, source_h: u32, limits: Option<&OutputLimits>) -> Result<(IdealLayout, DecoderRequest), LayoutError>` | Compute layout from command slice (lower-level than `Pipeline`) |

## Orientation

`Orientation` models the D4 dihedral group — 4 rotations × 2 flip states = 8 elements, matching EXIF orientations 1-8. Orientations compose algebraically (verified against the D4 Cayley table):

```rust
use zenlayout::Orientation;

let exif6 = Orientation::from_exif(6).unwrap(); // 90° CW
let combined = exif6.compose(Orientation::FLIP_H);
assert_eq!(combined, Orientation::TRANSPOSE);   // EXIF 5
```

| EXIF | Name | Rotation | Flip |
|------|------|----------|------|
| 1 | Identity | 0° | No |
| 2 | FlipH | 0° | Yes |
| 3 | Rotate180 | 180° | No |
| 4 | FlipV | 180° | Yes |
| 5 | Transpose | 90° | Yes |
| 6 | Rotate90 | 90° | No |
| 7 | Transverse | 270° | Yes |
| 8 | Rotate270 | 270° | No |

Coordinate transforms (`transform_dimensions`, `transform_rect_to_source`) convert between pre-orientation source coordinates and post-orientation display coordinates.

## OutputLimits

Post-computation safety limits applied after all constraint and padding logic:

- **max** is a security cap. Applied first. Proportionally downscales the entire layout (resize_to, canvas, placement, source_crop) if the canvas exceeds either dimension.
- **min** is a quality floor. Applied second. Proportionally upscales if the canvas is below either dimension. If min pushes past max, max is re-applied (max wins).
- **align** snaps to codec-required multiples. Applied last. Because it rounds dimensions, it can slightly exceed max or drop below min — this is by design, since codec alignment is a hard requirement.

## Align modes

Three strategies for snapping to codec multiples:

```text
    Source: 801×601, align to mod-16

    Crop:     800×592  --  round down, lose edge pixels
    Extend:   816×608  --  round up, replicate edges, content_size=(801,601)
    Distort:  800×608  --  round to nearest, slight stretch
```

- `Crop(x, y)` — rounds down per axis. Loses up to `n-1` edge pixels.
- `Extend(x, y)` — rounds up per axis. No content loss. The renderer replicates edge pixels into the extension area. Original content dimensions are stored in `content_size`.
- `Distort(x, y)` — rounds each axis to the nearest multiple. Minimal distortion, no pixel loss, no padding.

`Subsampling::mcu_align()` returns the right `Align::Extend` for JPEG MCU alignment.

## Codec layout

`CodecLayout` computes per-plane geometry for YCbCr encoders:

```rust
use zenlayout::{CodecLayout, Subsampling, Size};

let codec = CodecLayout::new(Size::new(800, 608), Subsampling::S420);

// Luma plane
assert_eq!(codec.luma.extended, Size::new(800, 608));
assert_eq!(codec.luma.blocks_w, 100); // 800 / 8

// Chroma plane (half resolution for 4:2:0)
assert_eq!(codec.chroma.extended, Size::new(400, 304));

// MCU grid
assert_eq!(codec.mcu_size, Size::new(16, 16));
assert_eq!(codec.mcu_cols, 50);

// Feed rows in chunks of this size to avoid intermediate buffering
assert_eq!(codec.luma_rows_per_mcu, 16);
```

Supports `S444` (no subsampling, 8×8 MCU), `S422` (half-width chroma, 16×8 MCU), and `S420` (quarter chroma, 16×16 MCU).

## Secondary planes

For gain maps, depth maps, or alpha planes that share spatial extent with the primary image but live at a different resolution:

```rust
use zenlayout::{Pipeline, DecoderOffer, Size};

// SDR: 4000×3000, gain map: 1000×750 (1/4 scale)
let (sdr_ideal, sdr_req) = Pipeline::new(4000, 3000)
    .auto_orient(6)
    .crop_pixels(100, 100, 2000, 2000)
    .fit(800, 800)
    .plan()
    .unwrap();

// Derive gain map plan — automatically maintains the source ratio
let (gm_ideal, gm_req) = sdr_ideal.derive_secondary(
    Size::new(4000, 3000),  // primary source
    Size::new(1000, 750),   // gain map source
    None,                   // auto: 1/4 of SDR output
);

// Each decoder independently handles its capabilities
let sdr_plan = sdr_ideal.finalize(&sdr_req, &DecoderOffer::full_decode(4000, 3000));
let gm_plan = gm_ideal.finalize(&gm_req, &DecoderOffer::full_decode(1000, 750));

// Both plans produce spatially-locked results
assert_eq!(sdr_plan.remaining_orientation, gm_plan.remaining_orientation);
```

Source crop coordinates are scaled from primary to secondary space with round-outward logic (origin floors, extent ceils) to ensure full spatial coverage.

## Feature flags

| Flag | Default | Description |
|------|---------|-------------|
| `std` | Yes | Standard library support. Disable for `no_std` environments. Enables `Error` impl for `LayoutError`. |

The crate uses `#![forbid(unsafe_code)]` and makes zero heap allocations.

## License

AGPL-3.0-or-later
