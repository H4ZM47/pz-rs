//! The ballistics natives of `zombie.core.physics.Bullet`.
//!
//! Positions cross the JNI boundary in the game's physics frame: `x` and
//! `z` relative to the world offset (subtracted in `float` on the way in,
//! added back in `double` on the way out), `y` the height. Output arrays
//! are written in place (`ReleaseFloatArrayElements` mode 0); entries the
//! native does not write keep their values. An empty output array throws
//! `IllegalArgumentException("output array is too small")` and the native
//! returns -1.
//!
//! What the game's Java passes and reads back (`BallisticsController`,
//! `BallisticsTarget`, `CombatManager`, `RagdollBuilder`). Positions in
//! the Java calls are iso tiles `(x, y, z level)`, sent as
//! `(x, z * 2.44949, y)`; hit records come back the same way.
//!
//! | method | native | Java passes | Java reads back |
//! |---|---|---|---|
//! | [`PzBullet::update_ballistics`] | `updateBallistics` | shooter id, muzzle `(x, z*2.44949, y)` | — |
//! | [`PzBullet::update_ballistics_aim_reticle_position`] | `updateBallisticsAimReticlePosition` | id, reticle `(isoX, 0, isoY)` | — |
//! | [`PzBullet::update_ballistics_muzzle_aim_direction`] | `updateBallisticsMuzzleAimDirection` | id, normalised muzzle direction `(x, z*2.44949, y)` | — |
//! | [`PzBullet::update_ballistics_aim_reticle_quaternion`] | `updateBallisticsAimReticleQuaternion` | id, camera rotation (JOML `rotationYXZ(pi/4, -pi/6, 0)`) | — |
//! | [`PzBullet::update_ballistics_aim_reticle_rotation`] | `updateBallisticsAimReticleRotation` | id, axis, angle (no Java caller) | — |
//! | [`PzBullet::update_ballistics_aim_reticle_rotate`] | `updateBallisticsAimReticleRotate` | id, quaternion (no Java caller) | — |
//! | [`PzBullet::set_ballistics_size`] / [`PzBullet::set_ballistics_color`] | `setBallisticsSize` / `setBallisticsColor` | id, 0.025 / (0, 0, 1) (debug) | — |
//! | [`PzBullet::set_ballistics_range`] | `setBallisticsRange` | id, weapon max range | — |
//! | [`PzBullet::get_ballistics_targets`] | `getBallisticsTargets` | id, range, 20, `float[80]` | count (0 or 1); 4 floats: id, x, y, z of the hit |
//! | [`PzBullet::get_ballistics_targets_spread_data`] | `getBallisticsTargetsSpreadData` | id, range, spread * scale, weight centre, projectile count, 9, `float[36]` | count (= projectile count); 4 floats per ray: target/vehicle id or 0, x, y, z of the hit or the ray end |
//! | [`PzBullet::get_ballistics_camera_targets`] | `getBallisticsCameraTargets` | id, range + 0.5, 10, parts (true), `float[50]` | count; 5 floats per target: id, x, y, z (the ray's first hit), body part (11 = none) |
//! | [`PzBullet::remove_ballistics`] | `removeBallistics` | id | — (throws without a world) |
//! | [`PzBullet::add_ballistics_target`] | `addBallisticsTarget` | target id | 0 added, 1 already in the world |
//! | [`PzBullet::set_ballistics_target_axis`] | `setBallisticsTargetAxis` | id, `(-pi/2, -pi/2 + animation angle, pi)` | — |
//! | [`PzBullet::update_ballistics_target`] | `updateBallisticsTarget` | id, `(x, z*2.44949, y)`, `(0, sin(f/2), 0, cos(f/2))`, false (only the position is used) | — |
//! | [`PzBullet::update_ballistics_target_skeleton`] | `updateBallisticsTargetSkeleton` | id, bone count (35), 7 floats per `SkeletonBone`: `-pos * 1.5`, rotation `(x, y, z, w)` of the animator's local bone transforms | — |
//! | [`PzBullet::remove_ballistics_target`] | `removeBallisticsTarget` | id | 0 removed / unknown, 1 not in the world |
//! | [`PzBullet::get_targeted_body_part`] | `getTargetedBodyPart` | id | body part 0..10, 11 none |
//! | [`PzBullet::set_ballistics_target_adjusting_shape_scale`] | `setBallisticsTargetAdjustingShapeScale` | (0.35, 0.4, 0.35) | — |
//! | [`PzBullet::set_ballistics_target_all_parts_color`] | `setBallisticsTargetAllPartsColor` | white (debug) | — |

use crate::api::{JniError, PzBullet};
use crate::bt::linear_math::*;
use crate::pz::ballistics::BallisticsStatics;

const NULL_INSTANCE: &str = "WorldSimulation::instance is null";
const ARRAY_TOO_SMALL: &str = "output array is too small";

/// The library's process-wide ballistics state (the ragdoll builder they
/// share is `PzBullet::ragdoll_builder`).
#[derive(Clone, Debug, Default)]
pub struct BallisticsGlobals {
    pub statics: BallisticsStatics,
}

impl PzBullet {
    /// `updateBallistics(id, x, y, z)`: creates the shooter if needed and
    /// moves its muzzle (only while its range is positive).
    pub fn update_ballistics(&mut self, id: i32, x: f32, y: f32, z: f32) {
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        ws.add_ballistics(id, &mut self.ballistics.statics);
        let (ox, oy) = (ws.offset_x as f32, ws.offset_y as f32);
        if let Some(b) = ws.ballistics_mut(id) {
            b.update(x - ox, y, z - oy);
        }
    }

    /// `updateBallisticsMuzzleAimDirection(id, x, y, z)`.
    pub fn update_ballistics_muzzle_aim_direction(&mut self, id: i32, x: f32, y: f32, z: f32) {
        if let Some(b) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            b.update_muzzle_aim_direction(x, y, z);
        }
    }

    /// `setBallisticsSize(id, size)` (debug shapes only).
    pub fn set_ballistics_size(&mut self, id: i32, size: f32) {
        if let Some(b) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            b.size = size as f64;
        }
    }

    /// `setBallisticsColor(id, r, g, b)` (debug colour only).
    pub fn set_ballistics_color(&mut self, id: i32, r: f32, g: f32, b: f32) {
        if let Some(o) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            o.color = Vec3::new(r as f64, g as f64, b as f64);
        }
    }

    /// `setBallisticsRange(id, range)`.
    pub fn set_ballistics_range(&mut self, id: i32, range: f32) {
        if let Some(b) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            b.range = range as f64;
        }
    }

    /// `removeBallistics(id)`.
    pub fn remove_ballistics(&mut self, id: i32) -> Result<(), JniError> {
        let Some(ws) = self.world.as_deref_mut() else {
            return Err(JniError::NullPointer(NULL_INSTANCE));
        };
        ws.remove_ballistics(id);
        Ok(())
    }

    /// `getBallisticsTargets(id, range, count, out)`: the target or vehicle
    /// the muzzle ray hits first; 4 floats (id, x, y, z), returns 0 or 1.
    pub fn get_ballistics_targets(
        &mut self,
        id: i32,
        range: f32,
        count: i32,
        out: &mut [f32],
    ) -> Result<i32, JniError> {
        if out.is_empty() {
            return Err(JniError::IllegalArgument(ARRAY_TOO_SMALL));
        }
        let Some(ws) = self.world.as_deref_mut() else {
            return Ok(0);
        };
        Ok(ws.ballistics_get_targets(id, range, count, out))
    }

    /// `getBallisticsTargetsSpreadData(id, range, spread, weightCenter,
    /// count, max, out)`: `count` random rays (C `rand()`), 4 floats each;
    /// returns `count`.
    #[allow(clippy::too_many_arguments)]
    pub fn get_ballistics_targets_spread_data(
        &mut self,
        id: i32,
        range: f32,
        spread: f32,
        weight_center: f32,
        count: i32,
        max: i32,
        out: &mut [f32],
    ) -> Result<i32, JniError> {
        if out.is_empty() {
            return Err(JniError::IllegalArgument(ARRAY_TOO_SMALL));
        }
        let Some(ws) = self.world.as_deref_mut() else {
            return Ok(0);
        };
        Ok(ws.ballistics_get_spread_target_data(
            id,
            range,
            spread,
            weight_center,
            count,
            max,
            out,
            &mut self.ballistics.statics,
        ))
    }

    /// `getBallisticsCameraTargets(id, range, count, parts, out)`: targets on
    /// the aim reticle's line within `range`; 5 floats each.
    pub fn get_ballistics_camera_targets(
        &mut self,
        id: i32,
        range: f32,
        count: i32,
        parts: bool,
        out: &mut [f32],
    ) -> Result<i32, JniError> {
        if out.is_empty() {
            return Err(JniError::IllegalArgument(ARRAY_TOO_SMALL));
        }
        let Some(ws) = self.world.as_deref_mut() else {
            return Ok(0);
        };
        Ok(ws.ballistics_get_camera_targets(id, range, count, out, parts))
    }

    /// `updateBallisticsAimReticlePosition(id, x, y, z)`.
    pub fn update_ballistics_aim_reticle_position(&mut self, id: i32, x: f32, y: f32, z: f32) {
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        let (ox, oy) = (ws.offset_x as f32, ws.offset_y as f32);
        if let Some(b) = ws.ballistics_mut(id) {
            b.aim.origin = Vec3::new((x - ox) as f64, y as f64, (z - oy) as f64);
        }
    }

    /// `updateBallisticsAimReticleRotation(id, axisX, axisY, axisZ, angle)`.
    pub fn update_ballistics_aim_reticle_rotation(
        &mut self,
        id: i32,
        ax: f32,
        ay: f32,
        az: f32,
        angle: f32,
    ) {
        if let Some(b) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            let q =
                Quat::from_axis_angle(&Vec3::new(ax as f64, ay as f64, az as f64), angle as f64);
            b.update_aim_reticle_rotation(&q);
        }
    }

    /// `updateBallisticsAimReticleQuaternion(id, x, y, z, w)`.
    pub fn update_ballistics_aim_reticle_quaternion(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    ) {
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        if let Some(b) = ws.ballistics_mut(id) {
            let q = Quat::new(x as f64, y as f64, z as f64, w as f64);
            b.update_aim_reticle_quaternion(&q, &mut self.ballistics.statics);
        }
    }

    /// `updateBallisticsAimReticleRotate(id, x, y, z, w)`.
    pub fn update_ballistics_aim_reticle_rotate(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    ) {
        if let Some(b) = self
            .world
            .as_deref_mut()
            .and_then(|ws| ws.ballistics_mut(id))
        {
            b.update_aim_reticle_rotate(&Quat::new(x as f64, y as f64, z as f64, w as f64));
        }
    }

    /// `updateBallisticsTargetSkeleton(id, numberOfBones, boneTransforms)`:
    /// local bone transforms, 7 floats per bone (position, quaternion).
    pub fn update_ballistics_target_skeleton(&mut self, id: i32, count: i32, data: &[f32]) {
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        let idx = ws.ballistics_target_alloc(id, &self.ragdoll_builder);
        ws.with_target(idx, |t, ws| {
            t.update_skeleton_bone_transforms(count, data, ws)
        });
    }

    /// `updateBallisticsTarget(id, x, y, z, qx, qy, qz, qw, b)`: moves the
    /// target and refits its box to the body parts.
    #[allow(clippy::too_many_arguments)]
    pub fn update_ballistics_target(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        _qx: f32,
        _qy: f32,
        _qz: f32,
        _qw: f32,
        _b: bool,
    ) -> Result<(), JniError> {
        let Some(ws) = self.world.as_deref_mut() else {
            return Err(JniError::NullPointer(NULL_INSTANCE));
        };
        let (ox, oy) = (ws.offset_x as f32, ws.offset_y as f32);
        let b = &self.ragdoll_builder;
        let st = &self.ballistics.statics;
        let idx = ws.ballistics_target_alloc(id, b);
        ws.with_target(idx, |t, ws| t.update(x - ox, y, z - oy, b, ws, st));
        Ok(())
    }

    /// `setBallisticsTargetAxis(id, x, y, z)`.
    pub fn set_ballistics_target_axis(&mut self, id: i32, x: f32, y: f32, z: f32) {
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        if let Some(&idx) = ws.ballistics.targets.get(&id) {
            ws.ballistics.target_objects[idx].set_axis(x, y, z);
        }
    }

    /// `addBallisticsTarget(id)`: 1 when already in the world, else 0.
    pub fn add_ballistics_target(&mut self, id: i32) -> Result<i32, JniError> {
        let Some(ws) = self.world.as_deref_mut() else {
            return Err(JniError::NullPointer(NULL_INSTANCE));
        };
        let idx = ws.ballistics_target_alloc(id, &self.ragdoll_builder);
        Ok(ws.ballistics_target_add_to_world(idx))
    }

    /// `removeBallisticsTarget(id)`: 0 when removed (or unknown), 1 when the
    /// target was not in the world; the target goes back to the pool.
    pub fn remove_ballistics_target(&mut self, id: i32) -> i32 {
        let Some(ws) = self.world.as_deref_mut() else {
            return 0;
        };
        ws.remove_ballistics_target(id)
    }

    /// `getTargetedBodyPart(id)`: 0..10, 11 when none (or unknown target).
    pub fn get_targeted_body_part(&mut self, id: i32) -> i32 {
        let Some(ws) = self.world.as_deref() else {
            return 11;
        };
        match ws.ballistics.targets.get(&id) {
            Some(&idx) => ws.ballistics.target_objects[idx].get_targeted_body_part(),
            None => 11,
        }
    }

    /// `setBallisticsTargetAdjustingShapeScale(x, y, z)`.
    pub fn set_ballistics_target_adjusting_shape_scale(&mut self, x: f32, y: f32, z: f32) {
        self.ballistics.statics.extents_scale = Vec3::new(x as f64, y as f64, z as f64);
    }

    /// `setBallisticsTargetAllPartsColor(r, g, b)` (debug colour).
    pub fn set_ballistics_target_all_parts_color(&mut self, r: f32, g: f32, b: f32) {
        self.ballistics.statics.all_parts_color = Vec3::new(r as f64, g as f64, b as f64);
    }
}
