//! `WorldMapGeometry.triangulate`: a polygon cut to its cell (and to the
//! cells of higher-priority map directories) and triangulated with the
//! game's Clipper, into its cell's triangle and index buffers.

use crate::data::{GeometryTriangles, WorldMapCell, WorldMapGeometry};
use pz_clipper::Clipper;

/// `MapFiles.getCurrentMapFiles()` as the triangulation reads it. Owner:
/// the world workstream (pz-world's map directories); the default has
/// none, so no cell is clipped and every cell keeps priority -1.
pub trait MapFilesInfo {
    /// `mapDirectoryName` of each `MapFiles`, highest priority first.
    fn map_directory_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// `mapFiles.get(index).hasCell300(x, y)`.
    fn has_cell300(&self, _index: usize, _x: i32, _y: i32) -> bool {
        false
    }
}

/// No map directories.
pub struct NoMapFiles;
impl MapFilesInfo for NoMapFiles {}

/// The original throws here (`indexBuffer.get` with a negative index, or
/// `trimToSize` on a missing list); the drawer stops for the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawAbort(pub &'static str);

fn ring_floats(cell: &WorldMapCell, g: &WorldMapGeometry, ring: usize) -> Vec<f32> {
    let p = &g.points[ring];
    let n = p.num_points();
    let mut v = Vec::with_capacity(n * 2);
    let order: Box<dyn Iterator<Item = usize>> = if p.is_clockwise(&cell.points) {
        Box::new((0..n).rev())
    } else {
        Box::new(0..n)
    };
    for i in order {
        v.push(p.get_x(&cell.points, i) as f32);
        v.push(p.get_y(&cell.points, i) as f32);
    }
    v
}

/// `cell.clipHigherPriorityCells`, computed once per cell.
fn clip_cells(cell: &WorldMapCell, files: &dyn MapFilesInfo) -> Vec<i32> {
    if let Some(v) = cell.clip_higher_priority_cells.borrow().as_ref() {
        return v.clone();
    }
    let min_x = ((cell.x as f32 * 256.0 / 300.0) as f64).floor() as i32;
    let min_y = ((cell.y as f32 * 256.0 / 300.0) as f64).floor() as i32;
    let max_x = (((cell.x + 1) as f32 * 256.0 / 300.0) as f64).floor() as i32;
    let max_y = (((cell.y + 1) as f32 * 256.0 / 300.0) as f64).floor() as i32;
    let mut v = Vec::new();
    for i in 0..cell.priority.get().max(0) as usize {
        for cy in min_y..=max_y {
            for cx in min_x..=max_x {
                if !files.has_cell300(i, cx, cy) {
                    continue;
                }
                let x1 = cx * 300 - cell.x * 256;
                let y1 = cy * 300 - cell.y * 256;
                // Stored as floats and read back with roundToInt.
                v.extend([x1, y1, x1 + 300, y1 + 300]);
            }
        }
    }
    *cell.clip_higher_priority_cells.borrow_mut() = Some(v.clone());
    v
}

/// Appends one triangulated polygon to the cell's buffers: returns its
/// first index position and index count.
fn append(cell: &WorldMapCell, verts: &[f32], idx: &[u16]) -> (usize, usize) {
    let mut tris = cell.triangle_buffer.borrow_mut();
    let first_point = tris.len() / 2;
    tris.extend_from_slice(verts);
    let mut ib = cell.index_buffer.borrow_mut();
    let first = ib.len();
    // (short)(firstPoint + index): wraps past 32767, read back & 0xFFFF.
    ib.extend(idx.iter().map(|&i| (first_point as i32 + i as i32) as i16));
    (first, idx.len())
}

/// `triangulate(cell, delta)`: fills `geometry.triangles` (a negative or
/// zero index count marks a failure the caller records).
pub fn triangulate(
    cell: &WorldMapCell,
    g: &WorldMapGeometry,
    delta: Option<&[f64]>,
    files: &dyn MapFilesInfo,
) -> Result<(), DrawAbort> {
    let mut clipper = Clipper::new();
    let outer = ring_floats(cell, g, 0);
    let _ = clipper.add_path(g.points[0].num_points() as i32, &outer, false, true);
    for i in 1..g.points.len() {
        let hole = ring_floats(cell, g, i);
        let _ = clipper.add_path(g.points[i].num_points() as i32, &hole, true, true);
    }
    if g.min_x < 0 || g.min_y < 0 || g.max_x > 256 || g.max_y > 256 {
        let frame: [f32; 20] = [
            -768.0, -768.0, 1024.0, -768.0, 1024.0, 1024.0, -768.0, 1024.0, -768.0, 0.0, 0.0, 0.0,
            0.0, 256.0, 256.0, 256.0, 256.0, 0.0, -768.0, 0.0,
        ];
        let _ = clipper.add_path(10, &frame, true, true);
    }
    let clips = clip_cells(cell, files);
    for c in clips.as_chunks::<4>().0 {
        clipper.clip_aabb(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32);
    }
    let mut t = GeometryTriangles {
        first_index: 0,
        index_count: 0,
        per_zoom: Vec::new(),
        failed: false,
    };
    let num = clipper.generate_polygons_delta(0.0);
    if num <= 0 {
        *g.triangles.borrow_mut() = Some(t);
        return Ok(());
    }
    t.first_index = -1;
    for i in 0..num {
        let Ok((verts, idx)) = clipper.triangulate2(i) else {
            continue;
        };
        if idx.len() < 3 {
            continue;
        }
        let (first, n) = append(cell, &verts, &idx);
        if t.first_index == -1 {
            // (short)indexBuffer.position()
            t.first_index = first as i16 as i32;
        }
        t.index_count = (t.index_count as i16).wrapping_add(n as i16) as i32;
    }
    let Some(delta) = delta else {
        *g.triangles.borrow_mut() = Some(t);
        return Ok(());
    };
    let mut any = false;
    for i in 0..delta.len() {
        let d2 = delta[i] - if i == 0 { 0.0 } else { delta[i - 1] };
        let num = clipper.generate_polygons_delta(d2);
        if num <= 0 {
            continue;
        }
        let (mut first_index, mut count) = (-1i32, -1i16);
        for j in 0..num {
            let Ok((verts, idx)) = clipper.triangulate2(j) else {
                continue;
            };
            if idx.len() < 3 {
                continue;
            }
            let (first, n) = append(cell, &verts, &idx);
            if first_index == -1 {
                first_index = first as i32;
            }
            count = count.wrapping_add(n as i16);
        }
        if count < 3 {
            continue;
        }
        any = true;
        t.per_zoom.push((delta[i], first_index, count as i32));
    }
    *g.triangles.borrow_mut() = Some(t);
    if !any {
        // trianglesPerZoom.trimToSize() on null.
        return Err(DrawAbort(
            "NullPointerException in WorldMapGeometry.triangulate",
        ));
    }
    Ok(())
}
