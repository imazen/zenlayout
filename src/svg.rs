//! SVG visualization of layout pipeline steps.
//!
//! Generates a vertical sequence of annotated panels showing each transformation
//! in the layout pipeline: source → crop → orient → resize → canvas → edge extend.
//!
//! # Example
//!
//! ```
//! use zenlayout::{Pipeline, DecoderOffer, svg::render_layout_svg};
//!
//! let (ideal, req) = Pipeline::new(4000, 3000)
//!     .auto_orient(6)
//!     .crop_pixels(200, 200, 3000, 2000)
//!     .fit_pad(800, 800)
//!     .plan()
//!     .unwrap();
//!
//! let offer = DecoderOffer::full_decode(4000, 3000);
//! let plan = ideal.finalize(&req, &offer);
//!
//! let svg = render_layout_svg(&ideal, &plan);
//! // svg is a complete SVG document string
//! ```

#[cfg(not(feature = "std"))]
use alloc::format;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::constraint::Size;
use crate::plan::{IdealLayout, LayoutPlan};

/// Maximum pixel width for any panel in the SVG output.
const MAX_PANEL_W: f64 = 300.0;
/// Maximum pixel height for any panel in the SVG output.
const MAX_PANEL_H: f64 = 200.0;
/// Vertical gap between panels.
const PANEL_GAP: f64 = 50.0;
/// Horizontal margin.
const MARGIN_X: f64 = 50.0;
/// Top margin for first panel.
const MARGIN_TOP: f64 = 30.0;
/// Height of label text area above each panel.
const LABEL_H: f64 = 22.0;
/// Arrow height between panels.
const ARROW_H: f64 = 28.0;

/// A single step in the pipeline visualization.
struct Step {
    label: String,
    /// The overall bounding box (e.g., the canvas).
    outer: Size,
    /// The inner content rect within the outer box (for showing crop/placement).
    /// None means the content fills the entire outer box.
    inner: Option<InnerRect>,
    /// Optional annotation text below the label.
    annotation: String,
    /// If true, show a hatch pattern for the extension area.
    show_extension: Option<Size>,
}

/// A positioned rectangle within a step's outer box.
struct InnerRect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// Render a complete SVG document showing the layout pipeline step by step.
///
/// Takes the [`IdealLayout`] (phase 1 result) and [`LayoutPlan`] (phase 2 result)
/// and produces a vertical sequence of annotated panels.
///
/// Returns a complete SVG document as a string.
pub fn render_layout_svg(ideal: &IdealLayout, plan: &LayoutPlan) -> String {
    let steps = build_steps(ideal, plan);
    render_steps(&steps)
}

/// Build the sequence of pipeline steps from layout results.
fn build_steps(ideal: &IdealLayout, plan: &LayoutPlan) -> Vec<Step> {
    let mut steps = Vec::new();
    let layout = &ideal.layout;

    // Step 1: Source
    steps.push(Step {
        label: format!("Source  {}×{}", layout.source.width, layout.source.height),
        outer: layout.source,
        inner: None,
        annotation: if !ideal.orientation.is_identity() {
            format!("EXIF {} ({:?})", ideal.orientation.to_exif(), ideal.orientation)
        } else {
            String::new()
        },
        show_extension: None,
    });

    // Step 2: Crop (if source_crop present in the layout)
    if let Some(crop) = &layout.source_crop {
        steps.push(Step {
            label: format!("Crop  {}×{}", crop.width, crop.height),
            outer: layout.source,
            inner: Some(InnerRect {
                x: crop.x as i32,
                y: crop.y as i32,
                w: crop.width,
                h: crop.height,
            }),
            annotation: format!("at ({}, {})", crop.x, crop.y),
            show_extension: None,
        });
    }

    // Step 3: Trim (only show when no Crop step — otherwise Trim is an
    // implementation detail of decoder negotiation that repeats the crop info)
    if layout.source_crop.is_none() {
        if let Some(trim) = &plan.trim {
            let decoder_dims = Size::new(trim.x + trim.width, trim.y + trim.height);
            steps.push(Step {
                label: format!("Trim  {}×{}", trim.width, trim.height),
                outer: decoder_dims,
                inner: Some(InnerRect {
                    x: trim.x as i32,
                    y: trim.y as i32,
                    w: trim.width,
                    h: trim.height,
                }),
                annotation: if trim.x == 0 && trim.y == 0 {
                    format!("from {}×{} decode", decoder_dims.width, decoder_dims.height)
                } else {
                    format!("offset ({},{}) in decode", trim.x, trim.y)
                },
                show_extension: None,
            });
        }
    }

    // Step 4: Orient (if not identity)
    if !plan.remaining_orientation.is_identity() {
        let effective = layout.effective_source();
        let oriented = plan.remaining_orientation.transform_dimensions(effective.width, effective.height);
        steps.push(Step {
            label: format!("Orient  {}×{}", oriented.width, oriented.height),
            outer: oriented,
            inner: None,
            annotation: format!("{:?}", plan.remaining_orientation),
            show_extension: None,
        });
    }

    // Step 5: Resize (if not identity)
    if !plan.resize_is_identity {
        steps.push(Step {
            label: format!("Resize  {}×{}", plan.resize_to.width, plan.resize_to.height),
            outer: plan.resize_to,
            inner: None,
            annotation: String::new(),
            show_extension: None,
        });
    }

    // Step 6: Canvas + Placement (if canvas differs from resize_to or placement is non-zero)
    let (px, py) = plan.placement;
    if plan.canvas != plan.resize_to || px != 0 || py != 0 {
        steps.push(Step {
            label: format!("Canvas  {}×{}", plan.canvas.width, plan.canvas.height),
            outer: plan.canvas,
            inner: Some(InnerRect {
                x: px,
                y: py,
                w: plan.resize_to.width,
                h: plan.resize_to.height,
            }),
            annotation: format!(
                "place at ({}, {}), bg {:?}",
                px, py, plan.canvas_color
            ),
            show_extension: None,
        });
    }

    // Step 7: Edge extension (if content_size set)
    if let Some(content) = plan.content_size {
        steps.push(Step {
            label: format!("Extend  {}×{}", plan.canvas.width, plan.canvas.height),
            outer: plan.canvas,
            inner: Some(InnerRect {
                x: 0,
                y: 0,
                w: content.width,
                h: content.height,
            }),
            annotation: format!("content {}×{}, edges replicated", content.width, content.height),
            show_extension: Some(content),
        });
    }

    // Final output step (always shown)
    let final_size = plan.canvas;
    let last = steps.last().map(|s| &s.label);
    let already_final = last.is_some_and(|l| {
        l.starts_with("Canvas") || l.starts_with("Extend") || l.starts_with("Resize")
    });
    if !already_final || steps.len() == 1 {
        steps.push(Step {
            label: format!("Output  {}×{}", final_size.width, final_size.height),
            outer: final_size,
            inner: None,
            annotation: String::new(),
            show_extension: None,
        });
    }

    steps
}

/// Scale a Size to fit within MAX_PANEL_W × MAX_PANEL_H, preserving aspect ratio.
fn scale_to_fit(size: Size) -> (f64, f64, f64) {
    let w = size.width as f64;
    let h = size.height as f64;
    if w == 0.0 || h == 0.0 {
        return (1.0, 1.0, 1.0);
    }
    let scale = (MAX_PANEL_W / w).min(MAX_PANEL_H / h);
    (w * scale, h * scale, scale)
}

/// Render step panels into a complete SVG document.
fn render_steps(steps: &[Step]) -> String {
    if steps.is_empty() {
        return String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#);
    }

    // Calculate total height
    let mut total_h = MARGIN_TOP;
    for (i, _step) in steps.iter().enumerate() {
        total_h += LABEL_H;
        total_h += MAX_PANEL_H;
        if i < steps.len() - 1 {
            total_h += ARROW_H + PANEL_GAP - ARROW_H; // gap includes arrow space
        }
    }
    total_h += MARGIN_TOP; // bottom margin

    let total_w = MAX_PANEL_W + 2.0 * MARGIN_X;

    let mut svg = String::with_capacity(4096);

    // SVG header
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        total_w as u32,
        total_h as u32,
        total_w,
        total_h
    ));
    svg.push('\n');

    // Style — light/dark mode via prefers-color-scheme
    svg.push_str(r##"<style>
  text { font-family: "Consolas", "DejaVu Sans Mono", "Courier New", monospace; }
  .label { font-size: 13px; font-weight: bold; fill: #333; }
  .annotation { font-size: 11px; fill: #666; }
  .outer { fill: #e8e8e8; stroke: #999; stroke-width: 1; }
  .inner { fill: #6ba3d6; stroke: #2c6faa; stroke-width: 1.5; }
  .content-fill { fill: #6ba3d6; }
  .extend-fill { fill: #b8d4ee; stroke: #7baed0; stroke-width: 1; stroke-dasharray: 4,2; }
  .arrow { stroke: #666; stroke-width: 1.5; fill: none; marker-end: url(#arrowhead); }
  .arrowhead { fill: #666; }
  @media (prefers-color-scheme: dark) {
    .label { fill: #e0e0e0; }
    .annotation { fill: #aaa; }
    .outer { fill: #2d2d2d; stroke: #555; }
    .inner { fill: #3a72a4; stroke: #5a9fd4; }
    .content-fill { fill: #3a72a4; }
    .extend-fill { fill: #2a4a65; stroke: #4a7a9e; }
    .arrow { stroke: #888; }
    .arrowhead { fill: #888; }
  }
</style>
"##);

    // Arrow marker definition
    svg.push_str(r##"<defs>
  <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
    <polygon points="0 0, 8 3, 0 6" class="arrowhead"/>
  </marker>
</defs>
"##);

    let mut y = MARGIN_TOP;
    let center_x = total_w / 2.0;

    for (i, step) in steps.iter().enumerate() {
        // Label
        svg.push_str(&format!(
            r#"<text x="{}" y="{}" class="label" text-anchor="middle">{}</text>"#,
            center_x,
            y + 14.0,
            escape_xml(&step.label)
        ));
        svg.push('\n');
        y += LABEL_H;

        // Panel
        let (sw, sh, scale) = scale_to_fit(step.outer);
        let panel_x = center_x - sw / 2.0;
        let panel_y = y;

        // Outer box (canvas / source background)
        svg.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="outer" rx="2"/>"#,
            panel_x, panel_y, sw, sh
        ));
        svg.push('\n');

        // Inner rect (crop region / placed content)
        if let Some(inner) = &step.inner {
            let ix = panel_x + inner.x as f64 * scale;
            let iy = panel_y + inner.y as f64 * scale;
            let iw = inner.w as f64 * scale;
            let ih = inner.h as f64 * scale;

            if let Some(content) = step.show_extension {
                // Extension panel: show content area and extend area differently
                let cw = content.width as f64 * scale;
                let ch = content.height as f64 * scale;

                // Content area
                svg.push_str(&format!(
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="content-fill"/>"#,
                    panel_x, panel_y, cw, ch
                ));
                svg.push('\n');

                // Right extension
                if cw < sw {
                    svg.push_str(&format!(
                        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="extend-fill"/>"#,
                        panel_x + cw, panel_y, sw - cw, ch
                    ));
                    svg.push('\n');
                }

                // Bottom extension (full width)
                if ch < sh {
                    svg.push_str(&format!(
                        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="extend-fill"/>"#,
                        panel_x, panel_y + ch, sw, sh - ch
                    ));
                    svg.push('\n');
                }
            } else {
                // Normal inner rect (crop highlight or placed content)
                svg.push_str(&format!(
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="inner" rx="1"/>"#,
                    ix, iy, iw, ih
                ));
                svg.push('\n');
            }
        } else if step.show_extension.is_none() {
            // No inner rect — fill the whole panel as content
            svg.push_str(&format!(
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" class="inner" rx="2"/>"#,
                panel_x, panel_y, sw, sh
            ));
            svg.push('\n');
        }

        // Annotation
        if !step.annotation.is_empty() {
            svg.push_str(&format!(
                r#"<text x="{}" y="{:.1}" class="annotation" text-anchor="middle">{}</text>"#,
                center_x,
                panel_y + sh + 14.0,
                escape_xml(&step.annotation)
            ));
            svg.push('\n');
        }

        y += MAX_PANEL_H;

        // Arrow to next step
        if i < steps.len() - 1 {
            let arrow_top = y + 8.0;
            let arrow_bot = y + PANEL_GAP - 8.0;
            svg.push_str(&format!(
                r#"<line x1="{}" y1="{:.1}" x2="{}" y2="{:.1}" class="arrow"/>"#,
                center_x, arrow_top, center_x, arrow_bot
            ));
            svg.push('\n');
            y += PANEL_GAP;
        }
    }

    svg.push_str("</svg>\n");
    svg
}

/// Escape special characters for XML text content.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DecoderOffer, Pipeline};

    #[test]
    fn svg_identity_passthrough() {
        let (ideal, req) = Pipeline::new(100, 100).plan().unwrap();
        let offer = DecoderOffer::full_decode(100, 100);
        let plan = ideal.finalize(&req, &offer);

        let svg = render_layout_svg(&ideal, &plan);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("100×100"));
        assert!(svg.ends_with("</svg>\n"));
    }

    #[test]
    fn svg_crop_and_resize() {
        let (ideal, req) = Pipeline::new(1000, 800)
            .crop_pixels(100, 100, 600, 400)
            .fit(300, 200)
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(1000, 800);
        let plan = ideal.finalize(&req, &offer);

        let svg = render_layout_svg(&ideal, &plan);
        assert!(svg.contains("Source"));
        assert!(svg.contains("1000×800"));
        assert!(svg.contains("Crop"));
        assert!(svg.contains("600×400"));
        assert!(svg.contains("Resize"));
        assert!(svg.contains("300×200"));
    }

    #[test]
    fn svg_fit_pad_shows_canvas() {
        let (ideal, req) = Pipeline::new(200, 100)
            .fit_pad(100, 100)
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(200, 100);
        let plan = ideal.finalize(&req, &offer);

        assert_eq!(plan.canvas, Size::new(100, 100));
        assert_eq!(plan.resize_to, Size::new(100, 50));

        let svg = render_layout_svg(&ideal, &plan);
        assert!(svg.contains("Canvas"));
        assert!(svg.contains("100×100"));
    }

    #[test]
    fn svg_orientation_shows_orient_step() {
        let (ideal, req) = Pipeline::new(600, 400)
            .auto_orient(6) // Rotate90 → 400×600
            .fit(200, 300)
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(600, 400);
        let plan = ideal.finalize(&req, &offer);

        let svg = render_layout_svg(&ideal, &plan);
        assert!(svg.contains("Orient"));
        assert!(svg.contains("Rotate90"));
    }

    #[test]
    fn svg_output_matches_plan_canvas() {
        // Test that the final step dimensions match plan.canvas
        let (ideal, req) = Pipeline::new(4000, 3000)
            .auto_orient(6)
            .crop_pixels(200, 200, 3000, 2000)
            .fit_pad(800, 800)
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(4000, 3000);
        let plan = ideal.finalize(&req, &offer);

        let svg = render_layout_svg(&ideal, &plan);

        // The SVG should contain the final canvas dimensions
        let final_dim = format!("{}×{}", plan.canvas.width, plan.canvas.height);
        assert!(
            svg.contains(&final_dim),
            "SVG must show final output dimensions {final_dim}"
        );
    }

    #[test]
    fn svg_extend_shows_content_area() {
        use crate::plan::{Align, OutputLimits};

        let (ideal, req) = Pipeline::new(801, 601)
            .output_limits(OutputLimits {
                max: None,
                min: None,
                align: Some(Align::uniform_extend(16)),
            })
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(801, 601);
        let plan = ideal.finalize(&req, &offer);

        assert!(plan.content_size.is_some());
        let svg = render_layout_svg(&ideal, &plan);
        assert!(svg.contains("Extend"));
        assert!(svg.contains("content"));
    }

    #[test]
    #[ignore] // run with: cargo test --features svg -- --ignored generate_sample_svgs --nocapture
    fn generate_sample_svgs() {
        use crate::plan::{Align, OutputLimits};
        let out = "/mnt/v/output/zenlayout/svg";
        let doc = concat!(env!("CARGO_MANIFEST_DIR"), "/doc/svg");
        std::fs::create_dir_all(out).unwrap();
        std::fs::create_dir_all(doc).unwrap();

        let cases: Vec<(&str, String)> = vec![
            // 1. Simple resize (most common operation)
            {
                let (ideal, req) = Pipeline::new(4000, 3000).fit(800, 600).plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(4000, 3000));
                ("fit", render_layout_svg(&ideal, &plan))
            },
            // 2. FitCrop — crop to different aspect ratio
            {
                let (ideal, req) = Pipeline::new(1920, 1080).fit_crop(500, 500).plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(1920, 1080));
                ("fit_crop", render_layout_svg(&ideal, &plan))
            },
            // 3. FitPad — letterbox into a square
            {
                let (ideal, req) = Pipeline::new(1600, 900).fit_pad(400, 400).plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(1600, 900));
                ("fit_pad", render_layout_svg(&ideal, &plan))
            },
            // 4. Explicit crop + resize
            {
                let (ideal, req) = Pipeline::new(1000, 800)
                    .crop_pixels(100, 50, 600, 500)
                    .fit(300, 250)
                    .plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(1000, 800));
                ("crop_resize", render_layout_svg(&ideal, &plan))
            },
            // 5. EXIF orientation + resize
            {
                let (ideal, req) = Pipeline::new(4000, 3000)
                    .auto_orient(6)
                    .fit(600, 800)
                    .plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(4000, 3000));
                ("orient_resize", render_layout_svg(&ideal, &plan))
            },
            // 6. Full pipeline — orient + crop + fit_pad
            {
                let (ideal, req) = Pipeline::new(4000, 3000)
                    .auto_orient(6)
                    .crop_pixels(200, 200, 2600, 2600)
                    .fit_pad(800, 800)
                    .plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(4000, 3000));
                ("orient_crop_pad", render_layout_svg(&ideal, &plan))
            },
            // 7. MCU edge extension
            {
                let (ideal, req) = Pipeline::new(801, 601)
                    .output_limits(OutputLimits {
                        max: None, min: None,
                        align: Some(Align::uniform_extend(16)),
                    })
                    .plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(801, 601));
                ("mcu_extend", render_layout_svg(&ideal, &plan))
            },
            // 8. WithinCrop — downscale only, crop to target ratio
            {
                let (ideal, req) = Pipeline::new(800, 600)
                    .within_crop(400, 400)
                    .plan().unwrap();
                let plan = ideal.finalize(&req, &DecoderOffer::full_decode(800, 600));
                ("within_crop", render_layout_svg(&ideal, &plan))
            },
        ];

        for (name, svg) in &cases {
            std::fs::write(format!("{out}/{name}.svg"), svg).unwrap();
            std::fs::write(format!("{doc}/{name}.svg"), svg).unwrap();
        }

        println!("Generated {} SVGs in {out} and {doc}", cases.len());
    }

    #[test]
    fn svg_is_valid_xml() {
        let (ideal, req) = Pipeline::new(1920, 1080)
            .auto_orient(3) // Rotate180
            .crop_percent(0.1, 0.1, 0.8, 0.8)
            .within_crop(800, 600)
            .plan()
            .unwrap();

        let offer = DecoderOffer::full_decode(1920, 1080);
        let plan = ideal.finalize(&req, &offer);

        let svg = render_layout_svg(&ideal, &plan);

        // Basic XML validity checks
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("</svg>"));
        // No unescaped angle brackets in text
        assert!(!svg.contains("<<"));
    }
}
