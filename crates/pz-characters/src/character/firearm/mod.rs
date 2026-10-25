//! Ranged combat and ragdolls: the firearm parts of `zombie.CombatManager`
//! (hit chance, ballistics target selection, shot resolution, damage,
//! hit reactions by body part), `BallisticsController` /
//! `BallisticsTarget`, the firearm side of `HandWeapon` (perk-scaled
//! ranges, recoil, jams, ammunition), `RagdollBuilder` /
//! `RagdollController`, and `CombatManager.processHit` (used by melee
//! hits on zombies too).
//!
//! The physics is reached through the [`PhysicsWorld`] trait (the Bullet
//! natives the game calls); with the `bullet` feature, `pz_bullet`'s
//! `PzBullet` implements it. The world owns the physics world and hands it
//! out with `StateWorld::physics`; the animation inputs (muzzle attachment,
//! reticle, bone transforms, the ragdoll clip) come from the other
//! `StateWorld` hooks.

pub mod ballistics;
#[cfg(feature = "bullet")]
pub mod bullet;
pub mod hit_chance;
pub mod physics;
pub mod ragdoll;
pub mod ranged;
pub mod reaction;
pub mod weapon;

#[cfg(test)]
mod tests;

pub use ballistics::{BallisticsController, BallisticsTarget, CombatDamageData, MuzzleAttachment, TargetSkeleton};
pub use hit_chance::{HitChanceData, TargetInfo, VehicleAim};
pub use physics::{PhysicsWorld, RagdollBodyPart, BULLET_Z_SCALE, SKELETON_BONE_COUNT};
pub use ragdoll::{RagdollAnim, RagdollBone, RagdollBuilder, RagdollController, RagdollSkeletonData, RagdollStateData};
pub use reaction::{HitReaction, HitWeapon, ShotDirection};
pub use weapon::FirearmStats;

/// `LosUtil.getFirstBlockingIsoGridSquare(...)`: the line's result, the
/// blocking square and the point where the line meets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockingSquare {
    pub result: crate::character::state::TestResults,
    pub square: Option<(i32, i32, i32)>,
    /// `hitPosition` (x, y; the tracer keeps the ray's z).
    pub position: (f32, f32),
}

impl Default for BlockingSquare {
    fn default() -> Self {
        BlockingSquare { result: crate::character::state::TestResults::Clear, square: None, position: (0.0, 0.0) }
    }
}

/// `CombatManager`'s mutable statics and instance fields, and the other
/// singletons ranged combat shares: kept in [`crate::Characters::combat`].
#[derive(Clone, Debug, PartialEq)]
pub struct CombatManagerState {
    /// `ballisticsStartPosition`, `ballisticsEndPosition`,
    /// `ballisticsDirectionVector`.
    pub ballistics_start_position: [f32; 3],
    pub ballistics_end_position: [f32; 3],
    pub ballistics_direction_vector: [f32; 3],
    /// `ballisticsSpreadAngleScale`.
    pub ballistics_spread_angle_scale: f32,
    /// `tempVector4f`: its `w` (the last nearest-target dot) is what a
    /// ranged hit info keeps as its `dot`.
    pub temp_vector4f: [f32; 4],
    /// `dotList` (piercing shots).
    pub dot_list: Vec<f32>,
    /// `RagdollController.numberOfActiveSimulations`.
    pub active_ragdolls: i32,
    /// `RagdollBuilder.instance`.
    pub ragdoll_builder: RagdollBuilder,
}

impl Default for CombatManagerState {
    fn default() -> Self {
        CombatManagerState {
            ballistics_start_position: [0.0; 3],
            ballistics_end_position: [0.0; 3],
            ballistics_direction_vector: [0.0; 3],
            ballistics_spread_angle_scale: 1.0,
            temp_vector4f: [0.0; 4],
            dot_list: Vec::new(),
            active_ragdolls: 0,
            ragdoll_builder: RagdollBuilder::default(),
        }
    }
}
