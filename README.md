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

Coordinate transforms (`transform_point`, `transform_rect`, `transform_dimensions`) convert between pre-orientation source coordinates and post-orientation user coordinates. The inverse transforms convert back.

## OutputLimits

Post-computation safety limits applied after all constraint and padding logic:

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

## API summary

### constraint module

| Type | Description |
|------|-------------|
| `ConstraintMode` | How to fit source into target (Fit, Within, FitCrop, etc.) |
| `Constraint` | Mode + target dimensions + gravity + canvas color + source crop |
| `Layout` | Computed result: source, resize_to, canvas, placement, source_crop |
| `Size` | Width × height pair |
| `Rect` | x, y, width, height rectangle |
| `SourceCrop` | Pixel or percentage crop specification |
| `Gravity` | Positioning for crop/pad (Center or Percentage) |
| `CanvasColor` | Background color (Transparent, Srgb, Linear) |
| `LayoutError` | Zero source or target dimensions |

### plan module

| Type | Description |
|------|-------------|
| `Pipeline` | Builder for command sequences |
| `Command` | Individual processing command |
| `IdealLayout` | Phase 1 result (pre-negotiation) |
| `DecoderRequest` | What the decoder should try to do |
| `DecoderOffer` | What the decoder actually did |
| `LayoutPlan` | Phase 2 result (post-negotiation) |
| `OutputLimits` | Post-computation max/min/align limits |
| `Align` | Codec alignment strategy (Crop, Extend, Distort) |
| `Subsampling` | Chroma subsampling scheme (S444, S422, S420) |
| `CodecLayout` | Per-plane geometry for YCbCr encoders |
| `PlaneLayout` | Single plane content/extended dimensions + block grid |
| `Padding` | Explicit padding (top, right, bottom, left, color) |
| `Rotation` | Manual rotation amount (90, 180, 270) |
| `FlipAxis` | Manual flip axis (Horizontal, Vertical) |

### orientation module

| Type | Description |
|------|-------------|
| `Orientation` | D4 dihedral group element (rotation + optional flip) |

### Free functions

| Function | Description |
|----------|-------------|
| `compute_layout()` | Compute layout from command slice (lower-level than Pipeline) |

## Feature flags

| Flag | Default | Description |
|------|---------|-------------|
| `std` | Yes | Standard library support. Disable for `no_std` environments. |

The crate uses `#![forbid(unsafe_code)]` and makes zero heap allocations.

## License

AGPL-3.0-or-later
