//! Replays a ragdoll op log recorded from the original native library and
//! compares every result, output array and callback bit for bit.
//!
//! Usage: `cargo run --release --example ragdoll_replay -- <ops.bin>...`
//!
//! The format is `oracle_replay`'s, with the ragdoll natives as tags 50..79
//! (arguments in the native's order; every `float[]` argument is followed
//! in the `RESULT` record by its contents after the call, a length of -1
//! being `null`). Tag 79 is `initPZBullet` with its
//! `SkeletonBone::ValidateAgainstJava` callbacks: after the result, `u8`
//! validation result (1 passed, 0 failed, 2 not logged) and the Java
//! answers: `i32 n`, then per bone its name and ordinal.
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

use pz_bullet::{JavaCallbacks, JniError, PzBullet, SkeletonBoneCallbacks};
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
    fn oarr(&mut self) -> Option<Vec<f32>> {
        let n = self.i32();
        if n < 0 {
            return None;
        }
        Some((0..n).map(|_| self.f32()).collect())
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

/// Bitwise float equality; with `PZB_NAN_EQ` set, any two NaNs are equal
/// (a diagnostic for NaN sign and payload differences).
fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits()
        || (a.is_nan() && b.is_nan() && std::env::var_os("PZB_NAN_EQ").is_some())
}

/// The Java answers to `SkeletonBone::ValidateAgainstJava`.
struct Bones(Vec<(String, i32)>);
impl SkeletonBoneCallbacks for Bones {
    fn get_bone_name(&mut self, ordinal: i32) -> Option<String> {
        self.0.get(ordinal as usize).map(|b| b.0.clone())
    }
    fn get_bone_ordinal(&mut self, name: &str) -> i32 {
        self.0
            .iter()
            .find(|b| b.0 == name)
            .map(|b| b.1)
            .unwrap_or(-1)
    }
}

/// Coverage counters (`PZB_RSTATS`): what the scenarios exercised.
#[derive(Default, Debug)]
struct Stats {
    wheel_hits: u64,
    wheel_checks: u64,
    states: std::collections::BTreeMap<i32, u64>,
    pool_reuses: u64,
    adds: u64,
    contacts_static: u64,
    contacts_ragdoll: u64,
    contacts_vehicle: u64,
    fast_bodies: u64,
    max_ragdolls: usize,
    min_y: f64,
    last_pool: usize,
}

struct Replay {
    stats: Stats,
    r: Reader,
    bullet: PzBullet,
    calls: u64,
    steps: u64,
    mismatches: u64,
    first: Vec<String>,
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
        if std::env::var_os("PZB_RSTATS").is_some() {
            self.count(tag, &want.0);
        }
        if std::env::var_os("PZB_TRACE_BODY_NAN").is_some() {
            if let Some(ws) = self.bullet.world.as_deref() {
                for (id, r) in &ws.ragdoll.ragdolls {
                    for (p, b) in r.ragdoll.as_ref().unwrap().bodies.iter().enumerate() {
                        let o = ws.dw.cw.obj(b.unwrap());
                        let rb = o.rb();
                        if o.world_transform.origin.x.is_nan()
                            || rb.linear_velocity.x.is_nan()
                            || rb.angular_velocity.x.is_nan()
                        {
                            println!(
                                "  body NaN after call {} tag {tag}: ragdoll {id} part {p}",
                                self.calls
                            );
                        }
                    }
                }
            }
        }
        if let Some(t) = std::env::var_os("PZB_TRACE") {
            let t = t.to_string_lossy().to_string();
            let (a, b) = t.split_once('-').unwrap_or(("0", "0"));
            if (a.parse().unwrap_or(0)..=b.parse().unwrap_or(0)).contains(&self.calls) {
                println!("  trace call {} tag {tag} -> {want:?}", self.calls);
            }
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

    fn count(&mut self, tag: u8, ret: &Ret) {
        let st = &mut self.stats;
        match (tag, ret) {
            (41, Ret::Bool(b)) => {
                st.wheel_checks += 1;
                st.wheel_hits += *b as u64;
            }
            (62, Ret::Int(v)) => *st.states.entry(*v).or_default() += 1,
            (52, _) => {
                st.adds += 1;
            }
            _ => {}
        }
        let Some(ws) = self.bullet.world.as_deref() else {
            return;
        };
        st.max_ragdolls = st.max_ragdolls.max(ws.ragdoll.ragdolls.len());
        if tag == 52 && ws.ragdoll.pool.len() < st.last_pool {
            st.pool_reuses += 1;
        }
        st.last_pool = ws.ragdoll.pool.len();
        if tag != 18 {
            return;
        }
        if std::env::var_os("PZB_POS").is_some() && self.steps.is_multiple_of(100) {
            for (id, v) in &ws.vehicles {
                println!(
                    "  step {} vehicle {id} at {:?}",
                    self.steps,
                    ws.dw.cw.obj(v.body).world_transform.origin
                );
            }
            for (id, r) in &ws.ragdoll.ragdolls {
                println!(
                    "  step {} ragdoll {id} pelvis {:?}",
                    self.steps,
                    ws.dw
                        .cw
                        .obj(r.ragdoll.as_ref().unwrap().body(0))
                        .world_transform
                        .origin
                );
            }
        }
        let mut is_rag = std::collections::HashSet::new();
        for r in ws.ragdoll.ragdolls.values() {
            for b in r.ragdoll.as_ref().unwrap().bodies.iter().flatten() {
                is_rag.insert(*b);
                let o = ws.dw.cw.obj(*b);
                if o.rb().linear_velocity.length() > 10.0 {
                    st.fast_bodies += 1;
                }
                let y = o.world_transform.origin.y;
                if y < st.min_y {
                    if y < -1.0 && st.min_y >= -1.0 {
                        println!("  ragdoll body below -1 at call {}", self.calls);
                    }
                    st.min_y = y;
                }
            }
        }
        let d = &ws.dw.cw.dispatcher;
        for &m in &d.manifolds_ptr {
            let m = d.manifold(m);
            if m.cached_points == 0 {
                continue;
            }
            let (a, b) = (is_rag.contains(&m.body0), is_rag.contains(&m.body1));
            if a && b {
                st.contacts_ragdoll += 1;
            } else if a || b {
                let other = if a { m.body1 } else { m.body0 };
                if ws.dw.cw.obj(other).user_index & 0xff == 1 {
                    st.contacts_vehicle += 1;
                } else {
                    st.contacts_static += 1;
                }
            }
        }
    }

    fn check_oarr(&mut self, tag: u8, got: &Option<Vec<f32>>) {
        let want = self.r.oarr();
        if std::env::var_os("PZB_TRACE_NAN").is_some() {
            if let Some(w) = &want {
                if let Some(i) = w.iter().position(|v| v.is_nan()) {
                    println!("  native NaN: call {} tag {tag} [{i}]", self.calls);
                }
            }
        }
        match (got, want) {
            (None, None) => {}
            (Some(g), Some(w))
                if g.len() == w.len() && g.iter().zip(&w).all(|(a, b)| same(*a, *b)) => {}
            (g, w) => {
                let i = match (g, &w) {
                    (Some(g), Some(w)) => g
                        .iter()
                        .zip(w)
                        .position(|(a, b)| !same(*a, *b))
                        .unwrap_or(0),
                    _ => 0,
                };
                let gi = g.as_ref().and_then(|g| g.get(i)).copied();
                let wi = w.as_ref().and_then(|w| w.get(i)).copied();
                let msg = format!(
                    "call {} (tag {tag}) array differs at [{i}]: got {:?} ({:x?}) native {:?} ({:x?})",
                    self.calls,
                    gi,
                    gi.map(f32::to_bits),
                    wi,
                    wi.map(f32::to_bits)
                );
                self.note(msg);
            }
        }
    }

    /// One ragdoll call: `$args` already read, `$e` runs it on the
    /// `Option<Vec<f32>>` arrays listed in `$arr`, which are checked after.
    fn ragdoll(&mut self, tag: u8) {
        let r = &mut self.r;
        macro_rules! go {
            ($($a:ident),* ; $e:expr) => {{
                let (q, ret, exc) = self.read_tail();
                #[allow(clippy::redundant_closure_call)]
                let got = ret_of(($e)(&mut self.bullet));
                let cbs = Expect { queue: q, mismatches: Vec::new() };
                self.check(tag, got, (ret, exc), cbs);
                $( self.check_oarr(tag, &$a); )*
            }};
        }
        macro_rules! ok {
            ($e:expr) => {{
                $e;
                Ok::<(), JniError>(())
            }};
        }
        fn m(a: &mut Option<Vec<f32>>) -> &mut [f32] {
            a.as_deref_mut().expect("non-null array")
        }
        match tag {
            50 => {
                let n = r.i32();
                let mut a = r.oarr();
                let q: [f32; 4] = r.fs();
                go!(a; |b: &mut PzBullet| ok!(b.initialize_ragdoll_pose(n, m(&mut a), q[0], q[1], q[2], q[3])))
            }
            51 => {
                let n = r.i32();
                let h = r.iarr();
                go!(; |b: &mut PzBullet| ok!(b.initialize_ragdoll_skeleton(n, &h)))
            }
            52 => {
                let id = r.i32();
                let p: [f32; 7] = r.fs();
                go!(; |b: &mut PzBullet| b.add_ragdoll(id, p[0], p[1], p[2], p[3], p[4], p[5], p[6]))
            }
            53 => {
                let id = r.i32();
                go!(; |b: &mut PzBullet| b.remove_ragdoll(id))
            }
            54 => {
                let id = r.i32();
                let mut a = r.oarr();
                go!(a; |b: &mut PzBullet| b.simulate_ragdoll(id, m(&mut a)))
            }
            55 => {
                let id = r.i32();
                let (mut a, mut c) = (r.oarr(), r.oarr());
                go!(a, c; |b: &mut PzBullet| b.simulate_ragdoll_with_rigid_body_output(id, m(&mut a), m(&mut c)))
            }
            56 => {
                let id = r.i32();
                let (mut a, mut c) = (r.oarr(), r.oarr());
                go!(a, c; |b: &mut PzBullet| b.update_skeleton_from_network_physics(id, m(&mut a), m(&mut c)))
            }
            57 => {
                let id = r.i32();
                let mut a = r.oarr();
                go!(a; |b: &mut PzBullet| b.get_corrected_world_space(id, m(&mut a)))
            }
            58 => {
                let id = r.i32();
                let q: [f32; 4] = r.fs();
                go!(; |b: &mut PzBullet| ok!(b.set_ragdoll_local_transform_rotation(id, q[0], q[1], q[2], q[3])))
            }
            59 => {
                let id = r.i32();
                let p: [f32; 7] = r.fs();
                go!(; |b: &mut PzBullet| ok!(b.update_ragdoll(id, p[0], p[1], p[2], p[3], p[4], p[5], p[6])))
            }
            60 => {
                let (id, n) = (r.i32(), r.i32());
                let mut a = r.oarr();
                go!(a; |b: &mut PzBullet| ok!(b.update_ragdoll_skeleton_transforms(id, n, m(&mut a))))
            }
            61 => {
                let (id, n, dt) = (r.i32(), r.i32(), r.f32());
                let mut a = r.oarr();
                go!(a; |b: &mut PzBullet| ok!(b.update_ragdoll_skeleton_previous_transforms(id, n, dt, m(&mut a))))
            }
            62 => {
                let id = r.i32();
                go!(; |b: &mut PzBullet| Ok::<i32, JniError>(b.get_ragdoll_simulation_state(id)))
            }
            63 => {
                let id = r.i32();
                go!(; |b: &mut PzBullet| ok!(b.reset_skeleton_pose(id)))
            }
            64 => {
                let (id, v) = (r.i32(), r.u8() != 0);
                go!(; |b: &mut PzBullet| ok!(b.set_ragdoll_active(id, v)))
            }
            65 => {
                let (id, v) = (r.i32(), r.u8() != 0);
                go!(; |b: &mut PzBullet| ok!(b.draw_debug_single_bone(id, v)))
            }
            66 | 67 => {
                let (id, v, w) = (r.i32(), r.u8() != 0, r.u8() != 0);
                if tag == 66 {
                    go!(; |b: &mut PzBullet| ok!(b.draw_debug_ragdoll_skeleton(id, v, w)))
                } else {
                    go!(; |b: &mut PzBullet| ok!(b.draw_debug_ragdoll_body_parts(id, v, w)))
                }
            }
            68 => {
                let (id, p) = (r.i32(), r.i32());
                go!(; |b: &mut PzBullet| ok!(b.highlight_ragdoll_body_part(id, p)))
            }
            69 | 70 => {
                let (id, p) = (r.i32(), r.i32());
                let mut a = r.oarr();
                if tag == 69 {
                    go!(a; |b: &mut PzBullet| ok!(b.apply_force(id, p, m(&mut a))))
                } else {
                    go!(a; |b: &mut PzBullet| ok!(b.apply_impulse(id, p, m(&mut a))))
                }
            }
            71 => {
                let (id, j) = (r.i32(), r.i32());
                go!(; |b: &mut PzBullet| ok!(b.detach_constraint(id, j)))
            }
            72 => {
                let v = r.f32();
                go!(; |b: &mut PzBullet| ok!(b.set_ragdoll_mass(v)))
            }
            73..=76 => {
                let a = r.oarr();
                let live = r.u8() != 0;
                go!(a; |b: &mut PzBullet| {
                    let p = a.as_deref();
                    match tag {
                        73 => b.define_ragdoll_constraints(p, live),
                        74 => b.define_ragdoll_anchors(p, live),
                        75 => b.define_ragdoll_body_part_info(p, live),
                        _ => b.define_ragdoll_body_dynamics(p, live),
                    }
                })
            }
            77 => {
                let id = r.i32();
                let a = r.oarr();
                go!(a; |b: &mut PzBullet| b.set_ragdoll_body_dynamics(id, a.as_deref()))
            }
            78 => {
                let id = r.i32();
                go!(; |b: &mut PzBullet| Ok::<bool, JniError>(b.reset_ragdoll_body_dynamics(id)))
            }
            79 => {
                let (q, ret, exc) = self.read_tail();
                let validate = self.r.u8();
                let n = self.r.i32();
                let bones: Vec<(String, i32)> =
                    (0..n).map(|_| (self.r.str(), self.r.i32())).collect();
                let ok = self.bullet.init_pz_bullet_with_bones(&mut Bones(bones));
                let cbs = Expect {
                    queue: q,
                    mismatches: Vec::new(),
                };
                self.check(tag, (Ret::Void, None), (ret, exc), cbs);
                if validate != 2 && (validate == 1) != ok {
                    let msg = format!(
                        "call {}: bone validation {ok}, native {validate}",
                        self.calls
                    );
                    self.note(msg);
                }
            }
            _ => panic!("unknown tag {tag} at {}", self.r.p - 1),
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
                50..=79 => self.ragdoll(tag),
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
            stats: Stats {
                min_y: f64::MAX,
                ..Default::default()
            },
            r: Reader { d, p: 0 },
            bullet: PzBullet::new(),
            calls: 0,
            steps: 0,
            mismatches: 0,
            first: Vec::new(),
        };
        // A log cut short (the original crashed mid-call) ends the replay at
        // the last complete call.
        let hook = std::panic::take_hook();
        if std::env::var_os("PZB_PANIC").is_none() {
            std::panic::set_hook(Box::new(|_| {}));
        }
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
        if std::env::var_os("PZB_RSTATS").is_some() {
            println!("  {:?}", rep.stats);
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
