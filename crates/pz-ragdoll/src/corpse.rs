//! A ragdolled body's final pose: what `IsoDeadBody` freezes from the dying
//! character (`diedBoneTransforms`), and how `DeadBodyAtlas`'s render job
//! poses its own `AnimatedModel` from it (`setDeathPose`).

use pz_anim::bone_transform::BoneTransform;
use pz_anim::AnimationPlayer;

/// `IsoGameCharacter.canUseCurrentPoseForCorpse()` on a client.
pub fn can_use_current_pose_for_corpse(
    player: &AnimationPlayer,
    scene_culled: bool,
    has_active_model: bool,
) -> bool {
    if scene_culled || !has_active_model {
        return false;
    }
    if player.is_ragdoll_simulation_active() {
        return true;
    }
    !player.bone_transforms_need_first_frame()
}

/// `IsoDeadBody`'s constructor: with `ragdollFall` and a usable pose, every
/// bone's local transform (`getBoneTransformAt`); otherwise none.
pub fn died_bone_transforms(
    player: &AnimationPlayer,
    ragdoll_fall: bool,
    can_use_current_pose: bool,
) -> Vec<BoneTransform> {
    if !(ragdoll_fall && can_use_current_pose) {
        return Vec::new();
    }
    (0..player.num_bones())
        .map(|i| player.bone_transform_at(i))
        .collect()
}

/// `RenderJob.setDeathPose()`: drops the render job's tracks and plays the
/// ragdoll clip (`DeathPose`) at full weight, every keyframe set to the
/// frozen pose. Nothing happens without a pose, or for a pose with a
/// different bone count.
pub fn set_death_pose(player: &mut AnimationPlayer, died: &[BoneTransform]) {
    if died.is_empty() || player.is_bone_count_mismatched(died.len()) {
        return;
    }
    player.stop_all();
    if let Some(id) = player.play("DeathPose", true, true, -1.0) {
        if let Some(t) = player.track_mut(id) {
            t.set_blend_weight(1.0);
        }
        let mut bones = died.to_vec();
        player.init_ragdoll_transforms(id, &mut bones);
    }
}

/// `RenderJob.useRagdollPose()`.
pub fn use_ragdoll_pose(
    ragdoll_fall: bool,
    died: &[BoneTransform],
    player: &AnimationPlayer,
) -> bool {
    ragdoll_fall && !died.is_empty() && !player.is_bone_count_mismatched(died.len())
}

/// `IsoDeadBody.load`'s `diedBoneTransforms`: `count` identity bones, then
/// each saved `(boneID, [px, py, pz, qx, qy, qz, qw, sx, sy, sz])` set
/// (ids out of range are skipped).
pub fn died_bone_transforms_from_saved(
    count: usize,
    saved: &[(i32, [f32; 10])],
) -> Vec<BoneTransform> {
    use pz_anim::math::{Quaternion, Vector3f};
    let mut out = vec![BoneTransform::default(); count];
    for (id, v) in saved {
        if *id < 0 || *id as usize >= count {
            continue;
        }
        out[*id as usize].set_prs(
            Vector3f::new(v[0], v[1], v[2]),
            Quaternion::new(v[3], v[4], v[5], v[6]),
            Vector3f::new(v[7], v[8], v[9]),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_pose_skips_bad_ids() {
        let v = [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let mut d = died_bone_transforms_from_saved(3, &[(1, v), (7, v), (-1, v)]);
        assert_eq!(d[1].position(), pz_anim::math::Vector3f::new(1.0, 2.0, 3.0));
        assert_eq!(d[0], BoneTransform::default());
        assert_eq!(d[2].position(), pz_anim::math::Vector3f::ZERO);
    }
}
