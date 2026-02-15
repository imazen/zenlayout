//! Orientation (D4 dihedral group), EXIF mapping, and coordinate transforms.

use crate::constraint::{Rect, Size};

/// Image orientation as an element of the D4 dihedral group.
///
/// Represents a rotation (0, 90, 180, 270 degrees clockwise) optionally
/// followed by a horizontal flip. All 8 EXIF orientations map to this.
///
/// The composition rule matches the D4 Cayley table verified against
/// zenjpeg's `coeff_transform.rs`.
///
/// ```text
///     EXIF orientations and their transforms:
///
///     1: Identity    2: FlipH       3: Rotate180   4: FlipV
///     ┌───┐          ┌───┐          ┌───┐          ┌───┐
///     │ F │          │ Ꟊ │          │   │          │   │
///     │   │          │   │          │ Ꟊ │          │ F │
///     └───┘          └───┘          └───┘          └───┘
///
///     5: Transpose   6: Rotate90    7: Transverse  8: Rotate270
///     ┌────┐         ┌────┐         ┌────┐         ┌────┐
///     │ F  │         │  F │         │  Ꟊ │         │ Ꟊ  │
///     └────┘         └────┘         └────┘         └────┘
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Orientation {
    /// Rotation in 90-degree increments (0-3). 0=0°, 1=90°, 2=180°, 3=270°.
    pub rotation: u8,
    /// Horizontal flip applied after rotation.
    pub flip: bool,
}

impl Orientation {
    /// Identity (no transformation). EXIF 1.
    pub const IDENTITY: Self = Self {
        rotation: 0,
        flip: false,
    };
    /// Horizontal flip. EXIF 2.
    pub const FLIP_H: Self = Self {
        rotation: 0,
        flip: true,
    };
    /// 180° rotation. EXIF 3.
    pub const ROTATE_180: Self = Self {
        rotation: 2,
        flip: false,
    };
    /// Vertical flip. EXIF 4.
    pub const FLIP_V: Self = Self {
        rotation: 2,
        flip: true,
    };
    /// Transpose (reflect over main diagonal). EXIF 5.
    pub const TRANSPOSE: Self = Self {
        rotation: 1,
        flip: true,
    };
    /// 90° clockwise rotation. EXIF 6.
    pub const ROTATE_90: Self = Self {
        rotation: 1,
        flip: false,
    };
    /// Transverse (reflect over anti-diagonal). EXIF 7.
    pub const TRANSVERSE: Self = Self {
        rotation: 3,
        flip: true,
    };
    /// 270° clockwise rotation (90° counter-clockwise). EXIF 8.
    pub const ROTATE_270: Self = Self {
        rotation: 3,
        flip: false,
    };

    /// All 8 elements of the D4 group, indexed by EXIF value - 1.
    const ALL: [Self; 8] = [
        Self::IDENTITY,
        Self::FLIP_H,
        Self::ROTATE_180,
        Self::FLIP_V,
        Self::TRANSPOSE,
        Self::ROTATE_90,
        Self::TRANSVERSE,
        Self::ROTATE_270,
    ];

    /// Create from EXIF orientation tag (1-8). Returns `None` for invalid values.
    pub fn from_exif(value: u8) -> Option<Self> {
        if (1..=8).contains(&value) {
            Some(Self::ALL[(value - 1) as usize])
        } else {
            None
        }
    }

    /// Convert to EXIF orientation tag (1-8).
    pub fn to_exif(self) -> u8 {
        // Search the ALL array for a match
        for (i, &o) in Self::ALL.iter().enumerate() {
            if o.rotation == self.rotation && o.flip == self.flip {
                return (i + 1) as u8;
            }
        }
        // rotation is masked to 0-3 in compose, so this is reachable only
        // if someone constructs an Orientation with rotation > 3
        1
    }

    /// Whether this is the identity transformation.
    pub fn is_identity(self) -> bool {
        self.rotation == 0 && !self.flip
    }

    /// Whether this orientation swaps width and height.
    pub fn swaps_axes(self) -> bool {
        self.rotation % 2 == 1
    }

    /// Compose two orientations: apply `self` first, then `other`.
    ///
    /// This follows the D4 group multiplication rule verified against
    /// the Cayley table in zenjpeg's `coeff_transform.rs`.
    pub fn compose(self, other: Self) -> Self {
        if !self.flip {
            Self {
                rotation: (self.rotation + other.rotation) & 3,
                flip: other.flip,
            }
        } else {
            Self {
                rotation: (self.rotation.wrapping_sub(other.rotation)) & 3,
                flip: !other.flip,
            }
        }
    }

    /// The inverse orientation: `self.compose(self.inverse()) == IDENTITY`.
    pub fn inverse(self) -> Self {
        if !self.flip {
            Self {
                rotation: (4 - self.rotation) & 3,
                flip: false,
            }
        } else {
            // Flips are self-inverse, but rotation direction reverses under flip
            self
        }
    }

    /// Transform source dimensions to display dimensions.
    pub fn transform_dimensions(self, w: u32, h: u32) -> Size {
        if self.swaps_axes() {
            Size::new(h, w)
        } else {
            Size::new(w, h)
        }
    }

    /// Transform a rectangle from display coordinates back to source coordinates.
    ///
    /// Given a rect in post-orientation (display) space and the source image
    /// dimensions, returns the corresponding rect in pre-orientation (source) space.
    pub fn transform_rect_to_source(self, rect: Rect, source_w: u32, source_h: u32) -> Rect {
        let (rx, ry, rw, rh) = (rect.x, rect.y, rect.width, rect.height);
        let (sw, sh) = (source_w, source_h);

        match (self.rotation, self.flip) {
            // Identity
            (0, false) => Rect::new(rx, ry, rw, rh),
            // FlipH
            (0, true) => Rect::new(sw - rx - rw, ry, rw, rh),
            // Rotate90
            (1, false) => Rect::new(ry, sh - rx - rw, rh, rw),
            // Transpose
            (1, true) => Rect::new(ry, rx, rh, rw),
            // Rotate180
            (2, false) => Rect::new(sw - rx - rw, sh - ry - rh, rw, rh),
            // FlipV
            (2, true) => Rect::new(rx, sh - ry - rh, rw, rh),
            // Rotate270
            (3, false) => Rect::new(sw - ry - rh, rx, rh, rw),
            // Transverse
            (3, true) => Rect::new(sw - ry - rh, sh - rx - rw, rh, rw),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_round_trip() {
        for v in 1..=8u8 {
            let o = Orientation::from_exif(v).unwrap();
            assert_eq!(o.to_exif(), v, "round-trip failed for EXIF {v}");
        }
    }

    #[test]
    fn exif_invalid() {
        assert!(Orientation::from_exif(0).is_none());
        assert!(Orientation::from_exif(9).is_none());
        assert!(Orientation::from_exif(255).is_none());
    }

    #[test]
    fn exif_mapping_matches_spec() {
        // Verified against zenjpeg exif.rs:168-180
        assert_eq!(Orientation::from_exif(1).unwrap(), Orientation::IDENTITY);
        assert_eq!(Orientation::from_exif(2).unwrap(), Orientation::FLIP_H);
        assert_eq!(Orientation::from_exif(3).unwrap(), Orientation::ROTATE_180);
        assert_eq!(Orientation::from_exif(4).unwrap(), Orientation::FLIP_V);
        assert_eq!(Orientation::from_exif(5).unwrap(), Orientation::TRANSPOSE);
        assert_eq!(Orientation::from_exif(6).unwrap(), Orientation::ROTATE_90);
        assert_eq!(Orientation::from_exif(7).unwrap(), Orientation::TRANSVERSE);
        assert_eq!(Orientation::from_exif(8).unwrap(), Orientation::ROTATE_270);
    }

    #[test]
    fn identity_properties() {
        assert!(Orientation::IDENTITY.is_identity());
        assert!(!Orientation::FLIP_H.is_identity());
        assert!(!Orientation::ROTATE_90.is_identity());
    }

    #[test]
    fn swaps_axes() {
        assert!(!Orientation::IDENTITY.swaps_axes());
        assert!(!Orientation::FLIP_H.swaps_axes());
        assert!(!Orientation::ROTATE_180.swaps_axes());
        assert!(!Orientation::FLIP_V.swaps_axes());
        assert!(Orientation::TRANSPOSE.swaps_axes());
        assert!(Orientation::ROTATE_90.swaps_axes());
        assert!(Orientation::TRANSVERSE.swaps_axes());
        assert!(Orientation::ROTATE_270.swaps_axes());
    }

    #[test]
    fn transform_dimensions() {
        use crate::constraint::Size;
        assert_eq!(
            Orientation::IDENTITY.transform_dimensions(100, 200),
            Size::new(100, 200)
        );
        assert_eq!(
            Orientation::FLIP_H.transform_dimensions(100, 200),
            Size::new(100, 200)
        );
        assert_eq!(
            Orientation::ROTATE_180.transform_dimensions(100, 200),
            Size::new(100, 200)
        );
        assert_eq!(
            Orientation::FLIP_V.transform_dimensions(100, 200),
            Size::new(100, 200)
        );
        assert_eq!(
            Orientation::TRANSPOSE.transform_dimensions(100, 200),
            Size::new(200, 100)
        );
        assert_eq!(
            Orientation::ROTATE_90.transform_dimensions(100, 200),
            Size::new(200, 100)
        );
        assert_eq!(
            Orientation::TRANSVERSE.transform_dimensions(100, 200),
            Size::new(200, 100)
        );
        assert_eq!(
            Orientation::ROTATE_270.transform_dimensions(100, 200),
            Size::new(200, 100)
        );
    }

    /// Verify the full D4 Cayley table against zenjpeg's coeff_transform.rs.
    ///
    /// The Cayley table from zenjpeg uses indices:
    /// 0=None, 1=FlipH, 2=FlipV, 3=Transpose, 4=Rotate90, 5=Rotate180, 6=Rotate270, 7=Transverse
    ///
    /// Our EXIF-ordered ALL array uses:
    /// 0=Identity, 1=FlipH, 2=Rotate180, 3=FlipV, 4=Transpose, 5=Rotate90, 6=Transverse, 7=Rotate270
    ///
    /// So we need a mapping between the two index orders.
    #[test]
    fn cayley_table() {
        // zenjpeg Cayley table (from coeff_transform.rs:130-140)
        // Index order: None=0, FlipH=1, FlipV=2, Transpose=3, Rot90=4, Rot180=5, Rot270=6, Transverse=7
        #[rustfmt::skip]
        const CAYLEY: [[usize; 8]; 8] = [
            [0,1,2,3,4,5,6,7], // None
            [1,0,5,6,7,2,3,4], // FlipH
            [2,5,0,4,3,1,7,6], // FlipV (note: not at EXIF index 2)
            [3,4,6,0,1,7,2,5], // Transpose
            [4,3,7,2,5,6,0,1], // Rotate90
            [5,2,1,7,6,0,4,3], // Rotate180
            [6,7,3,1,0,4,5,2], // Rotate270
            [7,6,4,5,2,3,1,0], // Transverse
        ];

        // zenjpeg index order to Orientation
        let zj_to_orient = [
            Orientation::IDENTITY,   // 0 = None
            Orientation::FLIP_H,     // 1 = FlipH
            Orientation::FLIP_V,     // 2 = FlipV
            Orientation::TRANSPOSE,  // 3 = Transpose
            Orientation::ROTATE_90,  // 4 = Rotate90
            Orientation::ROTATE_180, // 5 = Rotate180
            Orientation::ROTATE_270, // 6 = Rotate270
            Orientation::TRANSVERSE, // 7 = Transverse
        ];

        for (i, row) in CAYLEY.iter().enumerate() {
            for (j, &expected_idx) in row.iter().enumerate() {
                let a = zj_to_orient[i];
                let b = zj_to_orient[j];
                let expected = zj_to_orient[expected_idx];
                let got = a.compose(b);
                assert_eq!(
                    got, expected,
                    "Cayley mismatch: {a:?}.compose({b:?}) = {got:?}, expected {expected:?}"
                );
            }
        }
    }

    #[test]
    fn inverse_all() {
        let all = Orientation::ALL;
        for &o in &all {
            let inv = o.inverse();
            assert_eq!(
                o.compose(inv),
                Orientation::IDENTITY,
                "{o:?}.compose({inv:?}) should be IDENTITY"
            );
            assert_eq!(
                inv.compose(o),
                Orientation::IDENTITY,
                "{inv:?}.compose({o:?}) should be IDENTITY"
            );
        }
    }

    #[test]
    fn associativity() {
        let all = Orientation::ALL;
        for &a in &all {
            for &b in &all {
                for &c in &all {
                    let ab_c = a.compose(b).compose(c);
                    let a_bc = a.compose(b.compose(c));
                    assert_eq!(
                        ab_c, a_bc,
                        "associativity failed: ({a:?}*{b:?})*{c:?} != {a:?}*({b:?}*{c:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn identity_is_neutral() {
        let id = Orientation::IDENTITY;
        for &o in &Orientation::ALL {
            assert_eq!(id.compose(o), o);
            assert_eq!(o.compose(id), o);
        }
    }

    #[test]
    fn transform_rect_identity() {
        let rect = Rect::new(10, 20, 30, 40);
        let result = Orientation::IDENTITY.transform_rect_to_source(rect, 100, 200);
        assert_eq!(result, rect);
    }

    #[test]
    fn transform_rect_full_image() {
        // Full image rect should map to full source rect for all orientations
        for &o in &Orientation::ALL {
            let d = o.transform_dimensions(100, 200);
            let display_rect = Rect::new(0, 0, d.width, d.height);
            let source_rect = o.transform_rect_to_source(display_rect, 100, 200);
            assert_eq!(
                (source_rect.x, source_rect.y),
                (0, 0),
                "full image rect origin for {o:?}"
            );
            assert_eq!(
                (source_rect.width, source_rect.height),
                (100, 200),
                "full image rect dims for {o:?}"
            );
        }
    }

    #[test]
    fn transform_rect_1x1_at_corners() {
        // Test 1x1 pixel rects at all 4 corners of a 4x3 image
        let (sw, sh) = (4u32, 3u32);
        let corners = [
            (0, 0), // top-left
            (3, 0), // top-right (sw-1, 0)
            (0, 2), // bottom-left (0, sh-1)
            (3, 2), // bottom-right (sw-1, sh-1)
        ];

        for &o in &Orientation::ALL {
            let d = o.transform_dimensions(sw, sh);
            for &(sx, sy) in &corners {
                // Forward-map this source pixel to display coords
                let (dx, dy) = forward_map_point(o, sx, sy, sw, sh);
                assert!(
                    dx < d.width && dy < d.height,
                    "forward mapped ({sx},{sy}) to ({dx},{dy}) but display is {}x{} for {o:?}",
                    d.width,
                    d.height
                );

                // Now transform_rect_to_source should give us back the source pixel
                let display_rect = Rect::new(dx, dy, 1, 1);
                let source_rect = o.transform_rect_to_source(display_rect, sw, sh);
                assert_eq!(
                    (
                        source_rect.x,
                        source_rect.y,
                        source_rect.width,
                        source_rect.height
                    ),
                    (sx, sy, 1, 1),
                    "round-trip failed for source ({sx},{sy}) display ({dx},{dy}) orient {o:?}"
                );
            }
        }
    }

    #[test]
    fn transform_rect_brute_force_4x3() {
        // Test every single-pixel rect in a 4x3 image
        let (sw, sh) = (4u32, 3u32);
        for &o in &Orientation::ALL {
            let d = o.transform_dimensions(sw, sh);
            for sx in 0..sw {
                for sy in 0..sh {
                    let (dx, dy) = forward_map_point(o, sx, sy, sw, sh);
                    let display_rect = Rect::new(dx, dy, 1, 1);
                    let source_rect = o.transform_rect_to_source(display_rect, sw, sh);
                    assert_eq!(
                        (source_rect.x, source_rect.y),
                        (sx, sy),
                        "pixel ({sx},{sy}) via {o:?}: display ({dx},{dy}) in {}x{}, got back ({},{})",
                        d.width,
                        d.height,
                        source_rect.x,
                        source_rect.y
                    );
                }
            }
        }
    }

    #[test]
    fn transform_rect_multi_pixel() {
        // Test a 2x2 rect in a 4x3 image for all orientations
        let (sw, sh) = (4u32, 3u32);
        let rect = Rect::new(1, 1, 2, 2);

        // For identity: source rect is (1,1,2,2), display rect is same
        let result = Orientation::IDENTITY.transform_rect_to_source(rect, sw, sh);
        assert_eq!(result, rect);

        // For all orientations: forward-map all 4 pixels in the 2x2 block,
        // find bounding box in display, that should be what maps back
        for &o in &Orientation::ALL {
            // Forward-map corner pixels
            let (dx0, dy0) = forward_map_point(o, rect.x, rect.y, sw, sh);
            let (dx1, dy1) = forward_map_point(o, rect.x + rect.width - 1, rect.y, sw, sh);
            let (dx2, dy2) = forward_map_point(o, rect.x, rect.y + rect.height - 1, sw, sh);
            let (dx3, dy3) =
                forward_map_point(o, rect.x + rect.width - 1, rect.y + rect.height - 1, sw, sh);

            let min_x = dx0.min(dx1).min(dx2).min(dx3);
            let min_y = dy0.min(dy1).min(dy2).min(dy3);
            let max_x = dx0.max(dx1).max(dx2).max(dx3);
            let max_y = dy0.max(dy1).max(dy2).max(dy3);

            let display_rect = Rect::new(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1);

            let source_rect = o.transform_rect_to_source(display_rect, sw, sh);
            assert_eq!(
                source_rect, rect,
                "multi-pixel rect {rect:?} via {o:?}: display {display_rect:?} → source {source_rect:?}"
            );
        }
    }

    /// Forward-map a source pixel to display coordinates.
    /// Verified against zenjpeg coeff_transform.rs:89-97.
    fn forward_map_point(o: Orientation, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
        match (o.rotation, o.flip) {
            (0, false) => (x, y),                 // Identity
            (0, true) => (w - 1 - x, y),          // FlipH
            (1, false) => (h - 1 - y, x),         // Rotate90
            (1, true) => (y, x),                  // Transpose
            (2, false) => (w - 1 - x, h - 1 - y), // Rotate180
            (2, true) => (x, h - 1 - y),          // FlipV
            (3, false) => (y, w - 1 - x),         // Rotate270
            (3, true) => (h - 1 - y, w - 1 - x),  // Transverse
            _ => unreachable!(),
        }
    }
}
