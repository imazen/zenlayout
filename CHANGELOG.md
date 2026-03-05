# Changelog

## Unreleased (since v0.1.0)

### Added
- **Smart crop module** for content-aware cropping (`smart_crop`)
- **RIAPI query string parsing** — Phase 1 (`riapi` feature)
  - Comprehensive parity tests against legacy RIAPI behavior
- **SVG visualization** of layout pipeline steps (`svg` feature)
  - Region, rotation, and orientation SVG examples
  - Improved visual language with size variation
- Layout pipeline examples with SVG diagrams (`doc/layout-examples.md`)
- `#[non_exhaustive]` + `Default` on all public structs (`DecoderRequest`, `DecoderOffer`, `LayoutPlan`, etc.)
- `min_precise_decode_size` field on `DecoderRequest`
- Decoder hint promotion to first-class RIAPI fields
- AGPL-3.0-or-later license file
- Comprehensive CI: 6-platform matrix, i686 cross-compilation, WASM, Codecov

### Changed
- Removed `decoder.min_precise_scaling_ratio` and `jpeg_idct_downscale_linear`
- Removed decoder prescaling hints from `DecoderRequest`
- Edition 2024, MSRV 1.89

## 0.1.0

Initial release. Pure geometry image layout computation with constraint modes,
EXIF orientation (D4 dihedral group), and decoder negotiation. `no_std + alloc`,
`#![forbid(unsafe_code)]`.
