//! `zombie.network.SpawnRegions`: the server's spawn-region and
//! spawn-point Lua files. Running those files is the Lua layer's job; this
//! module parses the tables they return and writes the files back.

use pz_kahlua::{TableRef, Value};
use std::path::Path;

/// `SpawnRegions.Region`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub name: String,
    pub file: Option<String>,
    pub serverfile: Option<String>,
    pub professions: Option<Vec<Profession>>,
}

/// `SpawnRegions.Profession`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profession {
    pub name: String,
    pub points: Vec<Point>,
}

/// `SpawnRegions.Point`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub pos_x: i32,
    pub pos_y: i32,
    pub pos_z: i32,
}

fn get(t: &TableRef, k: &str) -> Value {
    t.rawget(&Value::str(k))
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        _ => None,
    }
}

/// `parseRegionTable`.
pub fn parse_region(t: &TableRef) -> Option<Region> {
    let Value::Str(name) = get(t, "name") else {
        return None;
    };
    if let Value::Str(file) = get(t, "file") {
        return Some(Region {
            name: name.to_string(),
            file: Some(file.to_string()),
            ..Default::default()
        });
    }
    if let Value::Str(sf) = get(t, "serverfile") {
        return Some(Region {
            name: name.to_string(),
            serverfile: Some(sf.to_string()),
            ..Default::default()
        });
    }
    None
}

/// The table `SpawnRegions()` returns, as regions (non-tables and invalid
/// entries are skipped).
pub fn parse_regions(t: &TableRef) -> Vec<Region> {
    t.entries()
        .into_iter()
        .filter_map(|(_, v)| match v {
            Value::Table(r) => parse_region(&r),
            _ => None,
        })
        .collect()
}

/// `parsePointTable`: `worldX/worldY` cells of 300 squares plus `posX/posY`,
/// or absolute `posX/posY`; `posZ` defaults to 0.
pub fn parse_point(t: &TableRef) -> Option<Point> {
    let z = || num(&get(t, "posZ")).map_or(0, |z| z as i32);
    if let (Some(wx), Some(wy), Some(px), Some(py)) = (
        num(&get(t, "worldX")),
        num(&get(t, "worldY")),
        num(&get(t, "posX")),
        num(&get(t, "posY")),
    ) {
        return Some(Point {
            pos_x: wx as i32 * 300 + px as i32,
            pos_y: wy as i32 * 300 + py as i32,
            pos_z: z(),
        });
    }
    if let (Some(px), Some(py)) = (num(&get(t, "posX")), num(&get(t, "posY"))) {
        return Some(Point {
            pos_x: px as i32,
            pos_y: py as i32,
            pos_z: z(),
        });
    }
    None
}

fn parse_points(t: &TableRef) -> Option<Vec<Point>> {
    let v: Vec<Point> = t
        .entries()
        .into_iter()
        .filter_map(|(_, p)| match p {
            Value::Table(p) => parse_point(&p),
            _ => None,
        })
        .collect();
    (!v.is_empty()).then_some(v)
}

/// `parseProfessionsTable`: string keys to point lists (`None` if empty).
pub fn parse_professions(t: &TableRef) -> Option<Vec<Profession>> {
    let v: Vec<Profession> = t
        .entries()
        .into_iter()
        .filter_map(|(k, p)| match (k, p) {
            (Value::Str(name), Value::Table(p)) => parse_points(&p).map(|points| Profession {
                name: name.to_string(),
                points,
            }),
            _ => None,
        })
        .collect();
    (!v.is_empty()).then_some(v)
}

/// `loadPointsTable`: professions back to a `{name = {{posX,posY,posZ}...}}`
/// table.
pub fn points_table(professions: &[Profession]) -> TableRef {
    let out = TableRef::new();
    for p in professions {
        let pts = TableRef::new();
        for (i, pt) in p.points.iter().enumerate() {
            let t = TableRef::new();
            t.rawset(Value::str("posX"), Value::Num(pt.pos_x as f64));
            t.rawset(Value::str("posY"), Value::Num(pt.pos_y as f64));
            t.rawset(Value::str("posZ"), Value::Num(pt.pos_z as f64));
            pts.rawset(Value::Num((i + 1) as f64), Value::Table(t));
        }
        out.rawset(Value::from(p.name.clone()), Value::Table(pts));
    }
    out
}

/// `fmtKey`: a Lua table key, quoted and bracketed when needed.
fn fmt_key(s: &str) -> String {
    let mut s = s.to_string();
    if s.contains('\\') {
        s = s.replace('\\', "\\\\");
    }
    if s.contains('"') {
        s = s.replace('"', "\\\"");
    }
    if s.contains(' ') || s.contains('\\') {
        s = format!("\"{s}\"");
    }
    if s.starts_with('"') {
        return format!("[{s}]");
    }
    s
}

/// `fmtValue`: a quoted Lua string.
fn fmt_value(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The text `saveRegionsFile` writes.
pub fn regions_file_text(regions: &[Region]) -> String {
    let sep = crate::line_separator();
    let mut out = format!("function SpawnRegions(){sep}\treturn {{{sep}");
    for r in regions {
        if let Some(f) = &r.file {
            out.push_str(&format!(
                "\t\t{{ name = {}, file = {} }},{sep}",
                fmt_value(&r.name),
                fmt_value(f)
            ));
            continue;
        }
        if let Some(f) = &r.serverfile {
            out.push_str(&format!(
                "\t\t{{ name = {}, serverfile = {} }},{sep}",
                fmt_value(&r.name),
                fmt_value(f)
            ));
            continue;
        }
        let Some(profs) = &r.professions else {
            continue;
        };
        out.push_str(&format!(
            "\t\t{{ name = {},{sep}\t\t\tpoints = {{{sep}",
            fmt_value(&r.name)
        ));
        for p in profs {
            out.push_str(&format!("\t\t\t\t{} = {{{sep}", fmt_key(&p.name)));
            for pt in &p.points {
                out.push_str(&format!(
                    "\t\t\t\t\t{{ posX = {}, posY = {}, posZ = {} }},{sep}",
                    pt.pos_x, pt.pos_y, pt.pos_z
                ));
            }
            out.push_str(&format!("\t\t\t\t}},{sep}"));
        }
        out.push_str(&format!("\t\t\t}}{sep}\t\t}},{sep}"));
    }
    out.push_str(&format!("\t}}{sep}end{sep}"));
    out
}

/// The text `savePointsFile` writes.
pub fn points_file_text(professions: &[Profession]) -> String {
    let sep = crate::line_separator();
    let mut out = format!("function SpawnPoints(){sep}\treturn {{{sep}");
    for p in professions {
        out.push_str(&format!("\t\t{} = {{{sep}", fmt_key(&p.name)));
        for pt in &p.points {
            out.push_str(&format!(
                "\t\t\t{{ posX = {}, posY = {}, posZ = {} }},{sep}",
                pt.pos_x, pt.pos_y, pt.pos_z
            ));
        }
        out.push_str(&format!("\t\t}},{sep}"));
    }
    out.push_str(&format!("\t}}{sep}end{sep}"));
    out
}

/// `getDefaultServerRegions()`: every `media/maps/<dir>` holding a
/// `spawnpoints.lua`, in directory order.
pub fn default_server_regions(maps_dir: &Path) -> Vec<Region> {
    let Ok(rd) = std::fs::read_dir(maps_dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_dir() && e.path().join("spawnpoints.lua").exists())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Region {
                file: Some(format!("media/maps/{name}/spawnpoints.lua")),
                name,
                ..Default::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_and_keys() {
        let t = TableRef::new();
        for (k, v) in [
            ("worldX", 2.0),
            ("worldY", 1.0),
            ("posX", 5.0),
            ("posY", 7.0),
        ] {
            t.rawset(Value::str(k), Value::Num(v));
        }
        assert_eq!(
            parse_point(&t),
            Some(Point {
                pos_x: 605,
                pos_y: 307,
                pos_z: 0
            })
        );
        assert_eq!(fmt_key("a b"), "[\"a b\"]");
        assert_eq!(fmt_key("plain"), "plain");
        let text = points_file_text(&[Profession {
            name: "x".into(),
            points: vec![Point::default()],
        }]);
        assert!(text.contains("\t\tx = {"));
    }
}
