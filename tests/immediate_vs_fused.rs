//! Immediate-mode pixel simulation vs fused layout computation.
//!
//! Every pixel in the source stores its (x, y) origin coordinates, making
//! any geometric error immediately detectable — wrong crop, wrong scale,
//! wrong placement all show up as mismatched coordinates.
//!
//! "Immediate mode" = apply each command to actual pixel data, one step at a
//! time, like a user would intuitively expect.
//!
//! "Fused mode" = compute a single Layout via compute_layout_sequential(),
//! then apply that Layout in one shot.
//!
//! Mismatches reveal where the fused layout abstraction can't faithfully
//! represent the sequential operation.

use zenlayout::*;

// ---- Pixel simulation ----

/// A pixel that remembers where it came from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Pixel {
    /// Source pixel at (x, y) in the original image.
    Source(u32, u32),
    /// Fill / padding pixel.
    Fill,
}

/// A pixel buffer for geometric validation.
#[derive(Clone, Debug)]
struct Grid {
    width: u32,
    height: u32,
    pixels: Vec<Pixel>,
}

impl Grid {
    /// Source image: pixel at (x,y) stores Source(x,y).
    fn source(w: u32, h: u32) -> Self {
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| Pixel::Source(x, y)))
            .collect();
        Self {
            width: w,
            height: h,
            pixels,
        }
    }

    fn get(&self, x: u32, y: u32) -> Pixel {
        assert!(
            x < self.width && y < self.height,
            "({x},{y}) out of bounds {}x{}",
            self.width,
            self.height
        );
        self.pixels[(y * self.width + x) as usize]
    }

    /// Crop: extract sub-rectangle. Clamps to bounds.
    fn crop(&self, cx: u32, cy: u32, cw: u32, ch: u32) -> Self {
        let cx = cx.min(self.width);
        let cy = cy.min(self.height);
        let cw = cw.min(self.width.saturating_sub(cx));
        let ch = ch.min(self.height.saturating_sub(cy));
        let mut pixels = Vec::with_capacity((cw * ch) as usize);
        for y in cy..cy + ch {
            for x in cx..cx + cw {
                pixels.push(self.get(x, y));
            }
        }
        Self {
            width: cw,
            height: ch,
            pixels,
        }
    }

    /// Nearest-neighbor resize.
    fn resize_nn(&self, new_w: u32, new_h: u32) -> Self {
        assert!(new_w > 0 && new_h > 0);
        if new_w == self.width && new_h == self.height {
            return self.clone();
        }
        let mut pixels = Vec::with_capacity((new_w * new_h) as usize);
        for y in 0..new_h {
            let src_y =
                ((y as f64 + 0.5) * self.height as f64 / new_h as f64).floor() as u32;
            let src_y = src_y.min(self.height - 1);
            for x in 0..new_w {
                let src_x =
                    ((x as f64 + 0.5) * self.width as f64 / new_w as f64).floor() as u32;
                let src_x = src_x.min(self.width - 1);
                pixels.push(self.get(src_x, src_y));
            }
        }
        Self {
            width: new_w,
            height: new_h,
            pixels,
        }
    }

    /// Place this grid at (px, py) on a canvas. Handles negative offsets (clipping).
    fn place_on_canvas(&self, cw: u32, ch: u32, px: i32, py: i32) -> Self {
        let mut pixels = vec![Pixel::Fill; (cw * ch) as usize];
        for sy in 0..self.height {
            let dy = py + sy as i32;
            if dy < 0 || dy >= ch as i32 {
                continue;
            }
            for sx in 0..self.width {
                let dx = px + sx as i32;
                if dx < 0 || dx >= cw as i32 {
                    continue;
                }
                pixels[(dy as u32 * cw + dx as u32) as usize] = self.get(sx, sy);
            }
        }
        Self {
            width: cw,
            height: ch,
            pixels,
        }
    }

    /// Add padding around the image.
    fn pad(&self, top: u32, right: u32, bottom: u32, left: u32) -> Self {
        let cw = self.width + left + right;
        let ch = self.height + top + bottom;
        self.place_on_canvas(cw, ch, left as i32, top as i32)
    }

    /// Flip horizontally.
    fn flip_h(&self) -> Self {
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for y in 0..self.height {
            for x in (0..self.width).rev() {
                pixels.push(self.get(x, y));
            }
        }
        Self {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// Flip vertically.
    fn flip_v(&self) -> Self {
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for y in (0..self.height).rev() {
            for x in 0..self.width {
                pixels.push(self.get(x, y));
            }
        }
        Self {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// Rotate 90° clockwise.
    fn rotate_90(&self) -> Self {
        let new_w = self.height;
        let new_h = self.width;
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for y in 0..new_h {
            for x in 0..new_w {
                pixels.push(self.get(y, new_w - 1 - x));
            }
        }
        Self {
            width: new_w,
            height: new_h,
            pixels,
        }
    }

    /// Rotate 180°.
    fn rotate_180(&self) -> Self {
        let mut pixels = self.pixels.clone();
        pixels.reverse();
        Self {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// Rotate 270° clockwise.
    fn rotate_270(&self) -> Self {
        let new_w = self.height;
        let new_h = self.width;
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for y in 0..new_h {
            for x in 0..new_w {
                pixels.push(self.get(new_h - 1 - y, x));
            }
        }
        Self {
            width: new_w,
            height: new_h,
            pixels,
        }
    }

    /// Apply an Orientation transform.
    fn apply_orientation(&self, o: Orientation) -> Self {
        match o {
            Orientation::Identity => self.clone(),
            Orientation::FlipH => self.flip_h(),
            Orientation::Rotate180 => self.rotate_180(),
            Orientation::FlipV => self.flip_v(),
            Orientation::Transpose => self.rotate_90().flip_h(),
            Orientation::Rotate90 => self.rotate_90(),
            Orientation::Transverse => self.rotate_270().flip_h(),
            Orientation::Rotate270 => self.rotate_270(),
            _ => panic!("unknown orientation variant"),
        }
    }

    /// Apply a Region viewport: crop source overlap, place on viewport canvas.
    fn apply_region(&self, reg: &Region) -> Self {
        let left = reg.left.resolve(self.width);
        let top = reg.top.resolve(self.height);
        let right = reg.right.resolve(self.width);
        let bottom = reg.bottom.resolve(self.height);

        let vw = (right - left).max(0) as u32;
        let vh = (bottom - top).max(0) as u32;
        if vw == 0 || vh == 0 {
            return Self {
                width: 0,
                height: 0,
                pixels: vec![],
            };
        }

        // Compute source overlap
        let ol = left.max(0) as u32;
        let ot = top.max(0) as u32;
        let or_ = (right.min(self.width as i32)).max(0) as u32;
        let ob = (bottom.min(self.height as i32)).max(0) as u32;

        if ol >= or_ || ot >= ob {
            // No overlap — blank canvas
            return Self {
                width: vw,
                height: vh,
                pixels: vec![Pixel::Fill; (vw * vh) as usize],
            };
        }

        // Crop the source overlap
        let overlap = self.crop(ol, ot, or_ - ol, ob - ot);
        // Place on viewport canvas
        let place_x = ol as i32 - left;
        let place_y = ot as i32 - top;
        overlap.place_on_canvas(vw, vh, place_x, place_y)
    }

    /// Apply a fused Layout to the original source grid.
    fn apply_layout(&self, layout: &Layout) -> Self {
        // 1. Crop
        let cropped = if let Some(sc) = &layout.source_crop {
            self.crop(sc.x, sc.y, sc.width, sc.height)
        } else {
            self.clone()
        };

        // 2. Resize
        let resized = cropped.resize_nn(layout.resize_to.width, layout.resize_to.height);

        // 3. Place on canvas
        let (px, py) = layout.placement;
        resized.place_on_canvas(layout.canvas.width, layout.canvas.height, px, py)
    }

    fn summary(&self) -> String {
        let mut s = format!("{}x{}\n", self.width, self.height);
        for y in 0..self.height.min(16) {
            for x in 0..self.width.min(16) {
                match self.get(x, y) {
                    Pixel::Source(sx, sy) => s.push_str(&format!("({sx:2},{sy:2}) ")),
                    Pixel::Fill => s.push_str("  ..   "),
                }
            }
            s.push('\n');
        }
        if self.width > 16 || self.height > 16 {
            s.push_str("...(truncated)\n");
        }
        s
    }
}

/// Immediate-mode: apply a sequence of commands to pixels step by step.
fn immediate_eval(source: &Grid, commands: &[Command]) -> Grid {
    let mut current = source.clone();

    for cmd in commands {
        match cmd {
            Command::AutoOrient(exif) => {
                if let Some(o) = Orientation::from_exif(*exif) {
                    current = current.apply_orientation(o);
                }
            }
            Command::Rotate(r) => {
                let o = match r {
                    Rotation::Rotate90 => Orientation::Rotate90,
                    Rotation::Rotate180 => Orientation::Rotate180,
                    Rotation::Rotate270 => Orientation::Rotate270,
                };
                current = current.apply_orientation(o);
            }
            Command::Flip(axis) => {
                let o = match axis {
                    FlipAxis::Horizontal => Orientation::FlipH,
                    FlipAxis::Vertical => Orientation::FlipV,
                };
                current = current.apply_orientation(o);
            }
            Command::Crop(sc) => {
                let r = sc.resolve(current.width, current.height);
                current = current.crop(r.x, r.y, r.width, r.height);
            }
            Command::Region(reg) => {
                current = current.apply_region(reg);
            }
            Command::Constrain { constraint } => {
                let layout = constraint.clone().compute(current.width, current.height).unwrap();
                // In immediate mode, the constraint's source is the current buffer.
                // source_crop applies to current, then resize, then place on canvas.
                let cropped = if let Some(sc) = &layout.source_crop {
                    current.crop(sc.x, sc.y, sc.width, sc.height)
                } else {
                    current
                };
                let resized =
                    cropped.resize_nn(layout.resize_to.width, layout.resize_to.height);
                current = resized.place_on_canvas(
                    layout.canvas.width,
                    layout.canvas.height,
                    layout.placement.0,
                    layout.placement.1,
                );
            }
            Command::Pad {
                top,
                right,
                bottom,
                left,
                ..
            } => {
                current = current.pad(*top, *right, *bottom, *left);
            }
        }
    }
    current
}

/// Fused-mode: compute a single Layout via compute_layout_sequential,
/// apply it in one pass to the original source.
fn fused_eval(source: &Grid, commands: &[Command]) -> Result<Grid, LayoutError> {
    let (ideal, _request) = compute_layout_sequential(
        commands,
        source.width,
        source.height,
        None,
    )?;

    // Apply orientation to source first (layout expects oriented source)
    let oriented = source.apply_orientation(ideal.orientation);
    let result = oriented.apply_layout(&ideal.layout);
    Ok(result)
}

fn compare(name: &str, source: &Grid, commands: &[Command]) {
    let immediate = immediate_eval(source, commands);
    let fused = fused_eval(source, commands);

    match fused {
        Ok(fused) => {
            let match_ok = immediate.width == fused.width
                && immediate.height == fused.height
                && immediate.pixels == fused.pixels;

            if !match_ok {
                eprintln!("=== MISMATCH: {name} ===");
                eprintln!("Immediate ({}x{}):", immediate.width, immediate.height);
                eprintln!("{}", immediate.summary());
                eprintln!("Fused ({}x{}):", fused.width, fused.height);
                eprintln!("{}", fused.summary());

                // Count mismatched pixels
                if immediate.width == fused.width && immediate.height == fused.height {
                    let mut mismatches = 0;
                    for i in 0..immediate.pixels.len() {
                        if immediate.pixels[i] != fused.pixels[i] {
                            mismatches += 1;
                        }
                    }
                    eprintln!(
                        "{mismatches}/{} pixels differ",
                        immediate.pixels.len()
                    );
                }
                panic!("{name}: immediate != fused");
            }
        }
        Err(e) => {
            panic!("{name}: fused eval failed with {e}, but immediate produced {}x{}", immediate.width, immediate.height);
        }
    }
}

fn compare_expect_mismatch(name: &str, source: &Grid, commands: &[Command]) -> Option<String> {
    let immediate = immediate_eval(source, commands);
    let fused = match fused_eval(source, commands) {
        Ok(f) => f,
        Err(e) => {
            return Some(format!("{name}: fused error '{e}', immediate produced {}x{}", immediate.width, immediate.height));
        }
    };

    let match_ok = immediate.width == fused.width
        && immediate.height == fused.height
        && immediate.pixels == fused.pixels;

    if match_ok {
        None
    } else {
        let mut msg = format!("{name}: size immediate={}x{} fused={}x{}",
            immediate.width, immediate.height, fused.width, fused.height);
        if immediate.width == fused.width && immediate.height == fused.height {
            let mismatches = immediate.pixels.iter().zip(&fused.pixels)
                .filter(|(a, b)| a != b).count();
            msg.push_str(&format!(", {mismatches}/{} pixels differ", immediate.pixels.len()));
        }
        Some(msg)
    }
}

// ---- Tests: cases that SHOULD match ----

#[test]
fn crop_only() {
    let src = Grid::source(8, 8);
    let commands = [Command::Crop(SourceCrop::Pixels(Rect::new(2, 2, 4, 4)))];
    compare("crop_only", &src, &commands);
}

#[test]
fn crop_crop() {
    let src = Grid::source(12, 12);
    let commands = [
        Command::Crop(SourceCrop::Pixels(Rect::new(2, 2, 8, 8))),
        Command::Crop(SourceCrop::Pixels(Rect::new(1, 1, 4, 4))),
    ];
    compare("crop→crop", &src, &commands);
}

#[test]
fn crop_constrain() {
    let src = Grid::source(100, 100);
    let commands = [
        Command::Crop(SourceCrop::Pixels(Rect::new(10, 10, 80, 80))),
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 40, 40),
        },
    ];
    compare("crop→constrain", &src, &commands);
}

#[test]
fn constrain_pad() {
    let src = Grid::source(100, 100);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 50, 50),
        },
        Command::Pad {
            top: 5,
            right: 5,
            bottom: 5,
            left: 5,
            color: CanvasColor::Transparent,
        },
    ];
    compare("constrain→pad", &src, &commands);
}

#[test]
fn orient_crop_constrain() {
    let src = Grid::source(12, 8);
    let commands = [
        Command::Rotate(Rotation::Rotate90),
        Command::Crop(SourceCrop::Pixels(Rect::new(1, 1, 6, 10))),
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 3, 5),
        },
    ];
    compare("orient→crop→constrain", &src, &commands);
}

#[test]
fn constrain_only() {
    let src = Grid::source(100, 50);
    let commands = [Command::Constrain {
        constraint: Constraint::new(ConstraintMode::Fit, 50, 50),
    }];
    compare("constrain_only", &src, &commands);
}

#[test]
fn region_pure_crop() {
    let src = Grid::source(10, 10);
    let commands = [Command::Region(Region::crop(2, 2, 8, 8))];
    compare("region_pure_crop", &src, &commands);
}

#[test]
fn region_pure_pad() {
    let src = Grid::source(8, 8);
    let commands = [Command::Region(Region::padded(2, CanvasColor::Transparent))];
    compare("region_pure_pad", &src, &commands);
}

#[test]
fn region_mixed_crop_pad() {
    // Crop right side, pad left side
    let src = Grid::source(10, 10);
    let commands = [Command::Region(Region {
        left: RegionCoord::px(-3),
        top: RegionCoord::px(0),
        right: RegionCoord::px(7),
        bottom: RegionCoord::pct(1.0),
        color: CanvasColor::Transparent,
    })];
    compare("region_mixed_crop_pad", &src, &commands);
}

// ---- Tests: cases that may NOT match (structural limitations) ----

#[test]
fn constrain_then_crop_origin() {
    // Post-constrain crop at origin — should work since placement stays non-negative
    let src = Grid::source(100, 100);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 50, 50),
        },
        Command::Crop(SourceCrop::Pixels(Rect::new(0, 0, 25, 25))),
    ];
    compare("constrain→crop(origin)", &src, &commands);
}

#[test]
fn constrain_then_crop_center() {
    // Post-constrain crop NOT at origin — placement goes negative, u32 saturates
    let src = Grid::source(100, 100);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 50, 50),
        },
        Command::Crop(SourceCrop::Pixels(Rect::new(10, 10, 30, 30))),
    ];
    let mismatch = compare_expect_mismatch("constrain→crop(center)", &src, &commands);
    if let Some(msg) = &mismatch {
        eprintln!("EXPECTED MISMATCH (u32 placement can't go negative): {msg}");
    }
    // Fixed: i32 placement allows negative offsets.
    assert!(mismatch.is_none(), "constrain→crop(center) should match now that placement is i32");
}

#[test]
fn constrain_then_crop_center_pixel_detail() {
    // Post-constrain crop with nonzero origin: i32 placement handles negative offsets
    let src = Grid::source(10, 10);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 10, 10), // identity resize
        },
        Command::Crop(SourceCrop::Pixels(Rect::new(3, 3, 4, 4))),
    ];
    compare("constrain(identity)→crop(3,3,4,4)", &src, &commands);
}

#[test]
fn pad_region_then_constrain() {
    // Pad then resize: constraint now targets viewport dimensions.
    // For this case (8x8 source, pad 4 → 16x16, Fit 8x8), the constraint
    // targets 16x16 with scale 0.5, producing 8x8 canvas with 4x4 content
    // at (2,2). This matches immediate mode: pad to 16x16, resize to 8x8.
    let src = Grid::source(8, 8);
    let commands = [
        Command::Region(Region::padded(4, CanvasColor::Transparent)),
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 8, 8),
        },
    ];
    compare("pad(4)→fit(8x8) on 8x8", &src, &commands);
}

#[test]
fn pad_region_then_constrain_downscale() {
    // Source 16x16, pad 4 → 24x24 viewport, Fit(8x8) → 8x8 output.
    // Dimensions match, but NN pixel coordinates differ because the
    // single-pass layout resizes content separately from padding.
    let src = Grid::source(16, 16);
    let commands = [
        Command::Region(Region::padded(4, CanvasColor::Transparent)),
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 8, 8),
        },
    ];

    let immediate = immediate_eval(&src, &commands);
    let fused = fused_eval(&src, &commands).unwrap();

    // Dimensions MUST match (constraint targets viewport → 8x8).
    assert_eq!(
        (immediate.width, immediate.height),
        (fused.width, fused.height),
        "pad+constrain dimensions should match"
    );
    // Pixel-level differences are expected: NN resampling a padded viewport
    // vs content-only produces different sampling grids at the boundary.
}

#[test]
fn crop_constrain_crop() {
    // Pre-constrain crop + post-constrain crop: i32 placement handles the offset
    let src = Grid::source(20, 20);
    let commands = [
        Command::Crop(SourceCrop::Pixels(Rect::new(2, 2, 16, 16))),
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 16, 16),
        },
        Command::Crop(SourceCrop::Pixels(Rect::new(4, 4, 8, 8))),
    ];
    compare("crop→constrain→crop(center)", &src, &commands);
}

#[test]
fn constrain_then_region_viewport() {
    // Post-constrain region: redefine canvas viewport of resized output
    let src = Grid::source(20, 20);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 10, 10),
        },
        Command::Region(Region {
            left: RegionCoord::px(2),
            top: RegionCoord::px(2),
            right: RegionCoord::px(8),
            bottom: RegionCoord::px(8),
            color: CanvasColor::Transparent,
        }),
    ];
    compare("constrain→region(2,2,8,8)", &src, &commands);
}

#[test]
fn constrain_then_pad_then_crop() {
    // Constrain → pad → crop the padded result: i32 placement handles the offset
    let src = Grid::source(20, 20);
    let commands = [
        Command::Constrain {
            constraint: Constraint::new(ConstraintMode::Fit, 10, 10),
        },
        Command::Pad {
            top: 5,
            right: 5,
            bottom: 5,
            left: 5,
            color: CanvasColor::Transparent,
        },
        Command::Crop(SourceCrop::Pixels(Rect::new(2, 2, 16, 16))),
    ];
    compare("constrain→pad→crop", &src, &commands);
}

// ---- Test: document ALL mismatches in one place ----

#[test]
fn audit_all_two_op_sequences() {
    // Systematically test all interesting 2-op sequences.
    //
    // 3 remaining mismatches are inherent NN sampling grid artifacts:
    //
    // 1. constrain→rotate: Fused mode rotates source THEN resizes (single pass).
    //    Immediate mode resizes THEN rotates. NN sampling from different grids
    //    produces different pixel assignments. Dimensions always match.
    //
    // 2. pad→constrain: Fused mode resizes content separately from padding
    //    (content is resize_to, padding is canvas-level). Immediate mode
    //    composes padding into the pixel buffer, then resizes the combined
    //    image. NN sampling at the content/padding boundary differs.
    //    Dimensions always match.
    //
    // 3. region_pad→constrain: Same mechanism as pad→constrain. The Region
    //    defines a padded viewport; fused mode decomposes it into content +
    //    canvas placement.
    //
    // All 3 produce correct dimensions. With real resamplers (bilinear,
    // lanczos) the pixel differences would be sub-pixel.
    let src = Grid::source(12, 12);

    let crop_a = Command::Crop(SourceCrop::Pixels(Rect::new(2, 2, 8, 8)));
    let crop_b = Command::Crop(SourceCrop::Pixels(Rect::new(1, 1, 4, 4)));
    let constrain = Command::Constrain {
        constraint: Constraint::new(ConstraintMode::Fit, 6, 6),
    };
    let pad = Command::Pad {
        top: 2,
        right: 2,
        bottom: 2,
        left: 2,
        color: CanvasColor::Transparent,
    };
    let region_pad = Command::Region(Region::padded(2, CanvasColor::Transparent));
    let region_crop = Command::Region(Region::crop(3, 3, 9, 9));
    let rotate = Command::Rotate(Rotation::Rotate90);

    let cases: Vec<(&str, Vec<Command>)> = vec![
        ("crop→crop", vec![crop_a.clone(), crop_b.clone()]),
        ("crop→constrain", vec![crop_a.clone(), constrain.clone()]),
        ("crop→pad", vec![crop_a.clone(), pad.clone()]),
        ("crop→rotate", vec![crop_a.clone(), rotate.clone()]),
        ("constrain→crop(origin)", vec![
            constrain.clone(),
            Command::Crop(SourceCrop::Pixels(Rect::new(0, 0, 3, 3))),
        ]),
        ("constrain→crop(center)", vec![constrain.clone(), crop_b.clone()]),
        ("constrain→pad", vec![constrain.clone(), pad.clone()]),
        ("constrain→rotate", vec![constrain.clone(), rotate.clone()]),
        ("pad→crop", vec![pad.clone(), crop_a.clone()]),
        ("pad→constrain", vec![pad.clone(), constrain.clone()]),
        ("rotate→crop", vec![rotate.clone(), crop_a.clone()]),
        ("rotate→constrain", vec![rotate.clone(), constrain.clone()]),
        ("rotate→pad", vec![rotate.clone(), pad.clone()]),
        ("region_pad→constrain", vec![region_pad.clone(), constrain.clone()]),
        ("region_crop→constrain", vec![region_crop.clone(), constrain.clone()]),
        ("constrain→region_crop_center", vec![
            constrain.clone(),
            Command::Region(Region::crop(1, 1, 5, 5)),
        ]),
    ];

    // Known NN sampling grid mismatches (dimensions correct, pixels differ)
    let expected_nn_mismatches = [
        "constrain→rotate",
        "pad→constrain",
        "region_pad→constrain",
    ];

    let mut matches = vec![];
    let mut nn_mismatches = vec![];
    let mut unexpected = vec![];

    for (name, cmds) in &cases {
        match compare_expect_mismatch(name, &src, cmds) {
            None => matches.push(*name),
            Some(msg) => {
                if expected_nn_mismatches.contains(name) {
                    nn_mismatches.push(msg);
                } else {
                    unexpected.push(msg);
                }
            }
        }
    }

    eprintln!("\n=== TWO-OP SEQUENCE AUDIT ===");
    eprintln!("\nMATCHES ({}):", matches.len());
    for m in &matches {
        eprintln!("  ✓ {m}");
    }
    eprintln!("\nNN SAMPLING MISMATCHES ({}, expected):", nn_mismatches.len());
    for m in &nn_mismatches {
        eprintln!("  ~ {m}");
    }

    assert!(unexpected.is_empty(),
        "Unexpected mismatches:\n{}",
        unexpected.iter().map(|m| format!("  ✗ {m}")).collect::<Vec<_>>().join("\n")
    );

    // Verify dimension correctness for NN mismatches
    for name in &expected_nn_mismatches {
        let cmds = cases.iter().find(|(n, _)| n == name).unwrap();
        let immediate = immediate_eval(&src, &cmds.1);
        let fused = fused_eval(&src, &cmds.1).unwrap();
        assert_eq!(
            (immediate.width, immediate.height),
            (fused.width, fused.height),
            "{name}: dimensions must match even if pixels differ"
        );
    }

    eprintln!();
}
