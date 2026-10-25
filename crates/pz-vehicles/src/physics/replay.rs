//! Recorded physics: a text log of `Bullet` calls with their arguments,
//! return values and output arrays, and a [`VehiclePhysics`] that replays
//! such a log (answering every query with the recorded values and noting
//! where the calls made differ from the recorded ones).
//!
//! One call per line:
//!
//! ```text
//! B <name> <arg> <arg> ... [= <ret>] [> <out> <out> ...]
//! ```
//!
//! Arguments are `i<int>`, `f<float bits in hex>`, `z0`/`z1`, `s<string>`
//! (spaces as `\s`, backslashes as `\\`) or `a<float>,<float>,...` for a
//! float array (bits in hex, empty array `a`). Return values use the same
//! tags; outputs are the written prefix of the output array as hex bits.

use super::VehiclePhysics;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One encoded argument.
pub fn enc_i(v: i32) -> String {
    format!("i{v}")
}
pub fn enc_f(v: f32) -> String {
    format!("f{:x}", v.to_bits())
}
pub fn enc_z(v: bool) -> String {
    if v {
        "z1".into()
    } else {
        "z0".into()
    }
}
pub fn enc_s(v: &str) -> String {
    format!("s{}", v.replace('\\', "\\\\").replace(' ', "\\s"))
}
pub fn enc_a(v: &[f32]) -> String {
    let mut s = String::from("a");
    for (i, f) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{:x}", f.to_bits()));
    }
    s
}

/// A recorded call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Call {
    pub name: String,
    pub args: Vec<String>,
    pub ret: Option<String>,
    pub out: Vec<f32>,
}

impl Call {
    /// Parses a `B ...` line.
    pub fn parse(line: &str) -> Option<Call> {
        let rest = line.strip_prefix("B ")?;
        let (head, out) = match rest.split_once(" > ") {
            Some((h, o)) => (h, Some(o)),
            None => (rest, None),
        };
        let (head, ret) = match head.split_once(" = ") {
            Some((h, r)) => (h, Some(r.to_string())),
            None => (head, None),
        };
        let mut it = head.split(' ');
        let name = it.next()?.to_string();
        let args = it.filter(|s| !s.is_empty()).map(str::to_string).collect();
        let out = out
            .map(|o| {
                o.split(' ')
                    .filter(|s| !s.is_empty())
                    .filter_map(|h| u32::from_str_radix(h, 16).ok())
                    .map(f32::from_bits)
                    .collect()
            })
            .unwrap_or_default();
        Some(Call {
            name,
            args,
            ret,
            out,
        })
    }

    /// The `B ...` line (without the output part when `out` is empty).
    pub fn line(&self) -> String {
        let mut s = format!("B {}", self.name);
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        if let Some(r) = &self.ret {
            s.push_str(" = ");
            s.push_str(r);
        }
        if !self.out.is_empty() {
            s.push_str(" >");
            for f in &self.out {
                s.push_str(&format!(" {:x}", f.to_bits()));
            }
        }
        s
    }

    fn ret_i(&self) -> i32 {
        self.ret
            .as_deref()
            .and_then(|r| r.strip_prefix('i'))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}

/// Replays a recorded call log. Clones share the replay state, so a copy
/// kept by the caller sees the calls made through the installed one.
#[derive(Clone, Default)]
pub struct ReplayPhysics {
    calls: Rc<Vec<Call>>,
    next: Rc<Cell<usize>>,
    /// Calls that differed from the recording: `(index, expected, made)`.
    mismatches: Rc<RefCell<Vec<(usize, String, String)>>>,
    /// Every call made, encoded.
    made: Rc<RefCell<Vec<String>>>,
}

impl ReplayPhysics {
    pub fn new(calls: Vec<Call>) -> Self {
        ReplayPhysics {
            calls: Rc::new(calls),
            ..Default::default()
        }
    }

    /// Parses the `B ` lines of a log.
    pub fn from_log(text: &str) -> Self {
        Self::new(text.lines().filter_map(Call::parse).collect())
    }

    /// Calls left in the recording.
    pub fn remaining(&self) -> usize {
        self.calls.len() - self.next.get()
    }

    /// Number of calls consumed so far.
    pub fn position(&self) -> usize {
        self.next.get()
    }

    /// The calls that differed: `(index, expected, made)`.
    pub fn mismatches(&self) -> Vec<(usize, String, String)> {
        self.mismatches.borrow().clone()
    }

    /// Every call made, encoded.
    pub fn made(&self) -> Vec<String> {
        self.made.borrow().clone()
    }

    fn take(&mut self, name: &str, args: Vec<String>) -> Call {
        let made = Call {
            name: name.to_string(),
            args,
            ret: None,
            out: Vec::new(),
        };
        let made_line = made.line();
        self.made.borrow_mut().push(made_line.clone());
        let idx = self.next.get();
        let Some(expected) = self.calls.get(idx).cloned() else {
            self.mismatches
                .borrow_mut()
                .push((idx, "<end of recording>".into(), made_line));
            return made;
        };
        self.next.set(idx + 1);
        if expected.name != made.name || expected.args != made.args {
            let exp = Call {
                ret: None,
                out: Vec::new(),
                ..expected.clone()
            };
            self.mismatches
                .borrow_mut()
                .push((idx, exp.line(), made_line));
            if expected.name != made.name {
                // Answer with nothing rather than another call's values.
                return made;
            }
        }
        expected
    }
}

impl VehiclePhysics for ReplayPhysics {
    fn init_world(&mut self, a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, s: bool) {
        self.take(
            "initWorld",
            vec![
                enc_i(a),
                enc_i(b),
                enc_i(c),
                enc_i(d),
                enc_i(e),
                enc_i(f),
                enc_z(s),
            ],
        );
    }
    fn destroy_world(&mut self) {
        self.take("destroyWorld", vec![]);
    }
    fn activate_chunk_map(&mut self, p: i32, wx: i32, wy: i32, w: i32) {
        self.take(
            "activateChunkMap",
            vec![enc_i(p), enc_i(wx), enc_i(wy), enc_i(w)],
        );
    }
    fn deactivate_chunk_map(&mut self, p: i32) {
        self.take("deactivateChunkMap", vec![enc_i(p)]);
    }
    fn scroll_chunk_map(&mut self, p: i32, dir: i32) {
        self.take("scrollChunkMap", vec![enc_i(p), enc_i(dir)]);
    }
    fn define_vehicle_script(&mut self, name: &str, params: &[f32]) {
        self.take("defineVehicleScript", vec![enc_s(name), enc_a(params)]);
    }
    fn define_vehicle_physics_mesh(&mut self, name: &str, i: i32, pts: &[f32]) {
        self.take(
            "defineVehiclePhysicsMesh",
            vec![enc_s(name), enc_i(i), enc_a(pts)],
        );
    }
    fn add_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        script: &str,
    ) {
        self.take(
            "addVehicle",
            vec![
                enc_i(id),
                enc_f(x),
                enc_f(y),
                enc_f(z),
                enc_f(qx),
                enc_f(qy),
                enc_f(qz),
                enc_f(qw),
                enc_s(script),
            ],
        );
    }
    fn remove_vehicle(&mut self, id: i32) {
        self.take("removeVehicle", vec![enc_i(id)]);
    }
    fn control_vehicle(&mut self, id: i32, e: f32, b: f32, s: f32) {
        self.take(
            "controlVehicle",
            vec![enc_i(id), enc_f(e), enc_f(b), enc_f(s)],
        );
    }
    fn set_vehicle_active(&mut self, id: i32, a: bool) {
        self.take("setVehicleActive", vec![enc_i(id), enc_z(a)]);
    }
    fn set_vehicle_static(&mut self, id: i32, s: bool) -> i32 {
        self.take("setVehicleStatic", vec![enc_i(id), enc_z(s)])
            .ret_i()
    }
    fn apply_central_force_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32) {
        self.take(
            "applyCentralForceToVehicle",
            vec![enc_i(id), enc_f(x), enc_f(y), enc_f(z)],
        );
    }
    fn apply_torque_to_vehicle(&mut self, id: i32, x: f32, y: f32, z: f32) {
        self.take(
            "applyTorqueToVehicle",
            vec![enc_i(id), enc_f(x), enc_f(y), enc_f(z)],
        );
    }
    fn teleport_vehicle(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) {
        self.take(
            "teleportVehicle",
            vec![
                enc_i(id),
                enc_f(x),
                enc_f(y),
                enc_f(z),
                enc_f(qx),
                enc_f(qy),
                enc_f(qz),
                enc_f(qw),
            ],
        );
    }
    fn set_tire_inflation(&mut self, id: i32, w: i32, v: f32) {
        self.take("setTireInflation", vec![enc_i(id), enc_i(w), enc_f(v)]);
    }
    fn set_tire_removed(&mut self, id: i32, w: i32, r: bool) {
        self.take("setTireRemoved", vec![enc_i(id), enc_i(w), enc_z(r)]);
    }
    fn step_simulation(&mut self, t: f32, n: i32, f: f32) {
        self.take("stepSimulation", vec![enc_f(t), enc_i(n), enc_f(f)]);
    }
    fn get_vehicle_count(&mut self) -> i32 {
        self.take("getVehicleCount", vec![]).ret_i()
    }
    fn get_vehicle_physics(&mut self, offset: i32, out: &mut [f32]) -> i32 {
        let c = self.take("getVehiclePhysics", vec![enc_i(offset)]);
        let n = c.out.len().min(out.len());
        out[..n].copy_from_slice(&c.out[..n]);
        c.ret_i()
    }
    fn get_own_vehicle_physics(&mut self, id: i32, out: &mut [f32]) -> i32 {
        let c = self.take("getOwnVehiclePhysics", vec![enc_i(id)]);
        let n = c.out.len().min(out.len());
        out[..n].copy_from_slice(&c.out[..n]);
        c.ret_i()
    }
    fn set_own_vehicle_physics(&mut self, id: i32, p: &[f32], force: bool) -> i32 {
        self.take(
            "setOwnVehiclePhysics",
            vec![enc_i(id), enc_a(p), enc_z(force)],
        )
        .ret_i()
    }
    fn set_vehicle_params(&mut self, id: i32, p: &[f32]) -> i32 {
        self.take("setVehicleParams", vec![enc_i(id), enc_a(p)])
            .ret_i()
    }
    fn set_vehicle_mass(&mut self, id: i32, m: f32) -> i32 {
        self.take("setVehicleMass", vec![enc_i(id), enc_f(m)])
            .ret_i()
    }
    fn get_object_physics(&mut self, out: &mut [f32]) -> i32 {
        let c = self.take("getObjectPhysics", vec![]);
        let n = c.out.len().min(out.len());
        out[..n].copy_from_slice(&c.out[..n]);
        c.ret_i()
    }
    fn set_vehicle_velocity_multiplier(&mut self, id: i32, max: f32, mul: f32) {
        self.take(
            "setVehicleVelocityMultiplier",
            vec![enc_i(id), enc_f(max), enc_f(mul)],
        );
    }
    fn add_6dof_constraint(&mut self, a: i32, b: i32, p: [f32; 18]) -> i32 {
        let mut args = vec![enc_i(a), enc_i(b)];
        args.extend(p.iter().map(|f| enc_f(*f)));
        self.take("add6DofConstraint", args).ret_i()
    }
    fn add_rope_constraint(
        &mut self,
        a: i32,
        b: i32,
        ax: f32,
        ay: f32,
        az: f32,
        bx: f32,
        by: f32,
        bz: f32,
        d: f32,
    ) -> i32 {
        self.take(
            "addRopeConstraint",
            vec![
                enc_i(a),
                enc_i(b),
                enc_f(ax),
                enc_f(ay),
                enc_f(az),
                enc_f(bx),
                enc_f(by),
                enc_f(bz),
                enc_f(d),
            ],
        )
        .ret_i()
    }
    fn set_constraint_erp(&mut self, c: i32, erp: f32, axis: i32) {
        self.take("setConstraintERP", vec![enc_i(c), enc_f(erp), enc_i(axis)]);
    }
    fn remove_constraint(&mut self, c: i32) {
        self.take("removeConstraint", vec![enc_i(c)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip() {
        let line = "B getVehiclePhysics i0 = i1 > 3f800000 40000000";
        let c = Call::parse(line).unwrap();
        assert_eq!(c.name, "getVehiclePhysics");
        assert_eq!(c.args, vec!["i0"]);
        assert_eq!(c.out, vec![1.0, 2.0]);
        assert_eq!(c.line(), line);
        assert_eq!(enc_s("a b\\c"), "sa\\sb\\\\c");
    }

    #[test]
    fn replay_answers_and_flags_mismatches() {
        let mut p =
            ReplayPhysics::from_log("B getVehicleCount = i2\nB controlVehicle i1 f0 f0 f0\n");
        assert_eq!(p.get_vehicle_count(), 2);
        p.control_vehicle(1, 1.0, 0.0, 0.0);
        assert_eq!(p.mismatches().len(), 1);
        assert_eq!(p.remaining(), 0);
    }
}
