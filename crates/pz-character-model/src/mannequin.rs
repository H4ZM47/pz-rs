//! Mannequins (`IsoMannequin`) in the build-42 renderer: posed once from
//! their script's anim set, state and `Pose` node (`Female` and `Pose`
//! variables), drawn as a 3D model straight into their chunk texture while
//! it is cached (`FBORenderCorpses.render(renderChunkIndex, IsoMannequin)`
//! with `FBORenderChunkCamera`, back faces culled), with a drop shadow
//! every frame (`renderShadow`) and, for outlines, a `DeadBodyAtlas` entry
//! (`getAtlasTexture` → `BodyParams.init(IsoMannequin)`, standing).

use std::rc::Rc;

use pz_anim::{AdvancedAnimator, AnimationPlayer, AnimationSet, MapVariableSource, VariableValue};
use pz_iso_render::chunk::ChunkTexture;
use pz_iso_render::chunk_camera::ChunkCamera;
use pz_iso_render::models::encode_light;
use pz_iso_render::objects::{ChunkModels, ShadowRequest};
use pz_iso_render::shadows::ShadowParams;
use pz_render::model::{ModelLight, ModelPass};
use pz_render::DrawCmd;

use crate::corpse::DeadBody;
use crate::model_tree::ModelSlot;
use crate::render::{render_slot, ModelResources, SlotUniforms};

/// What the mannequin render paths read from an `IsoMannequin`.
#[derive(Clone, Debug)]
pub struct Mannequin {
    /// The square.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `getForwardIsoDirection().ordinal()` (N, NW, W, SW, S, SE, E, NE).
    pub dir: u8,
    pub female: bool,
    /// The script's `animSet`, `animState` and `pose` (`validatePose`d).
    pub anim_set: String,
    pub anim_state: String,
    pub pose: String,
    /// `getSurfaceOffset()` of each table surface on the square before
    /// the mannequin in its object list, and of every one on the square.
    pub surfaces_below: Vec<f32>,
    pub surfaces_all: Vec<f32>,
    pub outside: bool,
    pub room: bool,
    /// `initAmbient`: the square's light interpolated at the mannequin.
    pub ambient: [f32; 3],
    /// The highlight colour when highlighted, else white.
    pub tint: [f32; 3],
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
}

/// `IsoDirections.ToVector().getDirection()`.
pub fn direction_angle(dir: u8) -> f32 {
    const D: [(f32, f32); 8] = [
        (0.0, -1.0),
        (-1.0, -1.0),
        (-1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 1.0),
        (1.0, 1.0),
        (1.0, 0.0),
        (1.0, -1.0),
    ];
    let (x, y) = D[(dir & 7) as usize];
    let l = (x * x + y * y).sqrt();
    (y / l).atan2(x / l)
}

impl Mannequin {
    /// `BodyParams.init(IsoMannequin)`'s angle.
    pub fn angle(&self) -> f32 {
        direction_angle(self.dir)
    }

    /// `FBORenderCorpses.BodyParams.init(IsoMannequin)`'s position: the
    /// square's centre, raised by `(surfaceOffset + 1) / 96` for each
    /// table surface below it.
    pub fn model_position(&self) -> [f32; 3] {
        let lift: f32 = self.surfaces_below.iter().map(|s| (s + 1.0) / 96.0).sum();
        [
            self.x as f32 + 0.5,
            self.y as f32 + 0.5,
            self.z as f32 + lift,
        ]
    }

    /// The `DeadBody`-shaped parameters `DeadBodyAtlas` renders the
    /// mannequin's atlas entry with (no square lights, white ambient).
    pub fn atlas_body(&self) -> DeadBody {
        DeadBody {
            x: self.x as f32,
            y: self.y as f32,
            z: self.z as f32,
            angle: self.angle(),
            dir: self.dir,
            female: self.female,
            outside: self.outside,
            room: self.room,
            alpha: 1.0,
            animal_size: 1.0,
            ..Default::default()
        }
    }

    /// `renderShadow`: a fixed-size shadow facing x, raised by
    /// `(surfaceOffset + 2) / 96` per table surface on the square, alpha
    /// 0.8. `light` is the square's `lightInfo()` (r, g, b, a); the
    /// original passes its alpha as the blue channel.
    pub fn shadow(&self, light: [f32; 4]) -> ShadowRequest {
        let lift: f32 = self.surfaces_all.iter().map(|s| (s + 2.0) / 96.0).sum();
        ShadowRequest::Character {
            pos: [
                self.x as f32 + 0.5,
                self.y as f32 + 0.5,
                self.z as f32 + lift,
            ],
            forward: [1.0, 0.0],
            params: ShadowParams {
                w: 0.45,
                fm: 0.4725,
                bm: 0.5611,
            },
            rgb: [light[0], light[1], light[3]],
            alpha: 0.8 * self.alpha,
            animal: false,
            slope: None,
        }
    }
}

/// The render job's `AnimatedModel`: `setAngle(dir.ToVector())`, the
/// script's anim set and state with the `Female` and `Pose` variables, the
/// blends finished at once (fps multiplier 100), then
/// `AnimationPlayer.Update(0)`.
pub fn pose(player: &mut AnimationPlayer, set: Rc<AnimationSet>, m: &Mannequin) {
    let a = m.angle() as f64;
    player.set_target_and_current_direction(a.cos() as f32, a.sin() as f32);
    let mut vars = MapVariableSource::new();
    vars.insert_slot("Female", VariableValue::Bool(m.female));
    vars.insert_slot("Pose", VariableValue::String(m.pose.clone()));
    let mut animator = AdvancedAnimator::new();
    animator.set_anim_set(Some(set));
    animator.set_state(player, &mut vars, &m.anim_state, &[]);
    animator.update(player, &mut vars, 100.0);
    animator.update(player, &mut vars, 100.0);
    player.update(0.0, Some(&vars));
}

/// `DrawCharSingular`'s lights with no square lights (`initLights(null)`):
/// the fill lights from the ambient outside (3 and 4) or in a room (4).
fn fill_lights(m: &Mannequin, origin: [f32; 3], angle: f32) -> [ModelLight; 5] {
    let none = ModelLight {
        direction: [0.0, 1.0, 0.0],
        colour: [0.0; 3],
    };
    let mut l = [none; 5];
    let fill = m.ambient.map(|c| c * 1.7 / 4.0);
    let [x, y, z] = origin;
    if m.outside {
        l[3] = encode_light(
            [x - 2.0, y - 2.0, z + 1.0],
            fill,
            5000.0,
            origin,
            Some(angle),
        );
        l[4] = encode_light(
            [x + 2.0, y + 2.0, z + 1.0],
            fill,
            5000.0,
            origin,
            Some(angle),
        );
    } else if m.room {
        l[4] = encode_light(
            [x + 2.0, y + 2.0, z + 1.0],
            fill,
            5000.0,
            origin,
            Some(angle),
        );
    }
    l
}

/// `FBORenderCorpses.RenderJob.render` for a mannequin: its meshes drawn
/// into `texture` (the chunk texture of its square's level pair) through
/// `FBORenderChunkCamera`, with the target depth of
/// `ChunkCamera.Begin` for a camera character on `cam`. The draws go into
/// [`pz_iso_render::objects::FrameObjects::chunk_models`] the frame the
/// chunk texture is (re)cached.
pub fn chunk_models(
    m: &Mannequin,
    slot: &ModelSlot,
    player: &AnimationPlayer,
    res: &dyn ModelResources,
    texture: ChunkTexture,
    cam: [f32; 2],
) -> ChunkModels {
    let cc = ChunkCamera::new(texture);
    let [x, y, z] = m.model_position();
    let angle = player.rendered_angle();
    let p = cc.projection();
    let mv = cc.model_view(x, y, z, angle);
    let mvp = p.mul(&mv);
    let u = SlotUniforms {
        mvp: mvp.m,
        target_depth: cc.target_depth(cam[0], cam[1], x, y, z, &mvp),
        lights: fill_lights(m, [x, y, z], angle),
        ambient: m.ambient.map(|c| c * 0.45),
        tint: m.tint,
        alpha: 1.0,
        final_scale: 1.0,
    };
    let mut draws = vec![
        DrawCmd::DepthMask(true),
        DrawCmd::ColorMask(true, true, true, true),
    ];
    draws.extend(render_slot(slot, player, &u, res).into_iter().map(|mut d| {
        // AnimatedModel.StartCharacter's blending, setCullFace(GL_BACK).
        d.pass = ModelPass::AnimatedModelCullBack;
        DrawCmd::Model(Box::new(d))
    }));
    ChunkModels { texture, draws }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_and_surfaces() {
        // S faces +y, E faces +x.
        assert!((direction_angle(4) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(direction_angle(6).abs() < 1e-6);
        let m = Mannequin {
            x: 10,
            y: 20,
            z: 0,
            dir: 4,
            female: true,
            anim_set: "mannequin".into(),
            anim_state: "female".into(),
            pose: "pose01".into(),
            surfaces_below: vec![30.0],
            surfaces_all: vec![30.0, 10.0],
            outside: false,
            room: true,
            ambient: [1.0; 3],
            tint: [1.0; 3],
            alpha: 1.0,
        };
        assert_eq!(m.model_position(), [10.5, 20.5, 31.0 / 96.0]);
        let ShadowRequest::Character {
            pos,
            params,
            alpha,
            rgb,
            ..
        } = m.shadow([0.2, 0.4, 0.6, 0.9])
        else {
            unreachable!()
        };
        assert_eq!(pos[2], 32.0 / 96.0 + 12.0 / 96.0);
        assert_eq!((params.w, params.fm, params.bm), (0.45, 0.4725, 0.5611));
        assert_eq!(alpha, 0.8);
        assert_eq!(rgb, [0.2, 0.4, 0.9]);
    }
}
