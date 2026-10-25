//! Replays a ballistics op log recorded from the original native library and
//! compares every result, output array and callback bit for bit.
//!
//! Usage: `cargo run --release --example ballistics_replay -- <ops.bin>...`
//!
//! The same record format as `oracle_replay`, plus the ballistics and
//! ragdoll-definition natives (tags 90..139, listed in `run`).
//!
//! The op logs are produced by a local, uncommitted harness that drives the
//! game's `libPZBullet` through its JNI entry points; no recorded data is
//! kept in this repository.
//!
//! Format: a sequence of records `u8 tag, payload` (all little-endian). Every
//! native call record is followed by a `RESULT` record (`u8 kind`: 0 void,
//! 1 `i32`, 2 `u8` bool, 3 string; then `u8` exception flag with exception
//! class and message strings; then, for calls taking a `float[]`, the whole
//! array as it was after the call). Callbacks the library makes during a
//! call appear between the call and its result: `CB_UPDATE wx wy level`,
//! nested `TOBULLET` calls the Java side made, `CB_RET u8`; and
//! `CB_IMPULSE constraint vidA vidB f32`. Strings and arrays are prefixed
//! with an `i32` length.

use pz_bullet::{JavaCallbacks, JniError, PzBullet};
use std::collections::VecDeque;

const RESULT: u8 = 0xF0;
const CB_UPDATE: u8 = 0xE0;
const CB_RET: u8 = 0xE1;
const CB_IMPULSE: u8 = 0xE2;

struct Reader {
    d: Vec<u8>,
    p: usize,
}

impl Reader {
    fn done(&self) -> bool {
        self.p >= self.d.len()
    }
    fn u8(&mut self) -> u8 {
        let v = self.d[self.p];
        self.p += 1;
        v
    }
    fn i32(&mut self) -> i32 {
        let v = i32::from_le_bytes(self.d[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.i32() as u32)
    }
    fn bytes(&mut self) -> Vec<u8> {
        let n = self.i32() as usize;
        let v = self.d[self.p..self.p + n].to_vec();
        self.p += n;
        v
    }
    fn str(&mut self) -> String {
        String::from_utf8(self.bytes()).unwrap()
    }
    fn arr(&mut self) -> Vec<f32> {
        let n = self.i32() as usize;
        (0..n).map(|_| self.f32()).collect()
    }
    fn iarr(&mut self) -> Vec<i32> {
        let n = self.i32() as usize;
        (0..n).map(|_| self.i32()).collect()
    }
    fn fs<const N: usize>(&mut self) -> [f32; N] {
        let mut a = [0f32; N];
        for v in a.iter_mut() {
            *v = self.f32();
        }
        a
    }
}

/// Value a native call returned.
#[derive(Debug, PartialEq)]
enum Ret {
    Void,
    Int(i32),
    Bool(bool),
    Str(String),
}

/// Recorded callbacks, consumed while the Rust call runs.
#[derive(Debug)]
enum Cb {
    Update {
        wx: i32,
        wy: i32,
        level: i32,
        buffers: Vec<Vec<u8>>,
        ret: bool,
    },
    Impulse {
        c: i32,
        a: i32,
        b: i32,
        imp: u32,
    },
}

struct Expect {
    queue: VecDeque<Cb>,
    mismatches: Vec<String>,
}

impl JavaCallbacks for Expect {
    fn update_physics_for_level_if_needed(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        match self.queue.pop_front() {
            Some(Cb::Update {
                wx: ex,
                wy: ey,
                level: el,
                buffers,
                ret,
            }) if (ex, ey, el) == (wx, wy, level) => {
                to_bullet.extend(buffers);
                ret
            }
            other => {
                self.mismatches.push(format!(
                    "callback updatePhysicsForLevelIfNeeded({wx},{wy},{level}), expected {other:?}"
                ));
                false
            }
        }
    }
    fn on_vehicle_constraint_impulse(&mut self, c: i32, a: i32, b: i32, imp: f32) {
        match self.queue.pop_front() {
            Some(Cb::Impulse {
                c: ec,
                a: ea,
                b: eb,
                imp: ei,
            }) if (ec, ea, eb, ei) == (c, a, b, imp.to_bits()) => {}
            other => self.mismatches.push(format!(
                "callback onVehicleConstraintImpulse({c},{a},{b},{imp}), expected {other:?}"
            )),
        }
    }
}

struct Replay {
    r: Reader,
    bullet: PzBullet,
    calls: u64,
    steps: u64,
    mismatches: u64,
    first: Vec<String>,
    /// Per tag: calls, calls returning a non-zero int, sum of the ints.
    tags: std::collections::BTreeMap<u8, (u64, u64, i64)>,
    last_int: Option<i32>,
    /// Spread rays with a hit, camera targets with a body part.
    hits: (u64, u64),
}

fn ret_of<T: Into<RetV>>(v: Result<T, JniError>) -> (Ret, Option<String>) {
    match v {
        Ok(v) => (v.into().0, None),
        Err(e) => (Ret::Void, Some(e.class_and_message())),
    }
}
struct RetV(Ret);
impl From<()> for RetV {
    fn from(_: ()) -> RetV {
        RetV(Ret::Void)
    }
}
impl From<i32> for RetV {
    fn from(v: i32) -> RetV {
        RetV(Ret::Int(v))
    }
}
impl From<bool> for RetV {
    fn from(v: bool) -> RetV {
        RetV(Ret::Bool(v))
    }
}
impl From<String> for RetV {
    fn from(v: String) -> RetV {
        RetV(Ret::Str(v))
    }
}

impl Replay {
    fn note(&mut self, what: String) {
        self.mismatches += 1;
        if self.first.len() < 20 {
            self.first.push(what);
        }
    }

    /// Reads the callbacks (with their nested ToBullet calls) and the
    /// RESULT record that follow a call.
    fn read_tail(&mut self) -> (VecDeque<Cb>, Ret, Option<String>) {
        let mut q = VecDeque::new();
        loop {
            let t = self.r.u8();
            match t {
                CB_UPDATE => {
                    let (wx, wy, level) = (self.r.i32(), self.r.i32(), self.r.i32());
                    let mut buffers = Vec::new();
                    loop {
                        let t2 = self.r.u8();
                        if t2 == CB_RET {
                            let ret = self.r.u8() != 0;
                            q.push_back(Cb::Update {
                                wx,
                                wy,
                                level,
                                buffers,
                                ret,
                            });
                            break;
                        }
                        assert_eq!(t2, 8, "only ToBullet is called back into");
                        buffers.push(self.r.bytes());
                        let (_, ret, exc) = self.read_tail();
                        assert!(ret == Ret::Void && exc.is_none());
                    }
                }
                CB_IMPULSE => {
                    let (c, a, b, imp) = (
                        self.r.i32(),
                        self.r.i32(),
                        self.r.i32(),
                        self.r.i32() as u32,
                    );
                    q.push_back(Cb::Impulse { c, a, b, imp });
                }
                RESULT => {
                    let ret = match self.r.u8() {
                        0 => Ret::Void,
                        1 => Ret::Int(self.r.i32()),
                        2 => Ret::Bool(self.r.u8() != 0),
                        _ => Ret::Str(self.r.str()),
                    };
                    let exc = if self.r.u8() != 0 {
                        let c = self.r.str();
                        let m = self.r.str();
                        Some(format!("{c}: {m}"))
                    } else {
                        None
                    };
                    return (q, ret, exc);
                }
                _ => panic!("unexpected tag {t:#x} at {}", self.r.p - 1),
            }
        }
    }

    fn check(
        &mut self,
        tag: u8,
        got: (Ret, Option<String>),
        want: (Ret, Option<String>),
        cbs: Expect,
    ) {
        self.calls += 1;
        let e = self.tags.entry(tag).or_insert((0, 0, 0));
        e.0 += 1;
        self.last_int = if let Ret::Int(v) = want.0 {
            Some(v)
        } else {
            None
        };
        if let Ret::Int(v) = want.0 {
            if v != 0 && !(tag == 118 && v == 11) {
                e.1 += 1;
            }
            e.2 += v as i64;
        }
        if got != want {
            let msg = format!(
                "call {} (tag {tag}): got {got:?}, native {want:?}",
                self.calls
            );
            self.note(msg);
        }
        for m in cbs.mismatches {
            self.note(format!("call {} (tag {tag}): {m}", self.calls));
        }
        if !cbs.queue.is_empty() {
            let msg = format!(
                "call {} (tag {tag}): {} callbacks not made: {:?}",
                self.calls,
                cbs.queue.len(),
                cbs.queue
            );
            self.note(msg);
        }
    }

    fn check_arr(&mut self, tag: u8, got: &[f32]) {
        let want = self.r.arr();
        // coverage: spread rays that hit something, camera targets with a body part
        let n = match self.last_int {
            Some(v) if v > 0 => v as usize,
            _ => 0,
        };
        if tag == 105 {
            self.hits.0 += (0..n)
                .filter(|k| want.get(k * 4).is_some_and(|v| *v != 0.0))
                .count() as u64;
        } else if tag == 106 {
            self.hits.1 += (0..n)
                .filter(|k| want.get(k * 5 + 4).is_some_and(|v| *v != 11.0))
                .count() as u64;
        }
        if want.len() != got.len()
            || want
                .iter()
                .zip(got)
                .any(|(a, b)| a.to_bits() != b.to_bits())
        {
            let i = want
                .iter()
                .zip(got)
                .position(|(a, b)| a.to_bits() != b.to_bits())
                .unwrap_or(0);
            let msg = format!(
                "call {} (tag {tag}) array differs at [{i}]: got {:?} native {:?}",
                self.calls,
                got.get(i),
                want.get(i)
            );
            self.note(msg);
        }
    }

    fn run(&mut self) {
        while !self.r.done() {
            let tag = self.r.u8();
            // Arguments, then the call with the recorded callbacks.
            macro_rules! call {
                ($e:expr) => {{
                    let (q, ret, exc) = self.read_tail();
                    let mut cbs = Expect {
                        queue: q,
                        mismatches: Vec::new(),
                    };
                    #[allow(clippy::redundant_closure_call)]
                    let got = ret_of(($e)(&mut self.bullet, &mut cbs));
                    self.check(tag, got, (ret, exc), cbs);
                }};
            }
            macro_rules! call_arr {
                ($a:ident, $e:expr) => {{
                    let (q, ret, exc) = self.read_tail();
                    let mut cbs = Expect {
                        queue: q,
                        mismatches: Vec::new(),
                    };
                    #[allow(clippy::redundant_closure_call)]
                    let got = ret_of(($e)(&mut self.bullet, &mut cbs, &mut $a));
                    self.check(tag, got, (ret, exc), cbs);
                    self.check_arr(tag, &$a);
                }};
            }
            let r = &mut self.r;
            match tag {
                1 => call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                    b.init_pz_bullet();
                    Ok(())
                }),
                2 => {
                    let a = [r.i32(), r.i32(), r.i32(), r.i32(), r.i32(), r.i32()];
                    let s = r.u8() != 0;
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .init_world(a[0], a[1], a[2], a[3], a[4], a[5], s))
                }
                3 => call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                    b.destroy_world();
                    Ok(())
                }),
                4 => {
                    let a = [r.i32(), r.i32(), r.i32(), r.i32()];
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .activate_chunk_map(a[0], a[1], a[2], a[3]))
                }
                5 => {
                    let i = r.i32();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.deactivate_chunk_map(i))
                }
                6 => {
                    let (i, d) = (r.i32(), r.i32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.scroll_chunk_map(i, d))
                }
                7 => {
                    let a = [r.i32(), r.i32(), r.i32(), r.i32()];
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .set_chunk_min_max_level(a[0], a[1], a[2], a[3]))
                }
                8 => {
                    let d = r.bytes();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.to_bullet(&d);
                        Ok(())
                    })
                }
                9 => {
                    let id = r.i32();
                    let f: [f32; 7] = r.fs();
                    let s = r.str();
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .add_vehicle(id, f[0], f[1], f[2], f[3], f[4], f[5], f[6], &s))
                }
                10 => {
                    let id = r.i32();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.remove_vehicle(id))
                }
                11 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.control_vehicle(id, f[0], f[1], f[2]))
                }
                12 => {
                    let (id, v) = (r.i32(), r.u8() != 0);
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_vehicle_active(id, v))
                }
                13 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .apply_central_force_to_vehicle(id, f[0], f[1], f[2]))
                }
                14 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .apply_torque_to_vehicle(id, f[0], f[1], f[2]))
                }
                15 => {
                    let id = r.i32();
                    let f: [f32; 7] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .teleport_vehicle(id, f[0], f[1], f[2], f[3], f[4], f[5], f[6]))
                }
                16 => {
                    let (id, w, f) = (r.i32(), r.i32(), r.f32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_tire_inflation(id, w, f))
                }
                17 => {
                    let (id, w, v) = (r.i32(), r.i32(), r.u8() != 0);
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_tire_removed(id, w, v))
                }
                18 => {
                    let (dt, n, f) = (r.f32(), r.i32(), r.f32());
                    self.steps += 1;
                    call!(|b: &mut PzBullet, cb: &mut Expect| b.step_simulation(dt, n, f, cb))
                }
                19 => call!(|b: &mut PzBullet, _: &mut Expect| b.get_vehicle_count()),
                20 => {
                    let off = r.i32();
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_vehicle_physics(off, a))
                }
                21 => {
                    let id = r.i32();
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_own_vehicle_physics(id, a))
                }
                22 => {
                    let id = r.i32();
                    let mut a = r.arr();
                    let v = r.u8() != 0;
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .set_own_vehicle_physics(id, a, v))
                }
                23 => {
                    let id = r.i32();
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .set_vehicle_params(id, a))
                }
                24 => {
                    let (id, m) = (r.i32(), r.f32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_vehicle_mass(id, m))
                }
                25 => {
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_object_physics(a))
                }
                26 => {
                    let (x, y) = (r.i32(), r.i32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.create_server_cell(x, y))
                }
                27 => {
                    let (x, y) = (r.i32(), r.i32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.remove_server_cell(x, y))
                }
                28 => {
                    let (x, y) = (r.f32(), r.f32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.add_physics_object(x, y))
                }
                29 => {
                    let s = r.str();
                    let a = r.arr();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.define_vehicle_script(&s, &a))
                }
                30 => {
                    let s = r.str();
                    let i = r.i32();
                    let a = r.arr();
                    call!(
                        |b: &mut PzBullet, _: &mut Expect| b.define_vehicle_physics_mesh(&s, i, &a)
                    )
                }
                31 => {
                    let (id, x, y) = (r.i32(), r.f32(), r.f32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .set_vehicle_velocity_multiplier(id, x, y))
                }
                32 => {
                    let (id, v) = (r.i32(), r.u8() != 0);
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_vehicle_static(id, v))
                }
                33 | 34 => {
                    let (a, c) = (r.i32(), r.i32());
                    let f: [f32; 6] = r.fs();
                    if tag == 33 {
                        call!(|b: &mut PzBullet, _: &mut Expect| b
                            .add_hinge_constraint(a, c, f[0], f[1], f[2], f[3], f[4], f[5]))
                    } else {
                        call!(|b: &mut PzBullet, _: &mut Expect| b
                            .add_point_constraint(a, c, f[0], f[1], f[2], f[3], f[4], f[5]))
                    }
                }
                35 => {
                    let (a, c) = (r.i32(), r.i32());
                    let f: [f32; 18] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.add_6dof_constraint(a, c, &f))
                }
                36 => {
                    let (a, c) = (r.i32(), r.i32());
                    let f: [f32; 7] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .add_rope_constraint(a, c, f[0], f[1], f[2], f[3], f[4], f[5], f[6]))
                }
                37 => {
                    let (c, v, axis) = (r.i32(), r.f32(), r.i32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.set_constraint_erp(c, v, axis))
                }
                38 => {
                    let c = r.i32();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.remove_constraint(c))
                }
                39 => call!(|b: &mut PzBullet, _: &mut Expect| b.clear_physics_meshes()),
                40 => {
                    let (i, c) = (r.i32(), r.u8() != 0);
                    let a = r.arr();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.define_physics_mesh(i, c, &a))
                }
                41 => {
                    let (id, a, c) = (r.i32(), r.i32(), r.i32());
                    call!(|b: &mut PzBullet, _: &mut Expect| b.check_wheel_collision(id, a, c))
                }
                // ---- ragdoll definitions (90..99) ----
                90..=93 => {
                    let a = r.arr();
                    let live = r.u8() != 0;
                    call!(|b: &mut PzBullet, _: &mut Expect| match tag {
                        90 => b.define_ragdoll_body_part_info(Some(&a), live),
                        91 => b.define_ragdoll_anchors(Some(&a), live),
                        92 => b.define_ragdoll_constraints(Some(&a), live),
                        _ => b.define_ragdoll_body_dynamics(Some(&a), live),
                    })
                }
                94 => {
                    let n = r.i32();
                    let h = r.iarr();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.initialize_ragdoll_skeleton(n, &h);
                        Ok(())
                    })
                }
                95 => {
                    let n = r.i32();
                    let d = r.arr();
                    let q: [f32; 4] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.initialize_ragdoll_pose(n, &d, q[0], q[1], q[2], q[3]);
                        Ok(())
                    })
                }
                96 => {
                    let m = r.f32();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.set_ragdoll_mass(m);
                        Ok(())
                    })
                }
                97 | 98 => {
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        if tag == 97 {
                            b.set_ballistics_target_adjusting_shape_scale(f[0], f[1], f[2]);
                        } else {
                            b.set_ballistics_target_all_parts_color(f[0], f[1], f[2]);
                        }
                        Ok(())
                    })
                }
                // ---- ballistics (100..139) ----
                100 | 101 | 103 | 109 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        match tag {
                            100 => b.update_ballistics(id, f[0], f[1], f[2]),
                            101 => b.update_ballistics_muzzle_aim_direction(id, f[0], f[1], f[2]),
                            103 => b.set_ballistics_color(id, f[0], f[1], f[2]),
                            _ => b.update_ballistics_aim_reticle_position(id, f[0], f[1], f[2]),
                        }
                        Ok(())
                    })
                }
                102 | 107 => {
                    let (id, v) = (r.i32(), r.f32());
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        if tag == 102 {
                            b.set_ballistics_size(id, v);
                        } else {
                            b.set_ballistics_range(id, v);
                        }
                        Ok(())
                    })
                }
                104 => {
                    let (id, range, count) = (r.i32(), r.f32(), r.i32());
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_ballistics_targets(id, range, count, a))
                }
                105 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    let (n, max) = (r.i32(), r.i32());
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_ballistics_targets_spread_data(
                            id, f[0], f[1], f[2], n, max, a
                        ))
                }
                106 => {
                    let (id, range, count) = (r.i32(), r.f32(), r.i32());
                    let parts = r.u8() != 0;
                    let mut a = r.arr();
                    call_arr!(a, |b: &mut PzBullet, _: &mut Expect, a: &mut Vec<f32>| b
                        .get_ballistics_camera_targets(id, range, count, parts, a))
                }
                108 => {
                    let id = r.i32();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.remove_ballistics(id))
                }
                110..=112 => {
                    let id = r.i32();
                    let f: [f32; 4] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        match tag {
                            110 => {
                                b.update_ballistics_aim_reticle_rotation(id, f[0], f[1], f[2], f[3])
                            }
                            111 => b.update_ballistics_aim_reticle_quaternion(
                                id, f[0], f[1], f[2], f[3],
                            ),
                            _ => b.update_ballistics_aim_reticle_rotate(id, f[0], f[1], f[2], f[3]),
                        }
                        Ok(())
                    })
                }
                113 => {
                    let (id, n) = (r.i32(), r.i32());
                    let d = r.arr();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.update_ballistics_target_skeleton(id, n, &d);
                        Ok(())
                    })
                }
                114 => {
                    let id = r.i32();
                    let f: [f32; 7] = r.fs();
                    let v = r.u8() != 0;
                    call!(|b: &mut PzBullet, _: &mut Expect| b
                        .update_ballistics_target(id, f[0], f[1], f[2], f[3], f[4], f[5], f[6], v))
                }
                115 => {
                    let id = r.i32();
                    let f: [f32; 3] = r.fs();
                    call!(|b: &mut PzBullet, _: &mut Expect| -> Result<(), JniError> {
                        b.set_ballistics_target_axis(id, f[0], f[1], f[2]);
                        Ok(())
                    })
                }
                116 => {
                    let id = r.i32();
                    call!(|b: &mut PzBullet, _: &mut Expect| b.add_ballistics_target(id))
                }
                117 | 118 => {
                    let id = r.i32();
                    call!(
                        |b: &mut PzBullet, _: &mut Expect| -> Result<i32, JniError> {
                            Ok(if tag == 117 {
                                b.remove_ballistics_target(id)
                            } else {
                                b.get_targeted_body_part(id)
                            })
                        }
                    )
                }
                _ => panic!("unknown tag {tag} at {}", self.r.p - 1),
            }
        }
    }
}

fn main() {
    let mut total_calls = 0;
    let mut total_mm = 0;
    let mut total_steps = 0;
    for path in std::env::args().skip(1) {
        let d = std::fs::read(&path).expect("read op log");
        let mut rep = Replay {
            r: Reader { d, p: 0 },
            bullet: PzBullet::new(),
            calls: 0,
            steps: 0,
            mismatches: 0,
            first: Vec::new(),
            tags: Default::default(),
            last_int: None,
            hits: (0, 0),
        };
        // A log cut short (the original crashed mid-call) ends the replay at
        // the last complete call.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let truncated =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rep.run())).is_err();
        std::panic::set_hook(hook);
        let note = if truncated {
            " (log ends early: the original crashed)"
        } else {
            ""
        };
        println!(
            "{path}: {} calls, {} steps, {} mismatches{note}",
            rep.calls, rep.steps, rep.mismatches
        );
        for m in &rep.first {
            println!("  {m}");
        }
        if std::env::var_os("PZB_TAGS").is_some() {
            for (t, (n, nz, sum)) in &rep.tags {
                println!("  tag {t}: {n} calls, {nz} non-zero/non-default int results, sum {sum}");
            }
            println!(
                "  spread rays with a hit: {}, camera targets with a body part: {}",
                rep.hits.0, rep.hits.1
            );
        }
        total_calls += rep.calls;
        total_mm += rep.mismatches;
        total_steps += rep.steps;
    }
    println!("total: {total_calls} calls, {total_steps} steps, {total_mm} mismatches");
    if std::env::var_os("PZB_STATS").is_some() {
        println!("{}", pz_bullet::pz::world::stats::report());
    }
}
