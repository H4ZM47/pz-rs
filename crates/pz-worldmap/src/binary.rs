//! `zombie.worldMap.WorldMapBinary`: the `worldmap*.xml.bin` reader
//! (magic `IGMB`, version 2, 256-square cells, a string table, then the
//! cells row by row).

use crate::data::{
    GeometryType, SharedProperties, WorldMapCell, WorldMapFeature, WorldMapGeometry,
    WorldMapPoints, WorldMapProperties,
};
use std::collections::HashMap;
use std::path::Path;

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    /// `InputStream.read()`: -1 at the end.
    fn byte(&mut self) -> i32 {
        match self.b.get(self.pos) {
            Some(&v) => {
                self.pos += 1;
                v as i32
            }
            None => -1,
        }
    }
    fn int(&mut self) -> Result<i32, String> {
        let (a, b, c, d) = (self.byte(), self.byte(), self.byte(), self.byte());
        if (a | b | c | d) < 0 {
            return Err("EOFException".into());
        }
        Ok(a.wrapping_add(b << 8)
            .wrapping_add(c << 16)
            .wrapping_add(d << 24))
    }
    fn short(&mut self) -> Result<i16, String> {
        let (a, b) = (self.byte(), self.byte());
        if (a | b) < 0 {
            return Err("EOFException".into());
        }
        Ok((a + (b << 8)) as i16)
    }
}

/// `read(filePath, data)`: the cells, or the `IOException` message.
pub fn read(path: &Path) -> Result<Vec<WorldMapCell>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    read_bytes(&bytes)
}

pub fn read_bytes(bytes: &[u8]) -> Result<Vec<WorldMapCell>, String> {
    let mut r = Reader { b: bytes, pos: 0 };
    if (r.byte(), r.byte(), r.byte(), r.byte()) != (73, 71, 77, 66) {
        return Err("invalid format (magic doesn't match)".into());
    }
    let version = r.int()?;
    if !(1..=2).contains(&version) {
        return Err(format!("unrecognized version {version}"));
    }
    if version == 1 {
        return Err("version 1 (cell size 300) is not supported".into());
    }
    let cell_size = r.int()?;
    let width = r.int()?;
    let height = r.int()?;
    if cell_size != 256 {
        return Err(format!("unsupported cell size {cell_size}"));
    }
    // readStringTable: little-endian length, then the bytes read as
    // GameWindow.ReadString (UTF-8).
    let mut table: HashMap<i32, String> = HashMap::new();
    let n = r.int()?;
    for i in 0..n {
        let len = r.short()?;
        let len = len.max(0) as usize;
        let end = (r.pos + len).min(bytes.len());
        let s = String::from_utf8_lossy(&bytes[r.pos..end]).into_owned();
        r.pos = end;
        table.insert(i, s);
    }
    let mut shared = SharedProperties::default();
    let mut cells = Vec::new();
    for _ in 0..height {
        for _ in 0..width {
            if let Some(c) = parse_cell(&mut r, &table, &mut shared)? {
                cells.push(c);
            }
        }
    }
    Ok(cells)
}

fn indexed<'t>(r: &mut Reader, table: &'t HashMap<i32, String>) -> Result<&'t str, String> {
    let i = r.short()? as i32;
    table
        .get(&i)
        .map(String::as_str)
        .ok_or_else(|| format!("invalid string-table index {i}"))
}

fn parse_cell(
    r: &mut Reader,
    table: &HashMap<i32, String>,
    shared: &mut SharedProperties,
) -> Result<Option<WorldMapCell>, String> {
    let x = r.int()?;
    if x == -1 {
        return Ok(None);
    }
    let y = r.int()?;
    let mut cell = WorldMapCell::new(x, y);
    let n = r.int()?;
    for _ in 0..n {
        // parseGeometry
        let kind = GeometryType::value_of(indexed(r, table)?).ok_or("No enum constant")?;
        let mut g = WorldMapGeometry::new(kind);
        let rings = r.byte();
        for _ in 0..rings {
            let np = r.short()? as i32;
            let first = cell.points.len();
            for _ in 0..np {
                let px = r.short()?;
                let py = r.short()?;
                cell.points.push(px);
                cell.points.push(py);
            }
            g.points.push(WorldMapPoints {
                first_point: first,
                point_count: np.max(0) as usize,
                min_x: 0,
                min_y: 0,
                max_x: 0,
                max_y: 0,
            });
        }
        g.calculate_bounds(&cell.points);
        // parseFeatureProperties
        let mut p = WorldMapProperties::default();
        let np = r.byte();
        for _ in 0..np {
            let k = shared.string(indexed(r, table)?);
            let v = shared.string(indexed(r, table)?);
            p.map.insert(k, v);
        }
        let properties = shared.get_or_create(p);
        cell.features.push(WorldMapFeature {
            geometry: Some(g),
            properties,
        });
    }
    Ok(Some(cell))
}
