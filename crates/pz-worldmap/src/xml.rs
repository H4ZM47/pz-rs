//! `zombie.worldMap.WorldMapXML`: the `worldmap*.xml` reader, used when
//! no `.bin` sits beside the file. As in the original, features read from
//! XML keep their properties but lose any geometry with points (see
//! `close`), so only the `.bin` files draw.

use crate::data::{
    GeometryType, SharedProperties, WorldMapCell, WorldMapFeature, WorldMapGeometry,
    WorldMapPoints, WorldMapProperties,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::path::Path;

fn attr(e: &BytesStart, name: &str) -> String {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == name.as_bytes() {
            return a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_default();
        }
    }
    String::new()
}

/// `PZMath.tryParseInt(s, 0)`.
fn parse_int(s: &str) -> i32 {
    s.trim().parse().unwrap_or(0)
}

/// `read(filePath, data)`: the cells (empty when the root is not
/// `world`).
pub fn read(path: &Path) -> Result<Vec<WorldMapCell>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    read_bytes(&bytes)
}

/// Where in the document the reader is.
enum Ctx {
    None,
    Cell,
    Feature,
    Geometry,
    Coordinates,
    Properties,
}

pub fn read_bytes(bytes: &[u8]) -> Result<Vec<WorldMapCell>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    let mut shared = SharedProperties::default();
    let mut cells = Vec::new();
    let mut depth = 0;
    let mut world = false;
    let mut cell: Option<WorldMapCell> = None;
    let mut feature: Option<(Option<WorldMapGeometry>, Option<WorldMapProperties>)> = None;
    let mut geometry: Option<WorldMapGeometry> = None;
    let mut props: Option<WorldMapProperties> = None;
    let mut ring: Option<usize> = None;
    let mut ctx = Ctx::None;
    loop {
        let ev = reader.read_event().map_err(|e| e.to_string())?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(_) => {
                depth -= 1;
                close(
                    &mut ctx,
                    &mut cells,
                    &mut cell,
                    &mut feature,
                    &mut geometry,
                    &mut props,
                    &mut ring,
                    &mut shared,
                );
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let e = e.unwrap();
        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
        if depth == 0 {
            if name != "world" {
                return Ok(Vec::new());
            }
            world = true;
            if !empty {
                depth += 1;
            }
            continue;
        }
        if !world {
            continue;
        }
        match ctx {
            Ctx::None if name == "cell" => {
                cell = Some(WorldMapCell::new(
                    parse_int(&attr(&e, "x")),
                    parse_int(&attr(&e, "y")),
                ));
                ctx = Ctx::Cell;
            }
            Ctx::Cell if name.eq_ignore_ascii_case("feature") => {
                feature = Some((None, None));
                ctx = Ctx::Feature;
            }
            Ctx::Feature if name.eq_ignore_ascii_case("geometry") => {
                let kind = GeometryType::value_of(&attr(&e, "type"));
                geometry = kind.map(WorldMapGeometry::new);
                ctx = Ctx::Geometry;
            }
            Ctx::Feature if name.eq_ignore_ascii_case("properties") => {
                props = Some(WorldMapProperties::default());
                ctx = Ctx::Properties;
            }
            Ctx::Geometry if name.eq_ignore_ascii_case("coordinates") => {
                if let (Some(g), Some(c)) = (geometry.as_mut(), cell.as_ref()) {
                    g.points.push(WorldMapPoints {
                        first_point: c.points.len(),
                        point_count: 0,
                        min_x: 0,
                        min_y: 0,
                        max_x: 0,
                        max_y: 0,
                    });
                    ring = Some(g.points.len() - 1);
                }
                ctx = Ctx::Coordinates;
            }
            Ctx::Coordinates if name.eq_ignore_ascii_case("point") => {
                if let (Some(g), Some(c), Some(i)) = (geometry.as_mut(), cell.as_mut(), ring) {
                    c.points.push(parse_int(&attr(&e, "x")) as i16);
                    c.points.push(parse_int(&attr(&e, "y")) as i16);
                    g.points[i].point_count += 1;
                }
                // Not a context; an empty or closed <point> ends here.
                if !empty {
                    depth += 1;
                    let _ = reader.read_to_end(e.name()).map_err(|e| e.to_string())?;
                    depth -= 1;
                }
                continue;
            }
            Ctx::Properties if name.eq_ignore_ascii_case("property") => {
                let k = shared.string(&attr(&e, "name"));
                let v = shared.string(&attr(&e, "value"));
                if let Some(p) = props.as_mut() {
                    p.map.insert(k, v);
                }
                if !empty {
                    let _ = reader.read_to_end(e.name()).map_err(|e| e.to_string())?;
                }
                continue;
            }
            _ => {
                // Unrecognised element: skipped with its contents.
                if !empty {
                    let _ = reader.read_to_end(e.name()).map_err(|e| e.to_string())?;
                }
                continue;
            }
        }
        if empty {
            close(
                &mut ctx,
                &mut cells,
                &mut cell,
                &mut feature,
                &mut geometry,
                &mut props,
                &mut ring,
                &mut shared,
            );
        } else {
            depth += 1;
        }
    }
    Ok(cells)
}

#[allow(clippy::too_many_arguments)]
fn close(
    ctx: &mut Ctx,
    cells: &mut Vec<WorldMapCell>,
    cell: &mut Option<WorldMapCell>,
    feature: &mut Option<(Option<WorldMapGeometry>, Option<WorldMapProperties>)>,
    geometry: &mut Option<WorldMapGeometry>,
    props: &mut Option<WorldMapProperties>,
    ring: &mut Option<usize>,
    shared: &mut SharedProperties,
) {
    *ctx = match ctx {
        Ctx::Cell => {
            if let Some(c) = cell.take() {
                cells.push(c);
            }
            Ctx::None
        }
        Ctx::Feature => {
            if let (Some((g, p)), Some(c)) = (feature.take(), cell.as_mut()) {
                // A feature without properties keeps a null map in the
                // original; an empty shared map stands in.
                let properties = shared.get_or_create(p.unwrap_or_default());
                c.features.push(WorldMapFeature {
                    geometry: g,
                    properties,
                });
            }
            Ctx::Cell
        }
        Ctx::Geometry => {
            if let (Some(mut g), Some(_)) = (geometry.take(), cell.as_ref()) {
                // The original's parseGeometryCoordinates stores the ring
                // length in shorts (two per point) in an exactly-sized point
                // buffer, so calculateBounds reads past its end and the
                // exception drops the geometry whenever a ring has points.
                if g.points.iter().all(|p| p.point_count == 0) {
                    g.calculate_bounds(&[]);
                    if let Some(f) = feature.as_mut() {
                        f.0 = Some(g);
                    }
                }
            }
            Ctx::Feature
        }
        Ctx::Coordinates => {
            *ring = None;
            Ctx::Geometry
        }
        Ctx::Properties => {
            if let (Some(p), Some(f)) = (props.take(), feature.as_mut()) {
                f.1 = Some(p);
            }
            Ctx::Feature
        }
        Ctx::None => Ctx::None,
    };
}
