//! A character in the iso view for one frame: `FBORenderCell.renderPlayer`
//! / `renderMovingObject` for an `IsoGameCharacter` — its drop shadow
//! (`IsoGameCharacter.renderShadow`, `calculateShadowParams`) and its
//! model (`IsoSprite.renderActiveModel` → `ModelSlotRenderData`,
//! [`crate::render::render_character`]) — as a
//! [`pz_iso_render::objects::ObjectRender`] for the frame's object pass.

use pz_anim::AnimationPlayer;
use pz_iso_render::models::ModelCamera;
use pz_iso_render::objects::{ObjectRender, ShadowRequest};
use pz_iso_render::shadows::{ShadowParams, SlopedSurface};
use pz_render::DrawCmd;

use crate::model_tree::ModelSlot;
use crate::render::{render_character, CharacterFrame, ModelResources};

/// `Model.vectorToWorldCoords(x, y, z, angle, vec)`: a model-space vector
/// of a character at `(x, y, z)` drawn at `angle` in world coordinates.
pub fn vector_to_world(x: f32, y: f32, z: f32, angle: f32, v: [f32; 3]) -> [f32; 3] {
    // Vector3.rotatey after mirroring x.
    let (vx, vz) = (-v[0] as f64, v[2] as f64);
    let a = angle as f64;
    let rx = (vx * a.cos() - vz * a.sin()) as f32;
    let rz = (vx * a.sin() + vz * a.cos()) as f32;
    // y and z swap: model up is world z.
    [rx * 1.5 + x, rz * 1.5 + y, v[1] * 0.612_372_34 + z]
}

/// `Model.boneToWorldCoords(animPlayer, x, y, z, animalSize, boneIndex)`.
pub fn bone_to_world(
    player: &AnimationPlayer,
    x: f32,
    y: f32,
    z: f32,
    animal_size: f32,
    bone: i32,
) -> [f32; 3] {
    // getModelTransformAt(-1) is out of range in the original too; every
    // body has the bones asked for.
    let m = player.model_transform(bone.max(0) as usize);
    let v = [
        m.m03 * animal_size,
        m.m13 * animal_size,
        m.m23 * animal_size,
    ];
    vector_to_world(x, y, z, player.rendered_angle(), v)
}

/// `IsoGameCharacter.closestpointonline` (in doubles).
fn closest_point_on_line(lx1: f64, ly1: f64, lx2: f64, ly2: f64, x0: f64, y0: f64) -> [f32; 2] {
    let a1 = ly2 - ly1;
    let b1 = lx1 - lx2;
    let c1 = (ly2 - ly1) * lx1 + (lx1 - lx2) * ly1;
    let c2 = -b1 * x0 + a1 * y0;
    let det = a1 * a1 - -b1 * b1;
    if det != 0.0 {
        [
            ((a1 * c1 - b1 * c2) / det) as f32,
            ((a1 * c2 - -b1 * c1) / det) as f32,
        ]
    } else {
        [x0 as f32, y0 as f32]
    }
}

/// `IsoGameCharacter.calculateShadowParams(animationPlayer, animalSize,
/// bRagdoll)`: the shadow reaches 0.35 (times 1.35) past the head and feet
/// projected on the facing line, forward and back. `None` for the player
/// is a character without a ready animation player.
pub fn shadow_params(
    player: Option<&AnimationPlayer>,
    animal_size: f32,
    ragdoll: bool,
) -> ShadowParams {
    let mut sp = ShadowParams::default();
    let Some(p) = player else { return sp };
    let bone = |name: &str| {
        bone_to_world(
            p,
            0.0,
            0.0,
            0.0,
            animal_size,
            p.skinning_bone_index(name, -1),
        )
    };
    let mut points = [
        bone("Bip01_Head"),
        bone("Bip01_L_Foot"),
        bone("Bip01_R_Foot"),
    ];
    if ragdoll {
        let pelvis = bone("Bip01_Pelvis");
        for q in &mut points {
            q[0] -= pelvis[0];
            q[1] -= pelvis[1];
        }
    }
    let a = p.angle() as f64;
    let forward = [(a.cos()) as f32, (a.sin()) as f32];
    let (mut f_len, mut b_len) = (0.0f32, 0.0f32);
    for q in points {
        let c = closest_point_on_line(
            0.0,
            0.0,
            forward[0] as f64,
            forward[1] as f64,
            q[0] as f64,
            q[1] as f64,
        );
        let len = (c[0] * c[0] + c[1] * c[1]).sqrt();
        if len > 0.001 {
            let n = [c[0] / len, c[1] / len];
            if forward[0] * n[0] + forward[1] * n[1] > 0.0 {
                f_len = f_len.max(len);
            } else {
                b_len = b_len.max(len);
            }
        }
    }
    sp.fm = (f_len + 0.35) * 1.35;
    sp.bm = (b_len + 0.35) * 1.35;
    sp
}

/// The shadow lengths a character eases towards (`shadowFm`, `shadowBm`,
/// `shadowTick`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadowSmoothing {
    pub fm: f32,
    pub bm: f32,
    pub tick: i64,
}

impl ShadowSmoothing {
    /// `renderShadow`'s easing on tick `tick`: restarts from `params` after
    /// a gap, else moves 10% (at 30 fps) towards it.
    pub fn ease(
        &mut self,
        params: ShadowParams,
        tick: i64,
        thirty_fps_multiplier: f32,
    ) -> ShadowParams {
        let mult = (0.1 * thirty_fps_multiplier).clamp(0.0, 1.0);
        if self.tick != tick - 1 {
            self.fm = params.fm;
            self.bm = params.bm;
        }
        self.tick = tick;
        self.fm += (params.fm - self.fm) * mult;
        self.bm += (params.bm - self.bm) * mult;
        ShadowParams {
            w: params.w,
            fm: self.fm,
            bm: self.bm,
        }
    }
}

/// What `renderShadow` reads from the character.
pub struct ShadowInput<'a> {
    /// The animation player when the character has an active, ready model.
    pub player: Option<&'a AnimationPlayer>,
    /// `getAnimAngleRadians()`: the facing the shadow points along.
    pub anim_angle: f32,
    /// `ragdollStateData.simulationDirection` while ragdolling.
    pub ragdoll_direction: Option<[f32; 2]>,
    /// `getHeightAboveFloor()`.
    pub height_above_floor: f32,
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
    /// Skeleton zombies cast half a shadow.
    pub skeleton: bool,
    /// `IsoAnimal.getAnimalSize()` for animals.
    pub animal_size: Option<f32>,
    /// The current square's light colour (`lighting[playerIndex].lightInfo()`).
    pub light: [f32; 3],
    /// The current square's slope, if any.
    pub slope: Option<SlopedSurface>,
    /// Hidden local player (`isDisplayPlayerModel` off), alpha and target
    /// zero, seated in a vehicle, or no current square: no shadow.
    pub hidden: bool,
}

/// `IsoGameCharacter.renderShadow(x, y, z)`: the shadow request, or `None`
/// when the character casts none (hidden, off the floor by more than half
/// a level). `smoothing` is the character's eased lengths, advanced when
/// it has a ready model.
pub fn character_shadow(
    input: &ShadowInput,
    pos: [f32; 3],
    smoothing: Option<(&mut ShadowSmoothing, i64, f32)>,
) -> Option<ShadowRequest> {
    if input.hidden {
        return None;
    }
    let height = input.height_above_floor;
    if height > 0.5 {
        return None;
    }
    let animal_size = input.animal_size.unwrap_or(1.0);
    let mut params = shadow_params(input.player, animal_size, false);
    let mut alpha = input.alpha;
    if input.skeleton {
        alpha *= 0.5;
    }
    if height > 0.0 {
        alpha *= 1.0 - height / 0.5;
    }
    if input.player.is_some() {
        if let Some((s, tick, mult)) = smoothing {
            params = s.ease(params, tick, mult);
        }
    }
    let a = input.anim_angle as f64;
    let forward = input
        .ragdoll_direction
        .unwrap_or([a.cos() as f32, a.sin() as f32]);
    Some(ShadowRequest::Character {
        pos: [pos[0], pos[1], pos[2] - height],
        forward,
        params,
        rgb: input.light,
        alpha,
        animal: input.animal_size.is_some(),
        slope: input.slope,
    })
}

/// One character's frame: its shadow request and model draws, in the
/// order `renderPlayer` / `renderMovingObject` issue them.
pub fn render_character_object(
    slot: &ModelSlot,
    frame: &CharacterFrame,
    shadow: Option<ShadowRequest>,
    camera: &ModelCamera,
    res: &dyn ModelResources,
) -> ObjectRender {
    let draws = render_character(slot, frame, camera, res)
        .into_iter()
        .map(|d| DrawCmd::Model(Box::new(d)))
        .collect();
    ObjectRender { shadow, draws }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_turn_with_the_character() {
        // Model +z (forward) at angle 0 is world +y, scaled by 1.5.
        let v = vector_to_world(10.0, 20.0, 0.0, 0.0, [0.0, 0.0, 1.0]);
        assert!((v[0] - 10.0).abs() < 1e-6 && (v[1] - 21.5).abs() < 1e-6);
        // Model up is world z, scaled to levels.
        let up = vector_to_world(0.0, 0.0, 1.0, 0.7, [0.0, 2.0, 0.0]);
        assert!((up[2] - (1.0 + 2.0 * 0.612_372_34)).abs() < 1e-6);
        // Model +x is mirrored.
        let x = vector_to_world(0.0, 0.0, 0.0, 0.0, [1.0, 0.0, 0.0]);
        assert!((x[0] + 1.5).abs() < 1e-6);
    }

    #[test]
    fn closest_point_projects_onto_the_line() {
        let c = closest_point_on_line(0.0, 0.0, 1.0, 0.0, 0.4, 0.9);
        assert!((c[0] - 0.4).abs() < 1e-6 && c[1].abs() < 1e-6);
    }

    #[test]
    fn shadow_lengths_ease() {
        let mut s = ShadowSmoothing::default();
        let p = ShadowParams {
            w: 0.45,
            fm: 1.0,
            bm: 0.5,
        };
        assert_eq!(s.ease(p, 10, 1.0), p);
        let q = s.ease(
            ShadowParams {
                w: 0.45,
                fm: 2.0,
                bm: 0.5,
            },
            11,
            1.0,
        );
        assert!((q.fm - 1.1).abs() < 1e-6);
        // A skipped tick starts over.
        assert_eq!(s.ease(p, 20, 1.0), p);
    }

    #[test]
    fn shadow_rules() {
        let input = ShadowInput {
            player: None,
            anim_angle: 0.0,
            ragdoll_direction: None,
            height_above_floor: 0.25,
            alpha: 1.0,
            skeleton: true,
            animal_size: None,
            light: [1.0; 3],
            slope: None,
            hidden: false,
        };
        let Some(ShadowRequest::Character {
            pos,
            alpha,
            params,
            forward,
            ..
        }) = character_shadow(&input, [1.0, 2.0, 0.5], None)
        else {
            panic!()
        };
        assert_eq!(pos, [1.0, 2.0, 0.25]);
        assert!((alpha - 0.25).abs() < 1e-6);
        assert_eq!(params, ShadowParams::default());
        assert_eq!(forward, [1.0, 0.0]);
        assert!(character_shadow(
            &ShadowInput {
                height_above_floor: 0.6,
                ..input
            },
            [0.0; 3],
            None
        )
        .is_none());
    }
}
