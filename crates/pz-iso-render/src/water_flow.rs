//! `IsoWaterFlow`: the flow points and shore zones the water shader's flow
//! and the shore detection read.
//!
//! The data comes from the maps' `media/maps/<dir>/objects.lua`, as the
//! game loads it: `metazoneHandler.lua`'s `doMapZones` runs each lot
//! directory's file (last directory first) and passes every `WaterFlow`
//! and `WaterZone` entry to `IsoWorld.registerWaterFlow` /
//! `registerWaterZone`. [`IsoWaterFlow::load_objects_lua`] runs the file
//! through Kahlua and applies `handleWaterFlow` / `handleWaterZone` to its
//! entries; [`IsoWaterFlow::load_map_dirs`] does it for the lot
//! directories. Nothing is extracted ahead of time.

use std::rc::Rc;

use pz_kahlua::{LStr, TableRef, Thread, Value};

/// `WaterFlowObject`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterFlowObject {
    pub x: i32,
    pub y: i32,
    pub radians: f32,
    pub speed: f32,
}

impl WaterFlowObject {
    /// `distSq`: `IsoUtils.DistanceToSquared` (doubles, rounded to float).
    fn dist_sq(&self, x: i32, y: i32) -> f32 {
        ((x as f64 - self.x as f64).powi(2) + (y as f64 - self.y as f64).powi(2)) as f32
    }
}

/// A `WaterZone`: `[x1, y1, x2, y2]` and its shore and ground values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterZone {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub shore: f32,
    pub water_ground: f32,
}

/// A kd-tree node (`WaterFlowObject.kdLeft/kdRight`).
#[derive(Clone, Copy, Debug)]
struct Node {
    point: WaterFlowObject,
    left: Option<usize>,
    right: Option<usize>,
}

/// `IsoWaterFlow`'s points (`POINTS` and the `WaterFlowLookup` kd-tree)
/// and zones.
#[derive(Clone, Debug, Default)]
pub struct IsoWaterFlow {
    nodes: Vec<Node>,
    root: Option<usize>,
    pub zones: Vec<WaterZone>,
}

/// `FastTrig360`.
mod fast_trig {
    use std::sync::OnceLock;

    fn table() -> &'static [f32; 360] {
        static T: OnceLock<[f32; 360]> = OnceLock::new();
        T.get_or_init(|| std::array::from_fn(|i| ((i as f32 * (std::f32::consts::PI / 180.0)) as f64).sin() as f32))
    }

    fn frac(v: f32) -> f32 {
        v - v.floor()
    }

    /// `PZMath.wrap(val, range)`.
    fn wrap_range(val: f32, range: f32) -> f32 {
        if range <= 0.0 {
            return 0.0;
        }
        if val < 0.0 {
            return (1.0 - frac(-val / range)) * range;
        }
        frac(val / range) * range
    }

    /// `PZMath.wrap(val, min, max)`.
    pub fn wrap(val: f32, min: f32, max: f32) -> f32 {
        let (hi, lo) = (max.max(min), max.min(min));
        lo + wrap_range(val - lo, hi - lo)
    }

    pub fn sin(rad: f32) -> f32 {
        let degf = wrap(rad * 57.295_776, 0.0, 360.0);
        let deg = pz_render::iso::fastfloor(degf);
        table()[deg.rem_euclid(360) as usize % 360]
    }

    pub fn cos(rad: f32) -> f32 {
        sin(rad + 1.570_796_4)
    }

    pub fn approximate_atan2(y: f32, x: f32) -> f32 {
        const PI: f32 = std::f32::consts::PI;
        if x == 0.0 {
            return if y > 0.0 {
                1.570_796_4
            } else if y < 0.0 {
                -1.570_796_4
            } else {
                0.0
            };
        }
        let z = y / x;
        let atan;
        if z.abs() < 1.0 {
            atan = z / (1.0 + 0.28086 * z * z);
            if x < 0.0 {
                return if y < 0.0 { atan - PI } else { atan + PI };
            }
        } else {
            atan = 1.570_796_4 - z / (z * z + 0.28086);
            if y < 0.0 {
                return atan - PI;
            }
        }
        atan
    }
}

fn trigonometric_circle(angle1: f32, weight1: f32, angle2: f32, weight2: f32) -> f32 {
    let x = weight1 * fast_trig::cos(angle1) + weight2 * fast_trig::cos(angle2);
    let y = weight1 * fast_trig::sin(angle1) + weight2 * fast_trig::sin(angle2);
    fast_trig::wrap(fast_trig::approximate_atan2(y, x), 0.0, std::f32::consts::PI * 2.0)
}

/// `blendDistFlowSpeed`: (flow, speed, distance).
fn blend(dist_a: f32, rad_a: f32, speed_a: f32, dist_b: f32, rad_b: f32, speed_b: f32) -> (f32, f32, f32) {
    let a = dist_a.max(0.1);
    let b = dist_b.max(0.1);
    let dist = a + b;
    let (wa, wb) = (1.0 - a / dist, 1.0 - b / dist);
    (trigonometric_circle(rad_a, wa, rad_b, wb), speed_a * wa + speed_b * wb, a * wa + b * wb)
}

impl IsoWaterFlow {
    /// `addFlow(x, y, flow, speed)`: `flow` in degrees, turned into the
    /// shader's angle.
    pub fn add_flow(&mut self, x: f32, y: f32, flow: f32, speed: f32) {
        let mut degrees = (360 - flow as i32 - 45) % 360;
        if degrees < 0 {
            degrees += 360;
        }
        let point = WaterFlowObject { x: x as i32, y: y as i32, radians: (degrees as f64).to_radians() as f32, speed };
        self.nodes.push(Node { point, left: None, right: None });
        let n = self.nodes.len() - 1;
        // WaterFlowLookup.insert.
        let Some(mut at) = self.root else {
            self.root = Some(n);
            return;
        };
        let mut compare_x = true;
        loop {
            let node = self.nodes[at];
            let left = if compare_x { point.x < node.point.x } else { point.y < node.point.y };
            let child = if left { node.left } else { node.right };
            match child {
                Some(c) => at = c,
                None => {
                    if left {
                        self.nodes[at].left = Some(n);
                    } else {
                        self.nodes[at].right = Some(n);
                    }
                    return;
                }
            }
            compare_x = !compare_x;
        }
    }

    /// `addZone(x1, y1, x2, y2, shore, waterGround)`.
    pub fn add_zone(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, shore: f32, water_ground: f32) {
        self.zones.push(WaterZone { x1, y1, x2, y2, shore, water_ground });
    }

    /// `getShore(x, y)`: the first zone holding the square, else 1.
    pub fn get_shore(&self, x: i32, y: i32) -> i32 {
        let (x, y) = (x as f32, y as f32);
        self.zones.iter().find(|z| z.x1 <= x && z.x2 >= x && z.y1 <= y && z.y2 >= y).map_or(1, |z| z.shore as i32)
    }

    pub fn points(&self) -> impl Iterator<Item = &WaterFlowObject> {
        self.nodes.iter().map(|n| &n.point)
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        *self = IsoWaterFlow::default();
    }

    /// `WaterFlowLookup.nearestMultiple`: up to `max` points by distance,
    /// nearest first.
    pub fn nearest_multiple(&self, x: i32, y: i32, max: usize) -> Vec<WaterFlowObject> {
        let mut found: Vec<(f32, WaterFlowObject)> = Vec::new();
        self.nearest_rec(self.root, x, y, max, true, &mut found);
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, p)| p).collect()
    }

    fn nearest_rec(&self, current: Option<usize>, x: i32, y: i32, max: usize, check_x: bool, found: &mut Vec<(f32, WaterFlowObject)>) {
        let Some(c) = current else { return };
        let node = self.nodes[c];
        let d = node.point.dist_sq(x, y);
        // The queue keeps the `max` nearest; its head is the farthest.
        let worst = |f: &Vec<(f32, WaterFlowObject)>| f.iter().enumerate().max_by(|a, b| a.1 .0.total_cmp(&b.1 .0)).map(|(i, v)| (i, v.0));
        if found.len() < max {
            found.push((d, node.point));
        } else if let Some((i, w)) = worst(found) {
            if (d as f64) < w as f64 {
                found.remove(i);
                found.push((d, node.point));
            }
        }
        let plane = if check_x { (x - node.point.x) as f64 } else { (y - node.point.y) as f64 };
        let (primary, secondary) = if plane < 0.0 { (node.left, node.right) } else { (node.right, node.left) };
        self.nearest_rec(primary, x, y, max, !check_x, found);
        if found.len() < max || worst(found).is_some_and(|(_, w)| plane * plane < w as f64) {
            self.nearest_rec(secondary, x, y, max, !check_x, found);
        }
    }

    /// `getFlow(square, ax, ay)`: (flow, speed) at the corner `(x + ax,
    /// y + ay)`, the speed slowed within four squares of land.
    /// `water_at(x, y)` is whether the level-0 square there has the
    /// `water` flag (false when missing).
    pub fn get_flow(&self, x: i32, y: i32, ax: i32, ay: i32, water_at: impl Fn(i32, i32) -> bool) -> (f32, f32) {
        if self.nodes.is_empty() {
            return (0.0, 0.0);
        }
        let (qx, qy) = (x + ax, y + ay);
        let near = self.nearest_multiple(qx, qy, 3);
        let a = near[0];
        let dist_a = a.dist_sq(qx, qy).sqrt();
        let (mut flow, mut speed) = (a.radians, a.speed);
        if near.len() > 1 {
            let b = near[1];
            let dist_b = b.dist_sq(qx, qy).sqrt();
            if dist_a * 10.0 >= dist_b {
                let r = if near.len() > 2 {
                    let c = near[2];
                    let dist_c = c.dist_sq(qx, qy).sqrt();
                    if dist_b * 10.0 >= dist_c {
                        let bc = blend(dist_b, b.radians, b.speed, dist_c, c.radians, c.speed);
                        blend(dist_a, a.radians, a.speed, bc.2, bc.0, bc.1)
                    } else {
                        blend(dist_a, a.radians, a.speed, dist_b, b.radians, b.speed)
                    }
                } else {
                    blend(dist_a, a.radians, a.speed, dist_b, b.radians, b.speed)
                };
                flow = r.0;
                speed = r.1;
            }
        }
        let mut s = 1.0f32;
        for dx in -5..5 {
            for dy in -5..5 {
                if water_at(qx + dx, qy + dy) {
                    continue;
                }
                s = (s as f64).min(((dx * dx + dy * dy) as f64).sqrt().max(0.0) / 4.0) as f32;
            }
        }
        (flow, speed * s)
    }

    /// The flows `IsoWaterGeometry.initRender` reads for a square's four
    /// corners, for [`crate::water::WaterInit::flow`].
    pub fn corner_flows(&self, x: i32, y: i32, water_at: impl Fn(i32, i32) -> bool) -> [(f32, f32); 4] {
        [(0, 0), (0, 1), (1, 1), (1, 0)].map(|(ax, ay)| self.get_flow(x, y, ax, ay, &water_at))
    }

    /// Runs one `objects.lua` (as `doMapZones` does: `objects = {}`, then
    /// the file) and registers its `WaterFlow` and `WaterZone` entries.
    /// Returns the problems `handleWaterFlow`/`handleWaterZone` print.
    pub fn load_objects_lua(&mut self, source: &str, filename: &str) -> Result<Vec<String>, String> {
        let env = TableRef::new();
        env.set_str("objects", Value::Table(TableRef::new()));
        let mut thread = Thread::new(env.clone());
        let base = filename.rsplit('/').next().unwrap_or(filename);
        let proto = pz_kahlua::compiler::compile(source, Some(base), Some(base), Some(filename)).map_err(|e| format!("{filename}: {e}"))?;
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), env.clone()));
        thread.pcall(&f, &[]).map_err(|(msg, trace)| format!("{filename}: {}\n{trace}", pz_kahlua::exec::java_value_of(&msg)))?;
        let Value::Table(objects) = env.get_str("objects") else { return Ok(Vec::new()) };
        let mut problems = Vec::new();
        // ipairs(objects).
        let mut i = 1;
        loop {
            let Value::Table(v) = objects.get_int(i) else { break };
            i += 1;
            match str_of(&v.get_str("type")).as_deref() {
                Some("WaterFlow") => self.handle_water_flow(filename, &v, &mut problems),
                Some("WaterZone") => self.handle_water_zone(filename, &v, &mut problems),
                _ => {}
            }
        }
        Ok(problems)
    }

    fn handle_water_flow(&mut self, file: &str, v: &TableRef, problems: &mut Vec<String>) {
        let at = || format!("{file} at {},{},{}", num_str(&v.get_str("x")), num_str(&v.get_str("y")), num_str(&v.get_str("z")));
        let Value::Table(p) = v.get_str("properties") else {
            problems.push(format!("ERROR: WaterFlow missing properties in {}", at()));
            return;
        };
        let (dir, speed) = (p.get_str("WaterDirection"), p.get_str("WaterSpeed"));
        if dir.is_nil() {
            problems.push(format!("ERROR: WaterFlow missing properties.WaterDirection in {}", at()));
            return;
        }
        if speed.is_nil() {
            problems.push(format!("ERROR: WaterFlow missing properties.WaterSpeed in {}", at()));
            return;
        }
        // registerWaterFlow(float x, float y, float flow, float speed).
        let n = |v: &Value| pz_kahlua::exec::raw_tonumber(v).map(|d| d as f32);
        match (n(&v.get_str("x")), n(&v.get_str("y")), n(&dir), n(&speed)) {
            (Some(x), Some(y), Some(d), Some(s)) => self.add_flow(x, y, d, s),
            _ => problems.push(format!("ERROR: WaterFlow with non-numeric values in {}", at())),
        }
    }

    fn handle_water_zone(&mut self, file: &str, v: &TableRef, problems: &mut Vec<String>) {
        let at = || format!("{file} at {},{},{}", num_str(&v.get_str("x")), num_str(&v.get_str("y")), num_str(&v.get_str("z")));
        let Value::Table(p) = v.get_str("properties") else {
            problems.push(format!("ERROR: WaterZone missing properties in {}", at()));
            return;
        };
        let (ground, shore) = (p.get_str("WaterGround"), p.get_str("WaterShore"));
        if ground.is_nil() {
            problems.push(format!("ERROR: WaterZone missing properties.WaterGround in {}", at()));
            return;
        }
        if shore.is_nil() {
            problems.push(format!("ERROR: WaterZone missing properties.WaterShore in {}", at()));
            return;
        }
        let n = |v: &Value| pz_kahlua::exec::raw_tonumber(v);
        let (Some(x), Some(y), Some(w), Some(h)) = (n(&v.get_str("x")), n(&v.get_str("y")), n(&v.get_str("width")), n(&v.get_str("height"))) else {
            problems.push(format!("ERROR: WaterZone with non-numeric bounds in {}", at()));
            return;
        };
        // `WaterShore and 1.0 or 0.0`: Lua truthiness.
        let truthy = |v: &Value| !matches!(v, Value::Nil | Value::Bool(false));
        self.add_zone(x as f32, y as f32, (x + w) as f32, (y + h) as f32, if truthy(&shore) { 1.0 } else { 0.0 }, if truthy(&ground) { 1.0 } else { 0.0 });
    }

    /// `doMapZones`' water part: `media/maps/<dir>/objects.lua` for each
    /// lot directory, last first. `read(path)` reads a game file through
    /// the file system (mods included), `None` when it does not exist.
    pub fn load_map_dirs(&mut self, lot_dirs: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<String> {
        let mut problems = Vec::new();
        for dir in lot_dirs.iter().rev() {
            let file = format!("media/maps/{dir}/objects.lua");
            let Some(src) = read(&file) else { continue };
            match self.load_objects_lua(&src, &file) {
                Ok(p) => problems.extend(p),
                Err(e) => problems.push(e),
            }
        }
        problems
    }
}

fn str_of(v: &Value) -> Option<LStr> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn num_str(v: &Value) -> String {
    pz_kahlua::exec::java_value_of(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECTS: &str = r#"
objects = {
  { name = "", type = "Nav", x = 1, y = 2, z = 0, width = 8, height = 234 },
  { name = "river", type = "WaterFlow", x = 100, y = 100, z = 0, width = 1, height = 1, properties = { WaterDirection = 90, WaterSpeed = 0.7 } },
  { name = "river", type = "WaterFlow", x = 110, y = 100, z = 0, width = 1, height = 1, properties = { WaterDirection = 0, WaterSpeed = 0.5 } },
  { name = "bad", type = "WaterFlow", x = 1, y = 1, z = 0, width = 1, height = 1, properties = { WaterSpeed = 0.5 } },
  { name = "", type = "WaterZone", x = 90, y = 90, z = 0, width = 30, height = 20, properties = { WaterGround = false, WaterShore = false } },
}
"#;

    #[test]
    fn objects_lua_registers_flows_and_zones() {
        let mut f = IsoWaterFlow::default();
        let problems = f.load_objects_lua(OBJECTS, "media/maps/Test/objects.lua").unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("WaterDirection"));
        let pts: Vec<_> = f.points().copied().collect();
        assert_eq!(pts.len(), 2);
        // 360 - 90 - 45 = 225 degrees.
        assert!((pts[0].radians - 225f32.to_radians()).abs() < 1e-6);
        // 360 - 0 - 45 = 315.
        assert!((pts[1].radians - 315f32.to_radians()).abs() < 1e-6);
        assert_eq!(f.get_shore(100, 100), 0);
        assert_eq!(f.get_shore(10, 10), 1);
        // Next to the first point: mostly its flow, full speed in open
        // water.
        let (flow, speed) = f.get_flow(101, 100, 0, 0, |_, _| true);
        assert!((speed - (0.7 * (1.0 - 1.0 / 10.0) + 0.5 * (1.0 - 9.0 / 10.0))).abs() < 1e-3, "{speed}");
        assert!(flow > 3.5 && flow < 4.5, "{flow}");
        // Next to land the speed drops (land one square away: 1/4).
        let (_, slow) = f.get_flow(101, 100, 0, 0, |x, _| x != 102);
        assert!((slow - speed * 0.25).abs() < 1e-5, "{slow}");
    }

    #[test]
    fn nearest_multiple_orders_by_distance() {
        let mut f = IsoWaterFlow::default();
        for (x, y) in [(0, 0), (10, 0), (5, 5), (20, 20), (1, 1)] {
            f.add_flow(x as f32, y as f32, 0.0, 1.0);
        }
        let n = f.nearest_multiple(2, 2, 3);
        assert_eq!(n.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), vec![(1, 1), (0, 0), (5, 5)]);
        assert!((fast_trig::approximate_atan2(1.0, 1.0) - std::f32::consts::FRAC_PI_4).abs() < 0.01);
    }
}

/// Loads the real maps when `PZ_GAME_DIR` points at a game install
/// (`cargo test -- --ignored`); nothing is read otherwise.
#[cfg(test)]
#[test]
#[ignore]
fn loads_the_game_maps() {
    let Ok(dir) = std::env::var("PZ_GAME_DIR") else { return };
    let maps = std::path::Path::new(&dir).join("media/maps");
    let dirs: Vec<String> = std::fs::read_dir(&maps).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let mut f = IsoWaterFlow::default();
    let problems = f.load_map_dirs(&dirs, |p| std::fs::read(std::path::Path::new(&dir).join(p)).ok().map(|b| String::from_utf8_lossy(&b).into_owned()));
    let n = f.points().count();
    eprintln!("{n} flow points, {} zones, {} problems", f.zones.len(), problems.len());
    assert!(n > 0);
}
