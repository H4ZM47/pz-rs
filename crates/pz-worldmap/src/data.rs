//! `zombie.worldMap.WorldMapData` and its contents (`WorldMapCell`,
//! `WorldMapFeature`, `WorldMapGeometry`, `WorldMapPoints`,
//! `WorldMapProperties`): one map data file (`worldmap.xml` or its
//! `.bin`), shared by every map UI that shows it.
//!
//! The original loads the asset on a file thread; here
//! [`get_or_create_data`] loads it at once, so the asset is ready (or
//! failed) when it is returned.

use crate::world_map::CELL_SIZE;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// `Asset.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetState {
    Empty,
    Ready,
    Failure,
}

/// `WorldMapGeometry.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryType {
    LineString,
    Point,
    Polygon,
}

impl GeometryType {
    /// `Type.valueOf(name)`.
    pub fn value_of(s: &str) -> Option<Self> {
        match s {
            "LineString" => Some(Self::LineString),
            "Point" => Some(Self::Point),
            "Polygon" => Some(Self::Polygon),
            _ => None,
        }
    }
}

/// `WorldMapProperties`: a feature's string properties, shared between
/// features with equal properties.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldMapProperties {
    pub map: HashMap<Rc<str>, Rc<str>>,
}

impl WorldMapProperties {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|v| &**v)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
}

/// `WorldMapPoints`: one ring or line of a geometry, a slice of its cell's
/// point buffer.
#[derive(Clone, Debug)]
pub struct WorldMapPoints {
    /// Index of the first `x` in the cell's point buffer.
    pub first_point: usize,
    pub point_count: usize,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl WorldMapPoints {
    pub fn num_points(&self) -> usize {
        self.point_count
    }
    pub fn get_x(&self, buf: &[i16], i: usize) -> i32 {
        buf[self.first_point + i * 2] as i32
    }
    pub fn get_y(&self, buf: &[i16], i: usize) -> i32 {
        buf[self.first_point + i * 2 + 1] as i32
    }
    /// `calculateBounds()`.
    pub fn calculate_bounds(&mut self, buf: &[i16]) {
        self.min_x = i32::MAX;
        self.min_y = i32::MAX;
        self.max_x = i32::MIN;
        self.max_y = i32::MIN;
        for i in 0..self.num_points() {
            let (px, py) = (self.get_x(buf, i), self.get_y(buf, i));
            self.min_x = self.min_x.min(px);
            self.min_y = self.min_y.min(py);
            self.max_x = self.max_x.max(px);
            self.max_y = self.max_y.max(py);
        }
    }
    /// `isClockwise()`.
    pub fn is_clockwise(&self, buf: &[i16]) -> bool {
        let n = self.num_points();
        let mut sum = 0.0f32;
        for i in 0..n {
            let (p1x, p1y) = (self.get_x(buf, i), self.get_y(buf, i));
            let (p2x, p2y) = (self.get_x(buf, (i + 1) % n), self.get_y(buf, (i + 1) % n));
            sum += ((p2x - p1x) * (p2y + p1y)) as f32;
        }
        sum as f64 > 0.0
    }
}

/// Triangles a geometry was cut into (`firstIndex`/`indexCount` in its
/// cell's index buffer), per zoom delta (`TrianglesPerZoom`).
#[derive(Clone, Debug, Default)]
pub struct GeometryTriangles {
    pub first_index: i32,
    pub index_count: i32,
    pub per_zoom: Vec<(f64, i32, i32)>,
    pub failed: bool,
}

/// `WorldMapGeometry`.
#[derive(Debug)]
pub struct WorldMapGeometry {
    pub kind: GeometryType,
    pub points: Vec<WorldMapPoints>,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    /// Set once triangulated (`firstIndex != -1`).
    pub triangles: RefCell<Option<GeometryTriangles>>,
}

impl WorldMapGeometry {
    pub fn new(kind: GeometryType) -> Self {
        WorldMapGeometry {
            kind,
            points: Vec::new(),
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            triangles: RefCell::new(None),
        }
    }

    /// `calculateBounds()`.
    pub fn calculate_bounds(&mut self, buf: &[i16]) {
        self.min_x = i32::MAX;
        self.min_y = i32::MAX;
        self.max_x = i32::MIN;
        self.max_y = i32::MIN;
        for p in &mut self.points {
            p.calculate_bounds(buf);
            self.min_x = self.min_x.min(p.min_x);
            self.min_y = self.min_y.min(p.min_y);
            self.max_x = self.max_x.max(p.max_x);
            self.max_y = self.max_y.max(p.max_y);
        }
    }

    /// `containsPoint(x, y)` in cell-local squares.
    pub fn contains_point(&self, buf: &[i16], x: f32, y: f32) -> bool {
        if self.kind != GeometryType::Polygon || self.points.is_empty() {
            return false;
        }
        self.winding_number(buf, x, y) != 0
    }

    /// `isPointInPolygon_WindingNumber` over the outer ring.
    fn winding_number(&self, buf: &[i16], x: f32, y: f32) -> i32 {
        let is_left = |x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32| {
            (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
        };
        let outer = &self.points[0];
        let n = outer.num_points();
        let mut wn = 0;
        for i in 0..n {
            let (x1, y1) = (outer.get_x(buf, i) as f32, outer.get_y(buf, i) as f32);
            let (x2, y2) = (
                outer.get_x(buf, (i + 1) % n) as f32,
                outer.get_y(buf, (i + 1) % n) as f32,
            );
            if y1 <= y {
                if y2 > y && is_left(x1, y1, x2, y2, x, y) > 0.0 {
                    wn += 1;
                }
            } else if y2 <= y && is_left(x1, y1, x2, y2, x, y) < 0.0 {
                wn -= 1;
            }
        }
        wn
    }
}

/// `WorldMapFeature`.
#[derive(Debug)]
pub struct WorldMapFeature {
    pub geometry: Option<WorldMapGeometry>,
    pub properties: Rc<WorldMapProperties>,
}

impl WorldMapFeature {
    pub fn has_line_string(&self) -> bool {
        self.geometry
            .as_ref()
            .is_some_and(|g| g.kind == GeometryType::LineString)
    }
    pub fn has_point(&self) -> bool {
        self.geometry
            .as_ref()
            .is_some_and(|g| g.kind == GeometryType::Point)
    }
    pub fn has_polygon(&self) -> bool {
        self.geometry
            .as_ref()
            .is_some_and(|g| g.kind == GeometryType::Polygon)
    }
}

/// `WorldMapCell`: the features of one 256x256 cell; coordinates are
/// cell-local squares in `points`.
#[derive(Debug, Default)]
pub struct WorldMapCell {
    pub x: i32,
    pub y: i32,
    pub features: Vec<WorldMapFeature>,
    /// `pointBuffer`: x, y pairs.
    pub points: Vec<i16>,
    /// `priority`: index of the map directory the cell comes from, -1
    /// until the renderer sets it.
    pub priority: Cell<i32>,
    /// `clipHigherPriorityCells`: rectangles (x1, y1, x2, y2) cut out of
    /// this cell's polygons.
    pub clip_higher_priority_cells: RefCell<Option<Vec<i32>>>,
    /// `triangleBuffer`: x, y pairs of triangle vertices.
    pub triangle_buffer: RefCell<Vec<f32>>,
    /// `indexBuffer`.
    pub index_buffer: RefCell<Vec<i16>>,
}

impl WorldMapCell {
    pub fn new(x: i32, y: i32) -> Self {
        WorldMapCell {
            x,
            y,
            priority: Cell::new(-1),
            ..Default::default()
        }
    }

    /// `hitTest(x, y, features)` with world squares: the indices of the
    /// features whose polygon contains the point.
    pub fn hit_test(&self, x: f32, y: f32, out: &mut Vec<usize>) {
        let x = x - (self.x * CELL_SIZE) as f32;
        let y = y - (self.y * CELL_SIZE) as f32;
        for (i, f) in self.features.iter().enumerate() {
            if f.geometry
                .as_ref()
                .is_some_and(|g| g.contains_point(&self.points, x, y))
            {
                out.push(i);
            }
        }
    }

    /// `clearTriangles()`.
    pub fn clear_triangles(&self) {
        self.priority.set(-1);
        *self.clip_higher_priority_cells.borrow_mut() = None;
        for f in &self.features {
            if let Some(g) = &f.geometry {
                *g.triangles.borrow_mut() = None;
            }
        }
        self.index_buffer.borrow_mut().clear();
        self.triangle_buffer.borrow_mut().clear();
    }
}

/// `zombie.worldMap.WorldMapData`.
pub struct WorldMapData {
    /// The absolute path the asset was created for.
    pub path: Rc<str>,
    /// `relativeFileName`: the name the last `addData` used.
    pub relative_file_name: RefCell<Rc<str>>,
    pub state: Cell<AssetState>,
    /// `minX, minY, maxX, maxY` in cells.
    pub bounds: Cell<[i32; 4]>,
    pub cells: RefCell<Vec<Rc<WorldMapCell>>>,
    cell_lookup: RefCell<HashMap<(i32, i32), Rc<WorldMapCell>>>,
}

impl WorldMapData {
    pub fn new(path: &str) -> Self {
        WorldMapData {
            path: path.into(),
            relative_file_name: RefCell::new("".into()),
            state: Cell::new(AssetState::Empty),
            bounds: Cell::new([0; 4]),
            cells: RefCell::new(Vec::new()),
            cell_lookup: RefCell::new(HashMap::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.state.get() == AssetState::Empty
    }
    pub fn is_ready(&self) -> bool {
        self.state.get() == AssetState::Ready
    }
    pub fn is_failure(&self) -> bool {
        self.state.get() == AssetState::Failure
    }

    pub fn get_width_in_cells(&self) -> i32 {
        let b = self.bounds.get();
        b[2] - b[0] + 1
    }
    pub fn get_height_in_cells(&self) -> i32 {
        let b = self.bounds.get();
        b[3] - b[1] + 1
    }

    /// The loader's result: cells in file order; `onLoaded` builds the
    /// lookup and bounds.
    pub fn set_loaded(&self, cells: Vec<WorldMapCell>) {
        let cells: Vec<Rc<WorldMapCell>> = cells.into_iter().map(Rc::new).collect();
        let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        let mut lookup = HashMap::new();
        for c in &cells {
            lookup.insert((c.x, c.y), c.clone());
            b = [b[0].min(c.x), b[1].min(c.y), b[2].max(c.x), b[3].max(c.y)];
        }
        *self.cells.borrow_mut() = cells;
        *self.cell_lookup.borrow_mut() = lookup;
        self.bounds.set(b);
        self.state.set(AssetState::Ready);
    }

    /// `getCell(x, y)`.
    pub fn get_cell(&self, x: i32, y: i32) -> Option<Rc<WorldMapCell>> {
        self.cell_lookup.borrow().get(&(x, y)).cloned()
    }

    /// `hitTest(x, y, features)`: the cell and its feature indices under a
    /// world square.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<(Rc<WorldMapCell>, Vec<usize>)> {
        let cx = crate::renderer::fast_floor(x / 256.0);
        let cy = crate::renderer::fast_floor(y / 256.0);
        let b = self.bounds.get();
        if cx < b[0] || cx > b[2] || cy < b[1] || cy > b[3] {
            return None;
        }
        let cell = self.get_cell(cx, cy)?;
        let mut v = Vec::new();
        cell.hit_test(x, y, &mut v);
        Some((cell, v))
    }

    /// `clearTriangles()`.
    pub fn clear_triangles(&self) {
        for c in self.cells.borrow().iter() {
            c.clear_triangles();
        }
    }
}

thread_local! {
    /// `WorldMapData.s_fileNameToData`.
    static FILE_NAME_TO_DATA: RefCell<HashMap<String, Rc<WorldMapData>>> = RefCell::new(HashMap::new());
}

/// `WorldMapData.getOrCreateData(fileName)` with an absolute path: the
/// shared asset, loaded (from `fileName.bin` when that exists, else the
/// XML); `None` when the file does not exist.
pub fn get_or_create_data(file_name: &str) -> Option<Rc<WorldMapData>> {
    if let Some(d) = FILE_NAME_TO_DATA.with(|m| m.borrow().get(file_name).cloned()) {
        return Some(d);
    }
    if !Path::new(file_name).exists() {
        return None;
    }
    let data = Rc::new(WorldMapData::new(file_name));
    let bin = format!("{file_name}.bin");
    let result = if Path::new(&bin).exists() {
        crate::binary::read(Path::new(&bin))
    } else {
        crate::xml::read(Path::new(file_name))
    };
    match result {
        Ok(cells) => data.set_loaded(cells),
        Err(e) => {
            pz_core::config::debug_log(&format!("Failed to load asset: {file_name} ({e})"));
            data.state.set(AssetState::Failure);
        }
    }
    FILE_NAME_TO_DATA.with(|m| m.borrow_mut().insert(file_name.to_string(), data.clone()));
    Some(data)
}

/// `WorldMapData.Reset()`: drops every data file's triangles.
pub fn reset() {
    FILE_NAME_TO_DATA.with(|m| {
        for d in m.borrow().values() {
            d.clear_triangles();
        }
    });
}

/// Shares equal property maps (`getOrCreateProperties`).
#[derive(Default)]
pub struct SharedProperties {
    list: Vec<Rc<WorldMapProperties>>,
    strings: HashMap<Rc<str>, Rc<str>>,
}

impl SharedProperties {
    /// `SharedStrings.get(s)`.
    pub fn string(&mut self, s: &str) -> Rc<str> {
        if let Some(v) = self.strings.get(s) {
            return v.clone();
        }
        let v: Rc<str> = s.into();
        self.strings.insert(v.clone(), v.clone());
        v
    }

    pub fn get_or_create(&mut self, p: WorldMapProperties) -> Rc<WorldMapProperties> {
        if let Some(s) = self.list.iter().find(|s| ***s == p) {
            return s.clone();
        }
        let r = Rc::new(p);
        self.list.push(r.clone());
        r
    }
}
