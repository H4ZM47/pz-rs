//! `zombie.worldMap.styles`: `WorldMapStyle` (the ordered style layers a
//! map draws) and the layer types with their zoom stops.

use crate::data::WorldMapFeature;
use crate::renderer::{clamp, lerp};
use pz_render::Texture;
use pz_text::UIFont;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `WorldMapStyleLayer.RGBAf`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RGBAf {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl RGBAf {
    pub const WHITE: RGBAf = RGBAf {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
}

/// `ColorStop`: a colour (0..255 channels) at a zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub zoom: f32,
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
}

/// `FloatStop`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatStop {
    pub zoom: f32,
    pub f: f32,
}

/// `TextureScaling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureScaling {
    IsoGridSquare,
    ScreenPixel,
}

impl TextureScaling {
    /// `TextureScaling.valueOf(name)`.
    pub fn value_of(s: &str) -> Option<Self> {
        match s {
            "IsoGridSquare" => Some(Self::IsoGridSquare),
            "ScreenPixel" => Some(Self::ScreenPixel),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::IsoGridSquare => "IsoGridSquare",
            Self::ScreenPixel => "ScreenPixel",
        }
    }
}

/// `TextureStop`: `Texture.getTexture(path)` at creation.
#[derive(Clone, Debug)]
pub struct TextureStop {
    pub zoom: f32,
    pub texture_path: Option<Rc<str>>,
    pub texture: Option<Texture>,
    pub scaling: TextureScaling,
}

impl TextureStop {
    pub fn new(zoom: f32, path: Option<&str>, scaling: TextureScaling) -> Self {
        TextureStop {
            zoom,
            texture_path: path.map(Rc::from),
            texture: path.and_then(pz_render::get_shared_texture),
            scaling,
        }
    }
}

/// The feature filter `setFilter(key, value)` installs.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// Line layers: a line string whose `key` equals `value`.
    LineEquals(Rc<str>, Rc<str>),
    /// Polygon layers: a polygon whose `key` equals `value`.
    PolygonEquals(Rc<str>, Rc<str>),
    /// Polygon layers with value `*`: a polygon that has `key`.
    PolygonHas(Rc<str>),
}

impl Filter {
    pub fn matches(&self, f: &WorldMapFeature) -> bool {
        match self {
            Filter::LineEquals(k, v) => f.has_line_string() && f.properties.get(k) == Some(&**v),
            Filter::PolygonEquals(k, v) => f.has_polygon() && f.properties.get(k) == Some(&**v),
            Filter::PolygonHas(k) => f.has_polygon() && f.properties.contains_key(k),
        }
    }
}

/// The subclass of `WorldMapStyleLayer` and its own fields.
#[derive(Clone, Debug)]
pub enum LayerKind {
    /// `WorldMapLineStyleLayer`.
    Line {
        fill: Vec<ColorStop>,
        line_width: Vec<FloatStop>,
    },
    /// `WorldMapPolygonStyleLayer`.
    Polygon {
        fill: Vec<ColorStop>,
        texture: Vec<TextureStop>,
        scale: Vec<FloatStop>,
    },
    /// `WorldMapTextureStyleLayer`.
    Texture {
        world_x1: i32,
        world_y1: i32,
        world_x2: i32,
        world_y2: i32,
        use_world_bounds: bool,
        fill: Vec<ColorStop>,
        texture: Vec<TextureStop>,
        tile: bool,
    },
    /// `WorldMapPyramidStyleLayer`.
    Pyramid {
        file_name: Option<Rc<str>>,
        fill: Vec<ColorStop>,
    },
    /// `WorldMapTextStyleLayer`.
    Text {
        font: UIFont,
        line_height: i32,
        fill: Vec<ColorStop>,
    },
}

/// `WorldMapStyleLayer`.
#[derive(Clone, Debug)]
pub struct StyleLayer {
    pub id: Rc<str>,
    pub min_zoom: f32,
    pub filter: Option<Filter>,
    pub filter_key: Option<Rc<str>>,
    pub filter_value: Option<Rc<str>>,
    pub kind: LayerKind,
}

pub type LayerRef = Rc<RefCell<StyleLayer>>;

impl StyleLayer {
    pub fn new(id: &str, kind: LayerKind) -> Self {
        StyleLayer {
            id: id.into(),
            min_zoom: 0.0,
            filter: None,
            filter_key: None,
            filter_value: None,
            kind,
        }
    }
    pub fn new_line(id: &str) -> Self {
        Self::new(
            id,
            LayerKind::Line {
                fill: Vec::new(),
                line_width: Vec::new(),
            },
        )
    }
    pub fn new_polygon(id: &str) -> Self {
        Self::new(
            id,
            LayerKind::Polygon {
                fill: Vec::new(),
                texture: Vec::new(),
                scale: Vec::new(),
            },
        )
    }
    pub fn new_texture(id: &str) -> Self {
        Self::new(
            id,
            LayerKind::Texture {
                world_x1: 0,
                world_y1: 0,
                world_x2: 0,
                world_y2: 0,
                use_world_bounds: false,
                fill: Vec::new(),
                texture: Vec::new(),
                tile: false,
            },
        )
    }
    pub fn new_pyramid(id: &str) -> Self {
        Self::new(
            id,
            LayerKind::Pyramid {
                file_name: None,
                fill: Vec::new(),
            },
        )
    }
    pub fn new_text(id: &str) -> Self {
        Self::new(
            id,
            LayerKind::Text {
                font: UIFont::Handwritten,
                line_height: 40,
                fill: Vec::new(),
            },
        )
    }

    /// `getTypeString()`.
    pub fn type_string(&self) -> &'static str {
        match self.kind {
            LayerKind::Line { .. } => "Line",
            LayerKind::Polygon { .. } => "Polygon",
            LayerKind::Texture { .. } => "Texture",
            LayerKind::Pyramid { .. } => "Pyramid",
            LayerKind::Text { .. } => "Text",
        }
    }

    /// `ignoreFeatures()`.
    pub fn ignore_features(&self) -> bool {
        matches!(
            self.kind,
            LayerKind::Texture { .. } | LayerKind::Pyramid { .. } | LayerKind::Text { .. }
        )
    }

    /// `filter(feature, args)`.
    pub fn filter(&self, f: &WorldMapFeature) -> bool {
        if matches!(self.kind, LayerKind::Texture { .. }) {
            return false;
        }
        self.filter.as_ref().is_some_and(|flt| flt.matches(f))
    }

    /// The `fill` stops of any layer type.
    pub fn fill(&self) -> &Vec<ColorStop> {
        match &self.kind {
            LayerKind::Line { fill, .. }
            | LayerKind::Polygon { fill, .. }
            | LayerKind::Texture { fill, .. }
            | LayerKind::Pyramid { fill, .. }
            | LayerKind::Text { fill, .. } => fill,
        }
    }

    pub fn fill_mut(&mut self) -> &mut Vec<ColorStop> {
        match &mut self.kind {
            LayerKind::Line { fill, .. }
            | LayerKind::Polygon { fill, .. }
            | LayerKind::Texture { fill, .. }
            | LayerKind::Pyramid { fill, .. }
            | LayerKind::Text { fill, .. } => fill,
        }
    }

    /// The texture stops of polygon and texture layers.
    pub fn textures_mut(&mut self) -> Option<&mut Vec<TextureStop>> {
        match &mut self.kind {
            LayerKind::Polygon { texture, .. } | LayerKind::Texture { texture, .. } => {
                Some(texture)
            }
            _ => None,
        }
    }

    /// `WorldMapTextStyleLayer.calculateScale`: `lineHeight / fontHeight`.
    pub fn text_scale(&self, font_height: i32) -> f32 {
        match &self.kind {
            LayerKind::Text { line_height, .. } => *line_height as f32 / font_height as f32,
            _ => 1.0,
        }
    }

    /// The text layer's font (`Handwritten` for other layers).
    pub fn font(&self) -> UIFont {
        match &self.kind {
            LayerKind::Text { font, .. } => *font,
            _ => UIFont::Handwritten,
        }
    }
}

trait Zoomed {
    fn zoom(&self) -> f32;
}
impl Zoomed for ColorStop {
    fn zoom(&self) -> f32 {
        self.zoom
    }
}
impl Zoomed for FloatStop {
    fn zoom(&self) -> f32 {
        self.zoom
    }
}
impl Zoomed for TextureStop {
    fn zoom(&self) -> f32 {
        self.zoom
    }
}

/// `findStop(zoom, stops)`: -2 when empty, -1 before the first stop,
/// else the stop at or before `zoom`.
fn find_stop<S: Zoomed>(zoom: f32, stops: &[S]) -> i32 {
    if stops.is_empty() {
        return -2;
    }
    if zoom <= stops[0].zoom() {
        return -1;
    }
    for i in 0..stops.len() - 1 {
        if zoom <= stops[i + 1].zoom() {
            return i as i32;
        }
    }
    stops.len() as i32 - 1
}

/// The two stops around `zoom` and the blend between them.
fn bracket<S: Zoomed>(zoom: f32, stops: &[S]) -> (usize, usize, f32) {
    let i = find_stop(zoom, stops);
    let i1 = if i == -1 { 0 } else { i };
    let i2 = (i + 1).min(stops.len() as i32 - 1);
    let (s1, s2) = (&stops[i1 as usize], &stops[i2 as usize]);
    let alpha = if i1 == i2 {
        1.0
    } else {
        (clamp(zoom, s1.zoom(), s2.zoom()) - s1.zoom()) / (s2.zoom() - s1.zoom())
    };
    (i1 as usize, i2 as usize, alpha)
}

/// `evalColor(zoom, stops)`.
pub fn eval_color(zoom: f32, stops: &[ColorStop]) -> RGBAf {
    if stops.is_empty() {
        return RGBAf::WHITE;
    }
    let (i1, i2, t) = bracket(zoom, stops);
    let (s1, s2) = (&stops[i1], &stops[i2]);
    RGBAf {
        r: lerp(s1.r as f32, s2.r as f32, t) / 255.0,
        g: lerp(s1.g as f32, s2.g as f32, t) / 255.0,
        b: lerp(s1.b as f32, s2.b as f32, t) / 255.0,
        a: lerp(s1.a as f32, s2.a as f32, t) / 255.0,
    }
}

/// `evalFloat(zoom, stops, emptyValue)`.
pub fn eval_float(zoom: f32, stops: &[FloatStop], empty: f32) -> f32 {
    if stops.is_empty() {
        return empty;
    }
    let (i1, i2, t) = bracket(zoom, stops);
    lerp(stops[i1].f, stops[i2].f, t)
}

/// `evalTextureStop(args, stops)`: the nearer stop between the two around
/// `zoom`, none outside them.
pub fn eval_texture_stop(zoom: f32, stops: &[TextureStop]) -> Option<&TextureStop> {
    if stops.is_empty() {
        return None;
    }
    let (i1, i2, t) = bracket(zoom, stops);
    let (s1, s2) = (&stops[i1], &stops[i2]);
    if i1 == i2 {
        return if zoom < s1.zoom { None } else { Some(s1) };
    }
    if zoom < s1.zoom || zoom > s2.zoom {
        return None;
    }
    Some(if t < 0.5 { s1 } else { s2 })
}

/// Changes to a style, for the API objects that mirror its layers.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleEvent {
    Add(usize),
    Remove(usize),
    Move(usize, usize),
    Clear,
}

/// `zombie.worldMap.styles.WorldMapStyle`.
pub struct WorldMapStyle {
    layers: Vec<LayerRef>,
    layer_by_name: HashMap<Rc<str>, LayerRef>,
    default_text_layer: LayerRef,
}

impl Default for WorldMapStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldMapStyle {
    pub fn new() -> Self {
        let mut t = StyleLayer::new_text("text-note");
        t.fill_mut().push(ColorStop {
            zoom: 0.0,
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
        WorldMapStyle {
            layers: Vec::new(),
            layer_by_name: HashMap::new(),
            default_text_layer: Rc::new(RefCell::new(t)),
        }
    }

    pub fn get_layer_count(&self) -> usize {
        self.layers.len()
    }
    pub fn get_layer_by_index(&self, i: usize) -> Option<&LayerRef> {
        self.layers.get(i)
    }
    pub fn layers(&self) -> &[LayerRef] {
        &self.layers
    }
    /// `getLayerByID(id)`.
    pub fn get_layer_by_id(&self, id: &str) -> Option<&LayerRef> {
        self.layer_by_name.get(id)
    }
    pub fn get_default_text_layer(&self) -> &LayerRef {
        &self.default_text_layer
    }
    /// `getTextStyleLayerOrDefault(id)`.
    pub fn get_text_style_layer_or_default(&self, id: &str) -> LayerRef {
        match self.get_layer_by_id(id) {
            Some(l) if matches!(l.borrow().kind, LayerKind::Text { .. }) => l.clone(),
            _ => self.default_text_layer.clone(),
        }
    }
    /// `addLayer(layer)`.
    pub fn add_layer(&mut self, layer: LayerRef) -> StyleEvent {
        let id = layer.borrow().id.clone();
        self.layers.push(layer.clone());
        self.layer_by_name.insert(id, layer);
        StyleEvent::Add(self.layers.len() - 1)
    }
    /// `insertLayer(index, layer)`.
    pub fn insert_layer(&mut self, index: usize, layer: LayerRef) -> StyleEvent {
        let id = layer.borrow().id.clone();
        self.layers.insert(index, layer.clone());
        self.layer_by_name.insert(id, layer);
        StyleEvent::Add(index)
    }
    /// `removeAt(index)`.
    pub fn remove_at(&mut self, index: usize) -> Option<(LayerRef, StyleEvent)> {
        if index >= self.layers.len() {
            return None;
        }
        let layer = self.layers.remove(index);
        let id = layer.borrow().id.clone();
        self.layer_by_name.remove(&id);
        Some((layer, StyleEvent::Remove(index)))
    }
    /// `removeLayer(layer)`.
    pub fn remove_layer(&mut self, layer: &LayerRef) -> Option<StyleEvent> {
        let i = self.index_of(layer)?;
        self.remove_at(i).map(|(_, e)| e)
    }
    /// `moveLayer(from, to)`.
    pub fn move_layer(&mut self, from: usize, to: usize) -> Option<StyleEvent> {
        if from >= self.layers.len() {
            return None;
        }
        let l = self.layers.remove(from);
        if to > self.layers.len() {
            self.layers.insert(from, l);
            return None;
        }
        self.layers.insert(to, l);
        Some(StyleEvent::Move(from, to))
    }
    /// `setLayerID(layer, id)`.
    pub fn set_layer_id(&mut self, layer: &LayerRef, id: &str) {
        let old = layer.borrow().id.clone();
        self.layer_by_name.remove(&old);
        layer.borrow_mut().id = id.into();
        self.layer_by_name.insert(id.into(), layer.clone());
    }
    /// `indexOf(layer)`.
    pub fn index_of(&self, layer: &LayerRef) -> Option<usize> {
        self.layers.iter().position(|l| Rc::ptr_eq(l, layer))
    }
    /// `clear()`.
    pub fn clear(&mut self) -> StyleEvent {
        self.layers.clear();
        self.layer_by_name.clear();
        StyleEvent::Clear
    }
    /// `copyFrom(other)`: the same layer objects.
    pub fn copy_from(&mut self, other: &WorldMapStyle) {
        self.layers = other.layers.clone();
        self.layer_by_name = other.layer_by_name.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(zoom: f32, a: i32) -> ColorStop {
        ColorStop {
            zoom,
            r: 100,
            g: 50,
            b: 0,
            a,
        }
    }

    #[test]
    fn color_stops() {
        assert_eq!(eval_color(5.0, &[]), RGBAf::WHITE);
        let s = [cs(14.5, 0), cs(15.0, 255)];
        assert_eq!(eval_color(10.0, &s).a, 0.0);
        assert_eq!(eval_color(20.0, &s).a, 1.0);
        assert_eq!(eval_color(14.75, &s).a, 127.5 / 255.0);
        assert_eq!(eval_float(3.0, &[], 1.0), 1.0);
    }

    #[test]
    fn layers_and_ids() {
        let mut st = WorldMapStyle::new();
        let a = Rc::new(RefCell::new(StyleLayer::new_polygon("a")));
        let b = Rc::new(RefCell::new(StyleLayer::new_text("b")));
        st.add_layer(a.clone());
        st.add_layer(b.clone());
        assert!(Rc::ptr_eq(&st.get_text_style_layer_or_default("b"), &b));
        assert!(Rc::ptr_eq(
            &st.get_text_style_layer_or_default("a"),
            st.get_default_text_layer()
        ));
        st.set_layer_id(&a, "c");
        assert!(st.get_layer_by_id("a").is_none());
        assert_eq!(st.move_layer(0, 1), Some(StyleEvent::Move(0, 1)));
        assert_eq!(st.index_of(&a), Some(1));
        assert_eq!(st.remove_layer(&b), Some(StyleEvent::Remove(0)));
        assert_eq!(st.get_layer_count(), 1);
    }

    /// `evalColor` and `evalFloat` over a zoom sweep for 0..4 stops
    /// against the original (oracle `style.txt`).
    #[test]
    fn matches_style_oracle() {
        let Some(dir) = std::env::var_os("PZ_WORLDMAP_ORACLE_DIR") else {
            eprintln!("PZ_WORLDMAP_ORACLE_DIR not set; skipping");
            return;
        };
        let want = std::fs::read_to_string(std::path::Path::new(&dir).join("style.txt")).unwrap();
        let cs = [
            (0.0f32, 189, 197, 163, 0),
            (14.5, 189, 197, 163, 0),
            (15.0, 189, 197, 163, 255),
            (24.0, 10, 20, 30, 255),
        ];
        let fs = [(13.5f32, 4.0f32), (18.0, 7.25), (20.0, 1.0)];
        let b = |f: f32| format!("{:x}", f.to_bits());
        let mut out = Vec::new();
        for n in 0..=cs.len() {
            let fill: Vec<ColorStop> = cs[..n]
                .iter()
                .map(|&(zoom, r, g, b, a)| ColorStop { zoom, r, g, b, a })
                .collect();
            let scale: Vec<FloatStop> = fs[..n.min(fs.len())]
                .iter()
                .map(|&(zoom, f)| FloatStop { zoom, f })
                .collect();
            let mut z = -1.0f32;
            while z <= 25.0 {
                let c = eval_color(z, &fill);
                let f = eval_float(z, &scale, 1.0);
                out.push(format!(
                    "{n} {} {} {} {} {} {}",
                    b(z),
                    b(c.r),
                    b(c.g),
                    b(c.b),
                    b(c.a),
                    b(f)
                ));
                z += 0.125;
            }
        }
        assert_eq!(out, want.lines().collect::<Vec<_>>());
    }
}
