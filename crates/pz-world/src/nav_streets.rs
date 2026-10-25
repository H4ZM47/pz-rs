//! The map side of `zombie.worldMap.streets`: reading a map's
//! `streets.xml` (`WorldMapStreetsXML.read`) and the `Nav` zones each
//! street registers (`WorldMapStreet.registerNavZones`), which
//! `IsoWorld.registerNavZones` runs for every lot directory on
//! `OnLoadMapZones` (`metazoneHandler.lua`'s `initDefaultStreetData`).
//!
//! Only what the zone registration reads is kept: the untranslated name,
//! the width and the points. The world-map side of the streets (lookup,
//! intersections, rendering, editing) is not here.

use crate::meta_grid::IsoMetaGrid;
use crate::zones::ZoneGeometryType;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// `WorldMapStreet.NAV_ZONE`.
pub const NAV_ZONE: &str = "Nav";

/// One `<street>` of `streets.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Street {
    /// `untranslatedText` (the `name` attribute).
    pub name: String,
    /// `width` (default 5).
    pub width: i32,
    /// `StreetPoints`: flat `x, y` floats.
    pub points: Vec<f32>,
}

impl Street {
    /// `getNumPoints()`.
    pub fn num_points(&self) -> usize {
        self.points.len() / 2
    }

    fn x(&self, i: usize) -> f32 {
        self.points[i * 2]
    }

    fn y(&self, i: usize) -> f32 {
        self.points[i * 2 + 1]
    }

    /// `WorldMapStreet.registerNavZones()`: axis-aligned segments become
    /// rectangle zones, runs of diagonal segments polyline zones (a new
    /// polyline starts when a segment ends more than 512 squares from the
    /// polyline's first point). Railroads register nothing.
    pub fn register_nav_zones(
        &self,
        grid: &mut IsoMetaGrid,
        railroad_strings: &[String],
    ) -> Result<(), String> {
        let lower = self.name.to_lowercase();
        if railroad_strings
            .iter()
            .any(|s| lower.contains(&s.to_lowercase()))
        {
            return Ok(());
        }
        let mut line: Vec<i32> = Vec::new();
        for i in 0..self.num_points().saturating_sub(1) {
            let (x1, y1) = (self.x(i) as i32, self.y(i) as i32);
            let (x2, y2) = (self.x(i + 1) as i32, self.y(i + 1) as i32);
            if x1 == x2 || y1 == y2 {
                self.register_nav_zone_polyline(grid, &mut line)?;
                self.register_nav_zone(grid, x1, y1, x2, y2);
                continue;
            }
            if !(line.is_empty()
                || (x2.wrapping_sub(line[0]).wrapping_abs() <= 512
                    && y2.wrapping_sub(line[1]).wrapping_abs() <= 512))
            {
                self.register_nav_zone_polyline(grid, &mut line)?;
            }
            line.extend_from_slice(&[x1, y1, x2, y2]);
        }
        self.register_nav_zone_polyline(grid, &mut line)
    }

    /// `registerNavZone(x1, y1, x2, y2)`.
    fn register_nav_zone(&self, grid: &mut IsoMetaGrid, x1: i32, y1: i32, x2: i32, y2: i32) {
        let w = self.width;
        let x = if x1 == x2 { x1 - w / 2 } else { x1.min(x2) };
        let y = if y1 == y2 { y1 - w / 2 } else { y1.min(y2) };
        let zw = if y1 == y2 {
            (x2 - x1).wrapping_abs()
        } else {
            w
        };
        let zh = if x1 == x2 {
            (y2 - y1).wrapping_abs()
        } else {
            w
        };
        register_nav_zone2(grid, x, y, zw, zh);
    }

    /// `registerNavZonePolyline(linePoints)`. `registerGeometryZone`
    /// clears the list it is given once the zone holds the points.
    fn register_nav_zone_polyline(
        &self,
        grid: &mut IsoMetaGrid,
        line: &mut Vec<i32>,
    ) -> Result<(), String> {
        if line.is_empty() {
            return Ok(());
        }
        let r = grid.register_geometry_zone_typed(
            "",
            NAV_ZONE,
            0,
            ZoneGeometryType::Polyline,
            line,
            None,
            self.width,
        );
        line.clear();
        r.map(|_| ()).map_err(|e| format!("{e:?}"))
    }
}

/// `registerNavZone2(x, y, w, h)`: zones are split into pieces of at most
/// 512 squares.
fn register_nav_zone2(grid: &mut IsoMetaGrid, x: i32, y: i32, w: i32, h: i32) {
    if w > 512 {
        register_nav_zone2(grid, x, y, 512, h);
        register_nav_zone2(grid, x + 512, y, w - 512, h);
    } else if h > 512 {
        register_nav_zone2(grid, x, y, w, 512);
        register_nav_zone2(grid, x, y + 512, w, h - 512);
    } else {
        grid.register_zone("", NAV_ZONE, x, y, 0, w, h);
    }
}

/// `WorldMapStreet.RAILROAD_STRINGS` from `projectzomboid.jar` (the
/// `List.of(...)` its static initialiser wraps in an `ArrayList`).
pub fn railroad_strings_from_jar(jar: &pz_classfile::Jar) -> Result<Vec<String>, String> {
    use pz_classfile::Value;
    const CLASS: &str = "zombie/worldMap/streets/WorldMapStreet";
    let v = pz_classfile::static_value(jar, CLASS, "RAILROAD_STRINGS")
        .map_err(|e| format!("{CLASS}: {e}"))?;
    let list = match v {
        Some(Value::New { args, .. }) => args.into_iter().next(),
        other => other,
    };
    match list {
        Some(Value::Call { name, args, .. }) if name == "of" => args
            .iter()
            .map(|a| a.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
    .ok_or_else(|| format!("{CLASS}.RAILROAD_STRINGS: not a list of strings"))
}

/// `PZMath.tryParseInt(s, default)`.
fn try_parse_int(s: Option<&str>, default: i32) -> i32 {
    let s = crate::java::java_trim(s.unwrap_or(""));
    if s.is_empty() {
        return default;
    }
    s.parse().unwrap_or(default)
}

/// `PZMath.tryParseFloat(s, default)` (`Float.parseFloat` also takes a
/// trailing `f`/`d` type suffix).
fn try_parse_float(s: Option<&str>, default: f32) -> f32 {
    let s = crate::java::java_trim(s.unwrap_or(""));
    if s.is_empty() {
        return default;
    }
    let s = s.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(s);
    if s.contains(['i', 'I', 'n', 'N'])
        && !matches!(s.trim_start_matches(['+', '-']), "Infinity" | "NaN")
    {
        return default;
    }
    s.parse().unwrap_or(default)
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name.as_bytes())
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

/// `WorldMapStreetsXML.read(path)`: the streets of a `streets.xml`
/// document. A document whose root is not `<streets>` has none; one that
/// is not well-formed, has another `version` than 1, an element other than
/// `<street>` under the root, or a street without a name or points fails
/// (`PZXmlParserException`).
pub fn read_streets_xml(text: &str) -> Result<Vec<Street>, String> {
    let mut reader = Reader::from_str(text);
    // Depth of the open element, and what is open at depths 1..3.
    let mut depth = 0usize;
    let mut streets = Vec::new();
    let mut current: Option<Street> = None;
    let mut in_points = false;
    let mut root_is_streets = false;
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| format!("PZXmlParserException: {e}"))?;
        let (start, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            _ => (None, false),
        };
        if let Some(e) = start {
            let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
            match depth {
                0 => {
                    root_is_streets = name == "streets";
                    if root_is_streets && try_parse_int(attr(&e, "version").as_deref(), 0) != 1 {
                        return Err("PZXmlParserException: missing or invalid version".into());
                    }
                }
                1 if root_is_streets => {
                    if name != "street" {
                        return Err(format!(
                            "PZXmlParserException: unrecognised element \"{name}\""
                        ));
                    }
                    current = Some(Street {
                        name: attr(&e, "name").unwrap_or_default(),
                        width: try_parse_int(attr(&e, "width").as_deref(), 5),
                        points: Vec::new(),
                    });
                }
                2 if root_is_streets => in_points = name == "points",
                3 if in_points && name.eq_ignore_ascii_case("point") => {
                    if let Some(s) = current.as_mut() {
                        s.points
                            .push(try_parse_float(attr(&e, "x").as_deref(), 0.0));
                        s.points
                            .push(try_parse_float(attr(&e, "y").as_deref(), 0.0));
                    }
                }
                _ => {}
            }
            if !empty {
                depth += 1;
            } else {
                end_element(depth + 1, &mut current, &mut in_points, &mut streets)?;
            }
            continue;
        }
        match ev {
            Event::End(_) => {
                end_element(depth, &mut current, &mut in_points, &mut streets)?;
                depth -= 1;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(if root_is_streets { streets } else { Vec::new() })
}

/// Closing an element at `depth` (1 = the root).
fn end_element(
    depth: usize,
    current: &mut Option<Street>,
    in_points: &mut bool,
    streets: &mut Vec<Street>,
) -> Result<(), String> {
    match depth {
        2 => {
            if let Some(s) = current.take() {
                if s.name.trim().is_empty() || s.points.is_empty() {
                    return Err("PZXmlParserException: invalid street name or points list".into());
                }
                streets.push(s);
            }
        }
        3 => *in_points = false,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_streets() {
        let xml = r#"<streets version="1">
            <street name="A St" width="8"><points>
                <point x="10.0" y="20.0"/><POINT x="30" y="20"/>
            </points><other/></street>
            <street name="B Rd"><points><point x="1.5"/></points></street>
        </streets>"#;
        let s = read_streets_xml(xml).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "A St");
        assert_eq!(s[0].width, 8);
        assert_eq!(s[0].points, vec![10.0, 20.0, 30.0, 20.0]);
        assert_eq!(s[1].width, 5);
        assert_eq!(s[1].points, vec![1.5, 0.0]);
    }

    #[test]
    fn rejects_bad_documents() {
        assert!(read_streets_xml(r#"<streets version="2"/>"#).is_err());
        assert!(read_streets_xml(r#"<streets version="1"><road/></streets>"#).is_err());
        assert!(read_streets_xml(
            r#"<streets version="1"><street name=" "><points><point/></points></street></streets>"#
        )
        .is_err());
        assert!(read_streets_xml(r#"<streets version="1"><street name="x"/></streets>"#).is_err());
        assert_eq!(
            read_streets_xml(r#"<other version="1"><x/></other>"#).unwrap(),
            vec![]
        );
    }

    #[test]
    fn parses_like_java() {
        assert_eq!(try_parse_float(Some(" 2.5f "), 0.0), 2.5);
        assert_eq!(try_parse_float(Some("abc"), 7.0), 7.0);
        assert_eq!(try_parse_int(Some(" 12 "), 5), 12);
        assert_eq!(try_parse_int(Some("1.0"), 5), 5);
    }
}
