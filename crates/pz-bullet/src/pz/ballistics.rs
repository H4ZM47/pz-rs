//! The game's ballistics glue (PZBallistics.cpp, PZBallisticsTarget.cpp and
//! their pools), reconstructed from the shipped arm64 build.
//!
//! A `PZBallistics` is a shooter: a muzzle transform, a range and an aim
//! reticle transform. A `PZBallisticsTarget` is a character that can be
//! hit: an adjustable box collision object (`BulletObject` type 11) around
//! the body parts of a ragdoll posed from the character's skeleton. The
//! queries cast rays through the dynamics world and report hits as float
//! records (see [`crate::api_ballistics`]).
//!
//! Debug-only state of the original (colours, debug shapes and their
//! transforms, the hit positions and colours written into `BulletObject`s,
//! `PZBallistics::cameraAimTransform`'s origin) is not kept: nothing outside
//! the debug renderer reads it. The one `BulletObject` field the queries
//! read back, the "hit" flag of body parts (`getTargetedBodyPart`), is.
//!
//! `C rand()` is the macOS libc generator (Park-Miller minimal standard,
//! seed 1, never reseeded by the library); [`CRand`] reproduces it.

use std::collections::BTreeMap;

use super::ragdoll::{Ragdoll, RagdollCtx, BODY_PART_COUNT};
use super::ragdoll_builder::RagdollBuilder;
use super::ragdoll_skeleton::SkeletonPose;
use crate::bt::collision_object::*;
use crate::bt::collision_world::{AllHitsRayResultCallback, ClosestRayResultCallback};
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;
use crate::pz::world::WorldSimulation;

/// `BulletObjectType` of a ballistics target's collision box.
pub const BULLET_OBJECT_BALLISTICS_TARGET: i64 = 11;
/// `BulletObjectType` of a target's body part (while it is in the world).
pub const BULLET_OBJECT_BODY_PART: i64 = 12;

mod libm {
    extern "C" {
        pub fn acos(x: f64) -> f64;
        pub fn powf(x: f32, y: f32) -> f32;
    }
}
/// Plain `acos` (the binary does not clamp first).
fn c_acos(x: f64) -> f64 {
    // SAFETY: platform libm.
    unsafe { libm::acos(x) }
}
/// `powf` from the platform libm.
fn c_powf(x: f32, y: f32) -> f32 {
    // SAFETY: platform libm.
    unsafe { libm::powf(x, y) }
}

/// macOS libc `rand()`: `next = 16807 * next mod (2^31 - 1)` by Schrage's
/// method, starting from 1 (`srand` is never called by the library, and
/// nothing else in the game's process draws from it).
#[derive(Clone, Copy, Debug)]
pub struct CRand {
    pub next: i64,
}

impl Default for CRand {
    fn default() -> Self {
        CRand { next: 1 }
    }
}

impl CRand {
    pub fn rand(&mut self) -> i32 {
        let mut x = self.next;
        if x == 0 {
            x = 123_459_876;
        }
        let hi = x / 127_773;
        let lo = x % 127_773;
        x = 16807 * lo - 2836 * hi;
        if x < 0 {
            x += 0x7fff_ffff;
        }
        self.next = x;
        x as i32
    }
}

/// What the ballistics code needs from a target's ragdoll, implemented by
/// the ragdoll port's [`Ragdoll`] with [`SkeletonPose`] and the shared
/// [`RagdollBuilder`] (the world's [`RagdollShapeCache`] holds the
/// body-part shapes ragdolls and targets share).
///
/// [`RagdollShapeCache`]: crate::pz::ragdoll::RagdollShapeCache
pub trait BallisticsRagdoll: Sized {
    type Builder;
    type Pose;
    /// `RagdollBuilder::createSkeletonPose()`, `new Ragdoll()` and
    /// `Ragdoll::create(pose)`. `None` when the builder has no skeleton yet.
    fn create(b: &Self::Builder, ws: &mut WorldSimulation) -> Option<(Self, Self::Pose)>;
    /// Writes the ragdoll transform's origin (`PZBallisticsTarget::update`).
    fn set_origin(&mut self, origin: Vec3);
    /// The parent of `bone` in the pose's skeleton.
    fn bone_parent(pose: &Self::Pose, bone: usize) -> i32;
    /// `SkeletonPose::setSkeletonPose(absolute)` then
    /// `Ragdoll::updateBodiesFromSkeleton(pose)`.
    fn set_pose_and_update_bodies(
        &mut self,
        pose: &mut Self::Pose,
        absolute: &[Transform],
        ws: &mut WorldSimulation,
    );
    /// `Ragdoll::updateBodyParts(pose)` (live body-part edits).
    fn update_body_parts(
        &mut self,
        b: &Self::Builder,
        pose: &mut Self::Pose,
        ws: &mut WorldSimulation,
    );
    /// `getRigidBody(i)`.
    fn rigid_body(&self, part: usize) -> Option<ObjId>;
    /// `RagdollBuilder::getRagdollBodyPartInfo(i)->part`.
    fn body_part_info_part(b: &Self::Builder, i: usize) -> i32;
}

impl BallisticsRagdoll for Ragdoll {
    type Builder = RagdollBuilder;
    type Pose = SkeletonPose;
    fn create(b: &RagdollBuilder, ws: &mut WorldSimulation) -> Option<(Self, SkeletonPose)> {
        let sk = b.default_skeleton.as_ref()?;
        let mut pose = SkeletonPose::new(sk.clone());
        let mut r = Ragdoll::new();
        let mut ctx = RagdollCtx {
            builder: b,
            dw: &mut ws.dw,
            shapes: &mut ws.ragdoll.shapes,
        };
        r.create(&mut ctx, &mut pose);
        Some((r, pose))
    }
    fn set_origin(&mut self, origin: Vec3) {
        self.transform.origin = origin;
    }
    fn bone_parent(pose: &SkeletonPose, bone: usize) -> i32 {
        pose.skeleton.parent(bone)
    }
    fn set_pose_and_update_bodies(
        &mut self,
        pose: &mut SkeletonPose,
        absolute: &[Transform],
        ws: &mut WorldSimulation,
    ) {
        pose.set_skeleton_pose(absolute);
        self.update_bodies_from_skeleton(&mut ws.dw, pose);
    }
    fn update_body_parts(
        &mut self,
        b: &RagdollBuilder,
        pose: &mut SkeletonPose,
        ws: &mut WorldSimulation,
    ) {
        let mut ctx = RagdollCtx {
            builder: b,
            dw: &mut ws.dw,
            shapes: &mut ws.ragdoll.shapes,
        };
        Ragdoll::update_body_parts(self, &mut ctx, pose);
    }
    fn rigid_body(&self, part: usize) -> Option<ObjId> {
        self.bodies.get(part).copied().flatten()
    }
    fn body_part_info_part(b: &RagdollBuilder, i: usize) -> i32 {
        b.script
            .get_body_part_info(i as i32)
            .map_or(i as i32, |info| info.part)
    }
}

/// The ragdoll type targets use.
pub type TargetRagdoll = Ragdoll;
type Builder = <TargetRagdoll as BallisticsRagdoll>::Builder;
type Pose = <TargetRagdoll as BallisticsRagdoll>::Pose;
const BODY_PARTS: usize = BODY_PART_COUNT;

/// Process-wide statics of the ballistics code.
#[derive(Clone, Debug)]
pub struct BallisticsStatics {
    pub rand: CRand,
    /// `PZBallistics::aimShape` exists: only the first `PZBallistics` ever
    /// made gets its aim transform initialised to identity.
    pub aim_shape_created: bool,
    /// `PZBallistics::cameraAimTransform`'s basis (debug drawing only; it
    /// would feed `computeIsometricTargetAabb`).
    pub camera_aim_basis: Mat3,
    /// `PZBallisticsTarget::useIsometricAabb` (never set by the game).
    pub use_isometric_aabb: bool,
    /// `PZBallisticsTarget::extentsScale` (`setBallisticsTargetAdjustingShapeScale`).
    pub extents_scale: Vec3,
    /// `PZBallisticsTarget::allPartsColor` (debug colour).
    pub all_parts_color: Vec3,
}

impl Default for BallisticsStatics {
    fn default() -> Self {
        BallisticsStatics {
            rand: CRand::default(),
            aim_shape_created: false,
            camera_aim_basis: Mat3::IDENTITY,
            use_isometric_aabb: false,
            extents_scale: Vec3::new(0.5, 0.5, 0.5),
            all_parts_color: Vec3::new(0.0, 1.0, 1.0),
        }
    }
}

/// `PZBallistics`.
#[derive(Clone, Debug)]
pub struct PzBallistics {
    pub id: i32,
    /// `+0x00`: muzzle transform (aim direction basis, muzzle origin).
    pub transform: Transform,
    /// `+0xa0`.
    pub range: f64,
    /// `+0x118`: aim reticle transform. Left uninitialised by the original
    /// for every `PZBallistics` but the first; zero here.
    pub aim: Transform,
    /// `setBallisticsSize` (scales the debug shapes).
    pub size: f64,
    /// `setBallisticsColor` (debug colour).
    pub color: Vec3,
}

impl PzBallistics {
    /// `PZBallistics(id)`.
    fn new(id: i32, statics: &mut BallisticsStatics) -> PzBallistics {
        let zero = Transform::new(
            Mat3::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            Vec3::ZERO,
        );
        let mut b = PzBallistics {
            id,
            transform: Transform::IDENTITY,
            range: 0.0,
            aim: zero,
            size: 1.0,
            color: Vec3::new(0.0, 1.0, 1.0),
        };
        // initialize(): only while the static aim shape does not exist.
        if !statics.aim_shape_created {
            b.aim = Transform::IDENTITY;
            statics.aim_shape_created = true;
        }
        b
    }

    /// `update(x, y, z)`: the muzzle position, only while the range is set.
    pub fn update(&mut self, x: f32, y: f32, z: f32) {
        if 0.0 < self.range {
            self.transform.origin = Vec3::new(x as f64, y as f64, z as f64);
        }
    }

    /// `updateMuzzleAimDirection(x, y, z)`: the rotation from forward
    /// (`+z`) to the direction.
    pub fn update_muzzle_aim_direction(&mut self, x: f32, y: f32, z: f32) {
        let forward = Vec3::new(0.0, 0.0, 1.0);
        let dir = Vec3::new(x as f64, y as f64, z as f64);
        let q = if (-forward - dir).length() < 1e-5 {
            Quat::from_axis_angle(&Vec3::new(0.0, 1.0, 0.0), SIMD_PI)
        } else {
            let axis = forward.cross(&dir).normalized();
            let angle = c_acos(forward.dot(&dir));
            Quat::from_axis_angle(&axis, angle)
        };
        self.transform.basis.set_rotation(&q);
    }

    /// `updateAimReticleQuaternion(q)`.
    pub fn update_aim_reticle_quaternion(&mut self, q: &Quat, statics: &mut BallisticsStatics) {
        let mut m = Mat3::IDENTITY;
        m.set_rotation(q);
        statics.camera_aim_basis = m;
        self.aim.basis = m;
    }

    /// `updateAimReticleRotation(q)`.
    pub fn update_aim_reticle_rotation(&mut self, q: &Quat) {
        self.aim.basis.set_rotation(q);
    }

    /// `updateAimReticleRotate(q)`: rotates the reticle by `q`.
    pub fn update_aim_reticle_rotate(&mut self, q: &Quat) {
        let cur = self.aim.basis.get_rotation();
        self.aim.basis.set_rotation(&(cur * *q));
    }

    /// `raycastClosestWithCompoundSupport(world, from, to, result)`.
    fn raycast_closest_with_compound_support(
        ws: &WorldSimulation,
        from: &Vec3,
        to: &Vec3,
    ) -> Option<CompoundHit> {
        let cw = &ws.dw.cw;
        let mut cb = ClosestRayResultCallback::new(*from, *to);
        cw.ray_test(from, to, &mut cb);
        let obj = cb.base.collision_object?;
        let shape_id = cw.obj(obj).shape();
        let shape = cw.shapes.get(shape_id);
        if !shape.is_compound() {
            if cb.base.closest_hit_fraction < 1.0 {
                return Some(CompoundHit {
                    object: obj,
                    child_index: -1,
                    hit_point: cb.hit_point_world,
                });
            }
            return None;
        }
        let ray_from_trans = Transform::new(Mat3::IDENTITY, *from);
        let ray_to_trans = Transform::new(Mat3::IDENTITY, *to);
        let world = cw.obj(obj).world_transform;
        let compound = shape.compound();
        let mut best: Option<CompoundHit> = None;
        let mut closest = 1.0;
        for i in 0..compound.get_num_child_shapes() {
            let child = compound.get_child_shape(i);
            let child_world = world * *compound.get_child_transform(i);
            let (aabb_min, aabb_max) = cw.shapes.get(child).get_aabb(&child_world);
            let mut param = 1.0;
            let mut normal = Vec3::ZERO;
            if !bt_ray_aabb(from, to, &aabb_min, &aabb_max, &mut param, &mut normal) {
                continue;
            }
            let mut ccb = ClosestRayResultCallback::new(*from, *to);
            // m_collisionObject is preset, so the hit test always passes.
            ccb.base.collision_object = Some(obj);
            cw.ray_test_single(
                &ray_from_trans,
                &ray_to_trans,
                obj,
                child,
                &child_world,
                &mut ccb,
            );
            let f = ccb.base.closest_hit_fraction;
            if f < closest {
                best = Some(CompoundHit {
                    object: obj,
                    child_index: i as i32,
                    hit_point: from.lerp(to, f),
                });
                closest = f;
            }
        }
        if closest < 1.0 {
            best
        } else {
            None
        }
    }
}

/// `AllCompoundHitsRayResult::Hit` (the fields the queries read).
#[derive(Clone, Copy, Debug)]
struct CompoundHit {
    object: ObjId,
    child_index: i32,
    hit_point: Vec3,
}

/// A body part's `BulletObject` (type 12), made the first time its target
/// enters camera targeting.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyPartObject {
    pub id: i32,
    pub part: i32,
    /// `+0x40`: set by every camera ray that hits the part; never cleared
    /// outside the debug renderer.
    pub hit: bool,
}

/// `PZBallisticsTarget`.
#[derive(Clone, Debug)]
pub struct PzBallisticsTarget {
    pub id: i32,
    /// `+0x08`: the adjusting shape (a box).
    pub adjusting_shape: ShapeId,
    /// `+0x60`.
    pub position: Vec3,
    /// `+0x80`: the skeleton's root transform (`setAxis`).
    pub axis: Transform,
    /// `+0x48` / `+0x50`.
    pub ragdoll: Option<(TargetRagdoll, Pose)>,
    /// `+0x58`: the targeting collision object.
    pub collision: ObjId,
    pub body_objects: [Option<BodyPartObject>; BODY_PARTS],
}

impl PzBallisticsTarget {
    /// `PZBallisticsTarget(id)` (with `createTargetingCollision`).
    fn new(id: i32, b: &Builder, ws: &mut WorldSimulation) -> Self {
        let ragdoll = <TargetRagdoll as BallisticsRagdoll>::create(b, ws);
        let half = flt(0.175);
        let shape = ws.dw.cw.shapes.add(Shape::new_box(&Vec3::new(
            half,
            0.816_496_670_246_124_3,
            half,
        )));
        let mut o = CollisionObject::new();
        o.set_collision_shape(shape);
        o.collision_flags = CF_NO_CONTACT_RESPONSE;
        o.user_index = user_index(id, BULLET_OBJECT_BALLISTICS_TARGET);
        let collision = ws.dw.cw.insert_object(o);
        PzBallisticsTarget {
            id,
            adjusting_shape: shape,
            position: Vec3::ZERO,
            axis: Transform::IDENTITY,
            ragdoll,
            collision,
            body_objects: [None; BODY_PARTS],
        }
    }

    /// `reset(id)` (when the pool hands out a released target).
    fn reset(&mut self, id: i32, ws: &mut WorldSimulation) {
        self.id = id;
        ws.dw.cw.obj_mut(self.collision).user_index =
            user_index(id, BULLET_OBJECT_BALLISTICS_TARGET);
        self.axis = Transform::IDENTITY;
    }

    /// `setAxis(x, y, z)`: `Rz(z) * Ry(y) * Rx(x)`.
    pub fn set_axis(&mut self, x: f32, y: f32, z: f32) {
        let qz = Quat::from_axis_angle(&Vec3::new(0.0, 0.0, 1.0), z as f64);
        let qy = Quat::from_axis_angle(&Vec3::new(0.0, 1.0, 0.0), y as f64);
        let qx = Quat::from_axis_angle(&Vec3::new(1.0, 0.0, 0.0), x as f64);
        self.axis = Transform::new(Mat3::from_quat(&((qz * qy) * qx)), Vec3::ZERO);
    }

    /// `update(x, y, z, qx, qy, qz, qw, b)`: only the position is used.
    pub fn update(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        b: &Builder,
        ws: &mut WorldSimulation,
        statics: &BallisticsStatics,
    ) {
        self.position = Vec3::new(x as f64, y as f64, z as f64);
        if let Some((r, _)) = &mut self.ragdoll {
            r.set_origin(self.position);
        }
        ws.dw
            .cw
            .obj_mut(self.collision)
            .set_collision_shape(self.adjusting_shape);
        if statics.use_isometric_aabb {
            // computeIsometricTargetAabb is unreachable in the game (the flag
            // is never set); it is not ported.
            unimplemented!("PZBallisticsTarget::computeIsometricTargetAabb");
        }
        self.compute_target_aabb(b, ws);
    }

    /// `computeTargetAabb()`: the box spans the AABB of the body parts
    /// except the arms' and calves' (parts 1, 3, 5, 7, 9).
    fn compute_target_aabb(&mut self, b: &Builder, ws: &mut WorldSimulation) {
        let mut mn = Vec3::new(BT_LARGE_FLOAT, BT_LARGE_FLOAT, BT_LARGE_FLOAT);
        let mut mx = Vec3::new(-BT_LARGE_FLOAT, -BT_LARGE_FLOAT, -BT_LARGE_FLOAT);
        let acc = |mn: &mut Vec3, mx: &mut Vec3, a: &Vec3, c: &Vec3| {
            for k in 0..3 {
                if a[k] <= mn[k] {
                    mn[k] = a[k];
                }
                if mx[k] <= c[k] {
                    mx[k] = c[k];
                }
            }
        };
        if let Some((r, _)) = &self.ragdoll {
            let cw = &ws.dw.cw;
            for i in 0..BODY_PARTS {
                let p = TargetRagdoll::body_part_info_part(b, i);
                if (p as u32) <= 9 && ((1u32 << (p as u32 & 31)) & 0x2aa) != 0 {
                    continue;
                }
                let Some(body) = r.rigid_body(p as usize) else {
                    continue;
                };
                let o = cw.obj(body);
                let Some(sid) = o.collision_shape else {
                    continue;
                };
                let t = o.world_transform;
                let shape = cw.shapes.get(sid);
                if shape.is_compound() {
                    let c = shape.compound();
                    for k in (0..c.get_num_child_shapes()).rev() {
                        let cwt = t * *c.get_child_transform(k);
                        let (a, z) = cw.shapes.get(c.get_child_shape(k)).get_aabb(&cwt);
                        acc(&mut mn, &mut mx, &a, &z);
                    }
                } else {
                    let (a, z) = shape.get_aabb(&t);
                    acc(&mut mn, &mut mx, &a, &z);
                }
            }
        }
        let s = flt(0.45);
        let ext = Vec3::new((mx.x - mn.x) * s, (mx.y - mn.y) * s, (mx.z - mn.z) * s);
        if let crate::bt::shapes::ShapeKind::Box(bx) =
            &mut ws.dw.cw.shapes.get_mut(self.adjusting_shape).kind
        {
            bx.ci.implicit_shape_dimensions = ext;
        }
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&Quat::IDENTITY);
        let o = ws.dw.cw.obj_mut(self.collision);
        o.update_revision += 1;
        o.world_transform = Transform::new(
            basis,
            Vec3::new(
                (mx.x + mn.x) * 0.5,
                (mx.y + mn.y) * 0.5,
                (mx.z + mn.z) * 0.5,
            ),
        );
    }

    /// `updateSkeletonBoneTransforms(count, data)`: local bone transforms
    /// (position, quaternion; 7 floats each), the root replaced by the axis.
    pub fn update_skeleton_bone_transforms(
        &mut self,
        count: i32,
        data: &[f32],
        ws: &mut WorldSimulation,
    ) {
        let Some((r, pose)) = &mut self.ragdoll else {
            return;
        };
        let n = count.max(0) as usize;
        let mut local = Vec::with_capacity(n);
        for i in 0..n {
            let d = &data[i * 7..i * 7 + 7];
            let mut m = Mat3::IDENTITY;
            m.set_rotation(&Quat::new(
                d[3] as f64,
                d[4] as f64,
                d[5] as f64,
                d[6] as f64,
            ));
            local.push(Transform::new(
                m,
                Vec3::new(d[0] as f64, d[1] as f64, d[2] as f64),
            ));
        }
        let mut abs = vec![Transform::IDENTITY; n.max(1)];
        abs[0] = self.axis;
        for i in 1..n {
            let p = TargetRagdoll::bone_parent(pose, i) as usize;
            abs[i] = abs[p] * local[i];
        }
        r.set_pose_and_update_bodies(pose, &abs, ws);
    }

    /// `addToWorld()`: 1 when the box is already in the world, else adds it
    /// (group 16, mask -1) and returns 0.
    fn add_to_world(&self, ws: &mut WorldSimulation) -> i32 {
        if ws.dw.cw.collision_objects.contains(&self.collision) {
            return 1;
        }
        ws.dw.add_collision_object(self.collision, 16, -1);
        0
    }

    /// `removeFromWorld()`: 0 when removed, 1 when it was not in the world.
    fn remove_from_world(&self, ws: &mut WorldSimulation) -> i32 {
        if ws.dw.cw.collision_objects.contains(&self.collision) {
            ws.dw.remove_collision_object(self.collision);
            return 0;
        }
        1
    }

    /// `addBodyPartCollision()`: every body part enters the world as a
    /// collision object (flags 4, group 16, mask -1).
    fn add_body_part_collision(&mut self, ws: &mut WorldSimulation) {
        let Some((r, _)) = &self.ragdoll else { return };
        for i in 0..BODY_PARTS {
            if let Some(body) = r.rigid_body(i) {
                let bo = self.body_objects[i].get_or_insert_with(BodyPartObject::default);
                bo.id = self.id;
                bo.part = i as i32;
                let o = ws.dw.cw.obj_mut(body);
                o.collision_flags = CF_NO_CONTACT_RESPONSE;
                o.user_index = user_index(self.id, BULLET_OBJECT_BODY_PART) | ((i as i64) << 40);
                ws.dw.add_collision_object(body, 16, -1);
            }
        }
    }

    /// `removeBodyPartCollision()`.
    fn remove_body_part_collision(&self, ws: &mut WorldSimulation) {
        let Some((r, _)) = &self.ragdoll else { return };
        for i in 0..BODY_PARTS {
            if let Some(body) = r.rigid_body(i) {
                ws.dw.remove_collision_object(body);
            }
        }
    }

    /// `getTargetedBodyPart()`: the first body part ever hit by a camera
    /// ray, 11 if none.
    pub fn get_targeted_body_part(&self) -> i32 {
        if let Some((r, _)) = &self.ragdoll {
            for i in 0..BODY_PARTS {
                if r.rigid_body(i).is_some() {
                    if let Some(bo) = &self.body_objects[i] {
                        if bo.hit {
                            return i as i32;
                        }
                    }
                }
            }
        }
        11
    }
}

/// `(id << 8) | type` (low byte: `BulletObjectType`). Body parts also keep
/// their index in bits 40.. .
fn user_index(id: i32, kind: i64) -> i64 {
    (((id as i64) << 8) & 0xff_ffff_ff00) | kind
}

/// The `BulletObject` a hit object carries: (type, id).
fn bullet_object(ws: &WorldSimulation, obj: ObjId) -> Option<(i64, i32)> {
    let u = ws.dw.cw.obj(obj).user_index;
    if u == 0 {
        return None;
    }
    let kind = u & 0xff;
    let id = match kind {
        1 => (u >> 8) as i32,
        BULLET_OBJECT_BALLISTICS_TARGET | BULLET_OBJECT_BODY_PART => {
            ((u >> 8) & 0xffff_ffff) as u32 as i32
        }
        _ => -1,
    };
    Some((kind, id))
}

/// `WorldSimulation`'s ballistics state: the `PZBallisticsPool` (`+0x100`)
/// and `PZBallisticsTargetPool` (`+0x148`), each a map by id plus a LIFO
/// free list of released objects (which keep their state).
#[derive(Debug, Default)]
pub struct BallisticsWorld {
    pub ballistics: BTreeMap<i32, usize>,
    pub ballistics_objects: Vec<PzBallistics>,
    pub ballistics_free: Vec<usize>,
    pub targets: BTreeMap<i32, usize>,
    pub target_objects: Vec<PzBallisticsTarget>,
    pub target_free: Vec<usize>,
}

impl WorldSimulation {
    /// `PZBallisticsPool::alloc(id)` (`WorldSimulation::addBallistics`).
    pub fn add_ballistics(&mut self, id: i32, statics: &mut BallisticsStatics) {
        let bw = &mut self.ballistics;
        if bw.ballistics.contains_key(&id) {
            return;
        }
        let idx = match bw.ballistics_free.pop() {
            Some(i) => {
                bw.ballistics_objects[i].id = id;
                i
            }
            None => {
                bw.ballistics_objects.push(PzBallistics::new(id, statics));
                bw.ballistics_objects.len() - 1
            }
        };
        bw.ballistics.insert(id, idx);
    }

    /// `WorldSimulation::removeBallistics(id)` (`PZBallisticsPool::release`).
    pub fn remove_ballistics(&mut self, id: i32) {
        if let Some(idx) = self.ballistics.ballistics.remove(&id) {
            self.ballistics.ballistics_free.push(idx);
        }
    }

    pub fn ballistics_mut(&mut self, id: i32) -> Option<&mut PzBallistics> {
        let idx = *self.ballistics.ballistics.get(&id)?;
        Some(&mut self.ballistics.ballistics_objects[idx])
    }

    /// The target with this id, allocating it from the pool when missing
    /// (`PZBallisticsTargetPool::alloc`).
    pub fn ballistics_target_alloc(&mut self, id: i32, b: &Builder) -> usize {
        if let Some(&idx) = self.ballistics.targets.get(&id) {
            return idx;
        }
        let idx = match self.ballistics.target_free.pop() {
            Some(i) => {
                let mut t =
                    std::mem::replace(&mut self.ballistics.target_objects[i], placeholder_target());
                t.reset(id, self);
                self.ballistics.target_objects[i] = t;
                i
            }
            None => {
                let t = PzBallisticsTarget::new(id, b, self);
                self.ballistics.target_objects.push(t);
                self.ballistics.target_objects.len() - 1
            }
        };
        self.ballistics.targets.insert(id, idx);
        idx
    }

    /// Runs `f` on target `idx` with the world borrowed separately.
    pub fn with_target<T>(
        &mut self,
        idx: usize,
        f: impl FnOnce(&mut PzBallisticsTarget, &mut WorldSimulation) -> T,
    ) -> T {
        let mut t = std::mem::replace(
            &mut self.ballistics.target_objects[idx],
            placeholder_target(),
        );
        let r = f(&mut t, self);
        self.ballistics.target_objects[idx] = t;
        r
    }

    /// `addBallisticsTarget`: `addToWorld()`.
    pub fn ballistics_target_add_to_world(&mut self, idx: usize) -> i32 {
        self.with_target(idx, |t, ws| t.add_to_world(ws))
    }

    /// `removeBallisticsTarget`: `removeFromWorld()` and back to the pool.
    pub fn remove_ballistics_target(&mut self, id: i32) -> i32 {
        let Some(idx) = self.ballistics.targets.get(&id).copied() else {
            return 0;
        };
        let r = self.with_target(idx, |t, ws| t.remove_from_world(ws));
        self.ballistics.target_free.push(idx);
        let tid = self.ballistics.target_objects[idx].id;
        self.ballistics.targets.remove(&tid);
        r
    }

    /// Live body-part changes (`updateRagdollBodyParts(true)`): every target,
    /// in the map and in the free list, rebuilds its bodies.
    pub fn ballistics_targets_update_body_parts(&mut self, b: &Builder) {
        let mut order: Vec<usize> = self.ballistics.targets.values().copied().collect();
        order.extend(self.ballistics.target_free.iter().rev().copied());
        for idx in order {
            self.with_target(idx, |t, ws| {
                if let Some((r, pose)) = &mut t.ragdoll {
                    BallisticsRagdoll::update_body_parts(r, b, pose, ws);
                }
            });
        }
    }

    /// `PZBallistics::getTargets(range, count, out)`: one ray from the muzzle
    /// along its direction; 4 floats per hit (id, x, y, z).
    pub fn ballistics_get_targets(
        &mut self,
        id: i32,
        range: f32,
        count: i32,
        out: &mut [f32],
    ) -> i32 {
        let Some(&idx) = self.ballistics.ballistics.get(&id) else {
            return 0;
        };
        let b = &self.ballistics.ballistics_objects[idx];
        let from = b.transform.origin;
        let to = b.transform.apply(&Vec3::new(0.0, 0.0, range as f64));
        let hit = PzBallistics::raycast_closest_with_compound_support(self, &from, &to);
        let mut n = 0;
        if let Some(h) = hit {
            if let Some((kind, oid)) = bullet_object(self, h.object) {
                if kind == BULLET_OBJECT_BALLISTICS_TARGET || kind == 1 {
                    let (ox, oy) = (self.offset_x as i64 as f64, self.offset_y as i64 as f64);
                    put(out, 0, oid as f32);
                    put(out, 1, (h.hit_point.x + ox) as f32);
                    put(out, 2, h.hit_point.y as f32);
                    put(out, 3, (h.hit_point.z + oy) as f32);
                    n = 1;
                }
            }
        }
        let _ = count;
        n
    }

    /// `PZBallistics::getSpreadTargetData(range, spread, weightCenter, count,
    /// max, out)`: `count` rays in a cone around the muzzle direction;
    /// 4 floats each (id or 0, x, y, z of the hit or the ray end).
    #[allow(clippy::too_many_arguments)]
    pub fn ballistics_get_spread_target_data(
        &mut self,
        id: i32,
        range: f32,
        spread: f32,
        weight_center: f32,
        count: i32,
        _max: i32,
        out: &mut [f32],
        statics: &mut BallisticsStatics,
    ) -> i32 {
        let Some(&idx) = self.ballistics.ballistics.get(&id) else {
            return 0;
        };
        let t = self.ballistics.ballistics_objects[idx].transform;
        let from = t.origin;
        let range = range as f64;
        let (ox, oy) = (self.offset_x as i64 as f64, self.offset_y as i64 as f64);
        for i in 0..count.max(0) as usize {
            let r1 = statics.rand.rand();
            let r2 = statics.rand.rand();
            let angle = ((r1 as f32) as f64 * std::f64::consts::TAU) / 2_147_483_647.0;
            let u = (r2 as f32) / 2_147_483_648.0f32;
            let radius = (c_powf(u, weight_center) * spread) as f64;
            let (s, c) = bt_sincos(angle);
            let target = t.apply(&Vec3::new(c * radius, s * radius, range));
            let k = i * 4;
            put(out, k, 0.0);
            put(out, k + 1, (target.x + ox) as f32);
            put(out, k + 2, target.y as f32);
            put(out, k + 3, (target.z + oy) as f32);
            let Some(h) = PzBallistics::raycast_closest_with_compound_support(self, &from, &target)
            else {
                continue;
            };
            let Some((kind, oid)) = bullet_object(self, h.object) else {
                continue;
            };
            let report =
                kind == BULLET_OBJECT_BALLISTICS_TARGET || (kind == 1 && h.child_index >= 0);
            if report {
                put(out, k, oid as f32);
                put(out, k + 1, (h.hit_point.x + ox) as f32);
                put(out, k + 2, h.hit_point.y as f32);
                put(out, k + 3, (h.hit_point.z + oy) as f32);
            }
        }
        count
    }

    /// `PZBallistics::getCameraTargets(range, count, out, parts)`: an
    /// all-hits ray 150 units each way through the aim reticle along its
    /// `-z`; 5 floats per target within `range` of the muzzle (id, x, y, z
    /// of the ray's first hit, body part). With `parts`, the hit targets'
    /// body parts join the world for a second ray that picks the body part.
    pub fn ballistics_get_camera_targets(
        &mut self,
        id: i32,
        range: f32,
        count: i32,
        out: &mut [f32],
        parts: bool,
    ) -> i32 {
        let Some(&idx) = self.ballistics.ballistics.get(&id) else {
            return 0;
        };
        let b = &self.ballistics.ballistics_objects[idx];
        let muzzle = b.transform.origin;
        let mut dir = b.aim.basis * Vec3::new(0.0, 0.0, -1.0);
        dir.normalize();
        dir *= 150.0;
        let from = b.aim.origin - dir;
        let to = b.aim.origin + dir;
        if (to - from).length() == 0.0 {
            return 0;
        }
        let mut cb = AllHitsRayResultCallback::new(from, to);
        self.dw.cw.ray_test(&from, &to, &mut cb);
        let (ox, oy) = (self.offset_x as i64 as f64, self.offset_y as i64 as f64);
        let mut n = 0i32;
        let mut k = 0usize;
        for i in 0..cb.collision_objects.len() {
            let obj = cb.collision_objects[i];
            let Some((kind, oid)) = bullet_object(self, obj) else {
                continue;
            };
            if kind != BULLET_OBJECT_BALLISTICS_TARGET {
                continue;
            }
            let d = muzzle - self.dw.cw.obj(obj).world_transform.origin;
            if d.length() > range as f64 {
                continue;
            }
            // The original reports the ray's first hit point for every target.
            let hp = cb.hit_point_world[0];
            put(out, k, oid as f32);
            put(out, k + 1, (hp.x + ox) as f32);
            put(out, k + 2, hp.y as f32);
            put(out, k + 3, (hp.z + oy) as f32);
            put(out, k + 4, 11.0);
            n += 1;
            if count <= n {
                break;
            }
            k += 5;
        }
        if !parts || n < 1 {
            return n;
        }
        let ids: Vec<i32> = (0..n as usize).map(|j| get(out, j * 5) as i32).collect();
        if !self.ballistics.targets.is_empty() {
            for &tid in &ids {
                if let Some(&ti) = self.ballistics.targets.get(&tid) {
                    self.with_target(ti, |t, ws| t.add_body_part_collision(ws));
                }
            }
        }
        // The same callback again: its hits accumulate.
        self.dw.cw.ray_test(&from, &to, &mut cb);
        let mut best: i32 = -1;
        for i in 0..cb.collision_objects.len() {
            let obj = cb.collision_objects[i];
            let u = self.dw.cw.obj(obj).user_index;
            if u == 0 || (u & 0xff) != BULLET_OBJECT_BODY_PART {
                continue;
            }
            let tid = ((u >> 8) & 0xffff_ffff) as u32 as i32;
            let part_index = ((u >> 40) & 0xff) as usize;
            let Some(&ti) = self.ballistics.targets.get(&tid) else {
                continue;
            };
            let tgt = &mut self.ballistics.target_objects[ti];
            let Some(bo) = tgt.body_objects[part_index].as_mut() else {
                continue;
            };
            let p = bo.part;
            let accept = if best == -1 {
                true
            } else {
                let thr: u32 = match best {
                    2 => 0,
                    1 => 1,
                    0 => 2,
                    _ => 3,
                };
                (p as u32) <= 2 && (2u32.wrapping_sub(p as u32)) < thr
            };
            if accept {
                let pid = bo.id as f32;
                let mut j = 0usize;
                while (j as i32) < n * 5 {
                    if get(out, j) == pid {
                        put(out, j + 4, p as f32);
                        break;
                    }
                    j += 5;
                }
                best = p;
            }
            bo.hit = true;
        }
        if !self.ballistics.targets.is_empty() {
            for &tid in &ids {
                if let Some(&ti) = self.ballistics.targets.get(&tid) {
                    self.with_target(ti, |t, ws| t.remove_body_part_collision(ws));
                }
            }
        }
        n
    }
}

fn placeholder_target() -> PzBallisticsTarget {
    PzBallisticsTarget {
        id: 0,
        adjusting_shape: usize::MAX,
        position: Vec3::ZERO,
        axis: Transform::IDENTITY,
        ragdoll: None,
        collision: usize::MAX,
        body_objects: [None; BODY_PARTS],
    }
}

/// Writes `out[i]` when the output array holds it (the original writes past
/// the end of a short array).
fn put(out: &mut [f32], i: usize, v: f32) {
    if let Some(x) = out.get_mut(i) {
        *x = v;
    }
}
fn get(out: &[f32], i: usize) -> f32 {
    out.get(i).copied().unwrap_or(0.0)
}
