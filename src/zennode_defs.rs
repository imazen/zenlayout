//! zenode node definitions for layout and geometry operations.
//!
//! Defines crop, orientation, flip, rotation, expand canvas, and constraint
//! nodes with RIAPI-compatible querystring keys matching imageflow's
//! established layout parameters.

extern crate alloc;
use alloc::string::String;

use zennode::*;

// ─── Crop ───

/// Crop the image to a pixel rectangle.
///
/// Specifies origin (x, y) and dimensions (w, h) in post-orientation
/// source coordinates. Matches imageflow's `crop` querystring parameter
/// which accepts `x1,y1,x2,y2` (edge coordinates).
///
/// RIAPI: `?crop=10,10,90,90`
/// JSON: `{ "x": 10, "y": 10, "w": 80, "h": 80 }`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.crop", group = Geometry, role = Orient)]
#[node(coalesce = "layout_plan", changes_dimensions)]
#[node(tags("crop", "geometry"))]
pub struct Crop {
    /// Left edge X coordinate in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "X")]
    pub x: u32,

    /// Top edge Y coordinate in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Y")]
    pub y: u32,

    /// Width of the crop region in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Width")]
    pub w: u32,

    /// Height of the crop region in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Height")]
    pub h: u32,
}

// ─── Orient ───

/// Apply EXIF orientation correction.
///
/// Orientation values 1-8 follow the EXIF standard:
/// 1 = identity, 2 = flip-H, 3 = rotate-180, 4 = flip-V,
/// 5 = transpose, 6 = rotate-90, 7 = transverse, 8 = rotate-270.
///
/// RIAPI: `?autorotate=true` (uses embedded EXIF), `?srotate=90`
/// JSON: `{ "orientation": 6 }`
#[derive(Node, Clone, Debug)]
#[node(id = "zenlayout.orient", group = Geometry, role = Orient)]
#[node(coalesce = "layout_plan")]
#[node(tags("orient", "exif", "geometry"))]
pub struct Orient {
    /// EXIF orientation value (1-8). 1 = no transformation.
    #[param(range(1..=8), default = 1, step = 1)]
    #[param(section = "Main", label = "Orientation")]
    #[kv("autorotate", "srotate")]
    pub orientation: i32,
}

impl Default for Orient {
    fn default() -> Self {
        Self { orientation: 1 }
    }
}

// ─── FlipH ───

/// Flip the image horizontally (mirror left-right).
///
/// No parameters. Presence in the pipeline means flip is applied.
/// Composes with other orientation operations into a single transform.
///
/// RIAPI: `?sflip=h`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.flip_h", group = Geometry, role = Orient)]
#[node(coalesce = "layout_plan")]
#[node(tags("flip", "geometry"))]
pub struct FlipH {}

// ─── FlipV ───

/// Flip the image vertically (mirror top-bottom).
///
/// No parameters. Presence in the pipeline means flip is applied.
/// Composes with other orientation operations into a single transform.
///
/// RIAPI: `?sflip=v`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.flip_v", group = Geometry, role = Orient)]
#[node(coalesce = "layout_plan")]
#[node(tags("flip", "geometry"))]
pub struct FlipV {}

// ─── Rotate90 ───

/// Rotate the image 90 degrees clockwise.
///
/// Swaps width and height. NOT coalesced because 90/270 degree
/// rotations require pixel materialization (axis swap).
///
/// RIAPI: `?srotate=90`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.rotate_90", group = Geometry, role = Orient)]
#[node(changes_dimensions)]
#[node(tags("rotate", "geometry"))]
pub struct Rotate90 {}

// ─── Rotate180 ───

/// Rotate the image 180 degrees.
///
/// Decomposes to flip-H + flip-V (no axis swap), so it can be
/// coalesced into the layout plan without pixel materialization.
///
/// RIAPI: `?srotate=180`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.rotate_180", group = Geometry, role = Orient)]
#[node(coalesce = "layout_plan")]
#[node(tags("rotate", "geometry"))]
pub struct Rotate180 {}

// ─── Rotate270 ───

/// Rotate the image 270 degrees clockwise (90 counter-clockwise).
///
/// Swaps width and height. NOT coalesced because 90/270 degree
/// rotations require pixel materialization (axis swap).
///
/// RIAPI: `?srotate=270`
#[derive(Node, Clone, Debug, Default)]
#[node(id = "zenlayout.rotate_270", group = Geometry, role = Orient)]
#[node(changes_dimensions)]
#[node(tags("rotate", "geometry"))]
pub struct Rotate270 {}

// ─── ExpandCanvas ───

/// Expand the canvas by adding padding around the image.
///
/// Adds specified pixel amounts to each side. The fill color
/// defaults to "transparent" (premultiplied zero). Accepts CSS-style
/// named colors or hex values.
///
/// JSON: `{ "left": 10, "top": 10, "right": 10, "bottom": 10, "color": "white" }`
#[derive(Node, Clone, Debug)]
#[node(id = "zenlayout.expand_canvas", group = Canvas, role = Resize)]
#[node(coalesce = "layout_plan", changes_dimensions)]
#[node(tags("pad", "canvas", "geometry"))]
pub struct ExpandCanvas {
    /// Left padding in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Left")]
    pub left: u32,

    /// Top padding in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Top")]
    pub top: u32,

    /// Right padding in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Right")]
    pub right: u32,

    /// Bottom padding in pixels.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Bottom")]
    pub bottom: u32,

    /// Fill color for the expanded area.
    ///
    /// Accepts "transparent", "white", "black", or hex "#RRGGBB" / "#RRGGBBAA".
    #[param(default = "transparent")]
    #[param(section = "Main", label = "Color")]
    pub color: String,
}

impl Default for ExpandCanvas {
    fn default() -> Self {
        Self {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            color: String::from("transparent"),
        }
    }
}

// ─── Constrain ───

/// Constrain image dimensions with a fit mode.
///
/// Combines target width/height with a constraint mode to compute
/// the output layout. Supports all imageflow/RIAPI fit modes.
///
/// RIAPI: `?w=800&h=600&mode=crop`
/// JSON: `{ "w": 800, "h": 600, "mode": "fit" }`
#[derive(Node, Clone, Debug)]
#[node(id = "zenlayout.constrain", group = Layout, role = Resize)]
#[node(coalesce = "layout_plan", changes_dimensions)]
#[node(tags("resize", "constrain", "layout", "geometry"))]
pub struct Constrain {
    /// Target width in pixels. 0 = unconstrained.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Width")]
    #[kv("w", "width")]
    pub w: u32,

    /// Target height in pixels. 0 = unconstrained.
    #[param(range(0..=u32::MAX), default = 0, step = 1)]
    #[param(unit = "px", section = "Main", label = "Height")]
    #[kv("h", "height")]
    pub h: u32,

    /// Constraint mode: how to fit the image into the target box.
    ///
    /// "fit" = scale to fit, "within" = fit without upscale,
    /// "fit_crop" = fill and crop, "within_crop" = fill without upscale,
    /// "fit_pad" = fit and pad, "within_pad" = fit without upscale + pad,
    /// "distort" = stretch to exact, "aspect_crop" = crop to aspect only.
    #[param(default = "fit")]
    #[param(section = "Main", label = "Mode")]
    #[kv("mode")]
    pub mode: String,
}

impl Default for Constrain {
    fn default() -> Self {
        Self {
            w: 0,
            h: 0,
            mode: String::from("fit"),
        }
    }
}

// ─── Registration ───

/// Register all zenlayout nodes with a registry.
pub fn register(registry: &mut NodeRegistry) {
    for node in ALL {
        registry.register(*node);
    }
}

/// All zenlayout zenode definitions.
pub static ALL: &[&dyn NodeDef] = &[
    &CROP_NODE,
    &ORIENT_NODE,
    &FLIP_H_NODE,
    &FLIP_V_NODE,
    &ROTATE90_NODE,
    &ROTATE180_NODE,
    &ROTATE270_NODE,
    &EXPAND_CANVAS_NODE,
    &CONSTRAIN_NODE,
];

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Crop tests ───

    #[test]
    fn crop_schema() {
        let schema = CROP_NODE.schema();
        assert_eq!(schema.id, "zenlayout.crop");
        assert_eq!(schema.group, NodeGroup::Geometry);
        assert_eq!(schema.role, NodeRole::Orient);
        assert!(schema.tags.contains(&"crop"));
        assert!(schema.tags.contains(&"geometry"));
        assert!(schema.coalesce.is_some());
        assert_eq!(schema.coalesce.as_ref().unwrap().group, "layout_plan");
        assert!(schema.format.changes_dimensions);
    }

    #[test]
    fn crop_defaults() {
        let node = CROP_NODE.create_default().unwrap();
        assert_eq!(node.get_param("x"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("y"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(0)));
    }

    #[test]
    fn crop_create_with_params() {
        let mut params = ParamMap::new();
        params.insert("x".into(), ParamValue::U32(10));
        params.insert("y".into(), ParamValue::U32(20));
        params.insert("w".into(), ParamValue::U32(100));
        params.insert("h".into(), ParamValue::U32(80));

        let node = CROP_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("x"), Some(ParamValue::U32(10)));
        assert_eq!(node.get_param("y"), Some(ParamValue::U32(20)));
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(100)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(80)));
    }

    #[test]
    fn crop_downcast() {
        let node = CROP_NODE.create_default().unwrap();
        let crop = node.as_any().downcast_ref::<Crop>().unwrap();
        assert_eq!(crop.x, 0);
        assert_eq!(crop.w, 0);
    }

    #[test]
    fn crop_round_trip() {
        let crop = Crop {
            x: 50,
            y: 60,
            w: 200,
            h: 150,
        };
        let params = crop.to_params();
        let node = CROP_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("x"), Some(ParamValue::U32(50)));
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(200)));
    }

    // ─── Orient tests ───

    #[test]
    fn orient_schema() {
        let schema = ORIENT_NODE.schema();
        assert_eq!(schema.id, "zenlayout.orient");
        assert_eq!(schema.group, NodeGroup::Geometry);
        assert_eq!(schema.role, NodeRole::Orient);
        assert!(schema.tags.contains(&"orient"));
        assert!(schema.tags.contains(&"exif"));
        assert!(schema.coalesce.is_some());
    }

    #[test]
    fn orient_defaults() {
        let node = ORIENT_NODE.create_default().unwrap();
        assert_eq!(node.get_param("orientation"), Some(ParamValue::I32(1)));
    }

    #[test]
    fn orient_from_kv() {
        let mut kv = KvPairs::from_querystring("srotate=6");
        let node = ORIENT_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("orientation"), Some(ParamValue::I32(6)));
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn orient_downcast() {
        let mut params = ParamMap::new();
        params.insert("orientation".into(), ParamValue::I32(3));
        let node = ORIENT_NODE.create(&params).unwrap();
        let orient = node.as_any().downcast_ref::<Orient>().unwrap();
        assert_eq!(orient.orientation, 3);
    }

    // ─── Flip tests ───

    #[test]
    fn flip_h_schema() {
        let schema = FLIP_H_NODE.schema();
        assert_eq!(schema.id, "zenlayout.flip_h");
        assert_eq!(schema.group, NodeGroup::Geometry);
        assert!(schema.tags.contains(&"flip"));
        assert!(schema.coalesce.is_some());
        assert_eq!(schema.params.len(), 0);
    }

    #[test]
    fn flip_v_schema() {
        let schema = FLIP_V_NODE.schema();
        assert_eq!(schema.id, "zenlayout.flip_v");
        assert!(schema.tags.contains(&"flip"));
        assert!(schema.coalesce.is_some());
    }

    // ─── Rotation tests ───

    #[test]
    fn rotate_90_not_coalesced() {
        let schema = ROTATE90_NODE.schema();
        assert_eq!(schema.id, "zenlayout.rotate_90");
        assert!(schema.tags.contains(&"rotate"));
        // 90/270 require materialization — no coalesce
        assert!(schema.coalesce.is_none());
        assert!(schema.format.changes_dimensions);
    }

    #[test]
    fn rotate_180_coalesced() {
        let schema = ROTATE180_NODE.schema();
        assert_eq!(schema.id, "zenlayout.rotate_180");
        // 180 decomposes to flip_h + flip_v — can be coalesced
        assert!(schema.coalesce.is_some());
        assert_eq!(schema.coalesce.as_ref().unwrap().group, "layout_plan");
    }

    #[test]
    fn rotate_270_not_coalesced() {
        let schema = ROTATE270_NODE.schema();
        assert_eq!(schema.id, "zenlayout.rotate_270");
        assert!(schema.coalesce.is_none());
        assert!(schema.format.changes_dimensions);
    }

    // ─── ExpandCanvas tests ───

    #[test]
    fn expand_canvas_schema() {
        let schema = EXPAND_CANVAS_NODE.schema();
        assert_eq!(schema.id, "zenlayout.expand_canvas");
        assert_eq!(schema.group, NodeGroup::Canvas);
        assert_eq!(schema.role, NodeRole::Resize);
        assert!(schema.tags.contains(&"pad"));
        assert!(schema.tags.contains(&"canvas"));
        assert!(schema.coalesce.is_some());
        assert!(schema.format.changes_dimensions);
    }

    #[test]
    fn expand_canvas_defaults() {
        let node = EXPAND_CANVAS_NODE.create_default().unwrap();
        assert_eq!(node.get_param("left"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("top"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("right"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("bottom"), Some(ParamValue::U32(0)));
        assert_eq!(
            node.get_param("color"),
            Some(ParamValue::Str("transparent".into()))
        );
    }

    #[test]
    fn expand_canvas_create_with_params() {
        let mut params = ParamMap::new();
        params.insert("left".into(), ParamValue::U32(10));
        params.insert("top".into(), ParamValue::U32(20));
        params.insert("right".into(), ParamValue::U32(10));
        params.insert("bottom".into(), ParamValue::U32(20));
        params.insert("color".into(), ParamValue::Str("white".into()));

        let node = EXPAND_CANVAS_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("left"), Some(ParamValue::U32(10)));
        assert_eq!(
            node.get_param("color"),
            Some(ParamValue::Str("white".into()))
        );
    }

    #[test]
    fn expand_canvas_downcast() {
        let node = EXPAND_CANVAS_NODE.create_default().unwrap();
        let ec = node.as_any().downcast_ref::<ExpandCanvas>().unwrap();
        assert_eq!(ec.left, 0);
        assert_eq!(ec.color, "transparent");
    }

    // ─── Constrain tests ───

    #[test]
    fn constrain_schema() {
        let schema = CONSTRAIN_NODE.schema();
        assert_eq!(schema.id, "zenlayout.constrain");
        assert_eq!(schema.group, NodeGroup::Layout);
        assert_eq!(schema.role, NodeRole::Resize);
        assert!(schema.tags.contains(&"resize"));
        assert!(schema.tags.contains(&"constrain"));
        assert!(schema.coalesce.is_some());
        assert!(schema.format.changes_dimensions);
    }

    #[test]
    fn constrain_defaults() {
        let node = CONSTRAIN_NODE.create_default().unwrap();
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(0)));
        assert_eq!(node.get_param("mode"), Some(ParamValue::Str("fit".into())));
    }

    #[test]
    fn constrain_from_kv() {
        let mut kv = KvPairs::from_querystring("w=800&h=600&mode=crop");
        let node = CONSTRAIN_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(800)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(600)));
        assert_eq!(node.get_param("mode"), Some(ParamValue::Str("crop".into())));
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn constrain_from_kv_width_only() {
        let mut kv = KvPairs::from_querystring("w=400");
        let node = CONSTRAIN_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(400)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(0)));
    }

    #[test]
    fn constrain_from_kv_no_match() {
        let mut kv = KvPairs::from_querystring("quality=85&jpeg.progressive=true");
        let result = CONSTRAIN_NODE.from_kv(&mut kv).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn constrain_downcast() {
        let node = CONSTRAIN_NODE.create_default().unwrap();
        let c = node.as_any().downcast_ref::<Constrain>().unwrap();
        assert_eq!(c.w, 0);
        assert_eq!(c.h, 0);
        assert_eq!(c.mode, "fit");
    }

    #[test]
    fn constrain_round_trip() {
        let c = Constrain {
            w: 1920,
            h: 1080,
            mode: String::from("fit_crop"),
        };
        let params = c.to_params();
        let node = CONSTRAIN_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("w"), Some(ParamValue::U32(1920)));
        assert_eq!(node.get_param("h"), Some(ParamValue::U32(1080)));
        assert_eq!(
            node.get_param("mode"),
            Some(ParamValue::Str("fit_crop".into()))
        );
    }

    // ─── Registry tests ───

    #[test]
    fn registry_all_nodes() {
        let mut registry = NodeRegistry::new();
        register(&mut registry);
        assert_eq!(registry.all().len(), 9);
        assert!(registry.get("zenlayout.crop").is_some());
        assert!(registry.get("zenlayout.orient").is_some());
        assert!(registry.get("zenlayout.flip_h").is_some());
        assert!(registry.get("zenlayout.flip_v").is_some());
        assert!(registry.get("zenlayout.rotate_90").is_some());
        assert!(registry.get("zenlayout.rotate_180").is_some());
        assert!(registry.get("zenlayout.rotate_270").is_some());
        assert!(registry.get("zenlayout.expand_canvas").is_some());
        assert!(registry.get("zenlayout.constrain").is_some());
    }

    #[test]
    fn registry_querystring() {
        let mut registry = NodeRegistry::new();
        register(&mut registry);

        let result = registry.from_querystring("w=800&h=600&mode=crop");
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.instances[0].schema().id, "zenlayout.constrain");
        assert_eq!(
            result.instances[0].get_param("w"),
            Some(ParamValue::U32(800))
        );
    }
}
