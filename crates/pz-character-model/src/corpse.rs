//! Dead bodies: `IsoDeadBody.render` / `renderShadow` through
//! `DeadBodyAtlas` (`BodyParams.init(IsoDeadBody)`, the render job's
//! `AnimatedModel` pose and bitmap render, `calculateShadowParams`), with
//! the atlas math and draws in [`pz_iso_render::body_atlas`].
//!
//! Bodies use the death animation's pose: the `zombie` anim set's
//! `onground` state (or the player set's `deadbody`) at track time 0 (1
//! when killed by a fall), with `FallOnFront`/`KilledByFall` set. Ragdoll
//! death poses (`diedBoneTransforms`) are not ported.

use std::collections::HashMap;
use std::rc::Rc;

use pz_anim::{AdvancedAnimator, AnimationPlayer, AnimationSet, MapVariableSource, VariableValue};
use pz_iso_render::atlas::{AtlasEntry, Atlases, Checksummer};
use pz_iso_render::body_atlas::{self, ATLAS_SIZE, BITMAP_SIZE};
use pz_iso_render::models::encode_light;
use pz_iso_render::objects::ShadowRequest;
use pz_iso_render::shadows::ShadowParams;
use pz_iso_render::Camera;
use pz_render::model::{ModelDraw, ModelLight, ModelPass};
use pz_render::DrawCmd;

use crate::model_tree::ModelSlot;
use crate::render::{render_slot, ModelResources, SlotUniforms};
use crate::scene::shadow_params;

/// A square's result light (`IsoGridSquare.ResultLight`) as the body
/// copies it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointLight {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub rgb: [f32; 3],
    pub radius: i32,
}

/// What `BodyParams.init(IsoDeadBody)` and the render paths read from a
/// dead body.
#[derive(Clone, Debug, Default)]
pub struct DeadBody {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getAngle()`.
    pub angle: f32,
    /// `getForwardIsoDirection().ordinal()`.
    pub dir: u8,
    pub female: bool,
    pub zombie: bool,
    pub skeleton: bool,
    pub animal: bool,
    pub fall_on_front: bool,
    pub killed_by_fall: bool,
    pub being_grappled: bool,
    /// The hand items' static models (`getStaticModel()`), when held.
    pub primary_hand: Option<String>,
    pub secondary_hand: Option<String>,
    /// Whether any hand item is held (even without a model).
    pub has_hand_item: bool,
    pub outside: bool,
    pub room: bool,
    /// `square.lighting[0]`'s result lights.
    pub lights: Vec<PointLight>,
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
    /// `IsoAnimal` size (1 for people).
    pub animal_size: f32,
    /// `animalAnimSet`: an animal's anim set (`pig`, `cow`, ...).
    pub animal_anim_set: Option<String>,
}

impl DeadBody {
    /// `BodyParams.init(IsoDeadBody)`'s anim set, state and track time.
    pub fn anim_state(&self) -> (&str, &'static str, f32) {
        let mut set = "zombie";
        let mut state = if self.being_grappled {
            "grappled"
        } else {
            "onground"
        };
        let mut track_time = 0.0;
        if self.has_hand_item {
            set = "player";
            state = "deadbody";
            if self.killed_by_fall {
                track_time = 1.0;
            }
        } else if !self.animal && !self.zombie && self.killed_by_fall {
            set = "player";
            state = "deadbody";
            track_time = 1.0;
        }
        if self.animal {
            set = self.animal_anim_set.as_deref().unwrap_or("");
            state = "deadbody";
        }
        (set, state, track_time)
    }
}

/// An attached model in a body key: self, parent, model, blood level.
pub type AttachedKey = (Option<String>, Option<String>, Option<String>, f32);
/// A clothing item in a body key: base texture, texture choice, tint,
/// model, blood.
pub type ClothingKey = (
    Option<String>,
    Option<String>,
    [f32; 3],
    Option<String>,
    f32,
);

/// The visual part of `getBodyKey` (`HumanVisual` and clothing), filled by
/// the caller.
#[derive(Clone, Debug, Default)]
pub struct BodyVisualKey {
    pub hair_model: Option<String>,
    pub beard_model: Option<String>,
    pub skin_color: [f32; 3],
    pub skin_texture: Option<String>,
    pub total_blood: f32,
    /// Attached models: self, parent, model, blood level.
    pub attached: Vec<AttachedKey>,
    /// Clothing: base texture, texture choice, tint, model, blood.
    pub clothing: Vec<ClothingKey>,
}

/// `getBodyKey` for a human body.
pub fn body_key(b: &DeadBody, v: &BodyVisualKey) -> String {
    let (set, state, track_time) = b.anim_state();
    posed_body_key(b, v, set, state, track_time, false)
}

/// `getBodyKey` of `BodyParams` posed with `set`/`state` at `track_time`,
/// standing or not (`init(IsoMannequin)` stands).
pub fn posed_body_key(b: &DeadBody, v: &BodyVisualKey, set: &str, state: &str, track_time: f32, standing: bool) -> String {
    let mut c = Checksummer::default();
    c.byte(b.dir);
    c.int(
        (pz_anim::math::pzmath::wrap_range(b.angle, 0.0, std::f32::consts::PI * 2.0) * 57.295_776)
            as i32,
    );
    c.str(v.hair_model.as_deref());
    c.str(v.beard_model.as_deref());
    let col = |c: &mut Checksummer, rgb: [f32; 3]| rgb.iter().for_each(|&x| c.unit(x));
    col(&mut c, v.skin_color);
    c.str(v.skin_texture.as_deref());
    c.int((v.total_blood * 100.0) as i32);
    c.str(b.primary_hand.as_deref());
    c.str(b.secondary_hand.as_deref());
    for (s, p, m, blood) in &v.attached {
        c.str(s.as_deref());
        c.str(p.as_deref());
        c.str(m.as_deref());
        c.int((blood * 100.0) as i32);
    }
    c.bool(b.female);
    c.bool(b.zombie);
    c.bool(b.skeleton);
    c.str(Some(set));
    c.str(Some(state));
    for (base, choice, tint, model, blood) in &v.clothing {
        c.str(base.as_deref());
        c.str(choice.as_deref());
        col(&mut c, *tint);
        c.str(model.as_deref());
        c.int((blood * 100.0) as i32);
    }
    c.bool(b.fall_on_front);
    c.bool(standing);
    c.bool(b.outside);
    c.bool(b.room);
    // initAmbient: always white for the atlas.
    for _ in 0..3 {
        c.unit(((1.0f32 * 10.0) as i32) as f32 / 10.0);
    }
    c.int(track_time as i32);
    for l in &b.lights {
        if l.radius <= 0 {
            continue;
        }
        c.int((l.x as f32 - b.x) as i32);
        c.int((l.y as f32 - b.y) as i32);
        c.int((l.z as f32 - b.z) as i32);
        l.rgb.iter().for_each(|&x| c.unit(x));
        c.byte(l.radius as u8);
    }
    c.finish()
}

/// `getBodyKey` for an animal body (`AnimalVisual`): the model's name,
/// size and skin texture instead of the human visual and clothing.
pub fn animal_body_key(b: &DeadBody, model_name: Option<&str>, skin_texture: Option<&str>, attached: &[AttachedKey]) -> String {
    let (set, state, track_time) = b.anim_state();
    let mut c = Checksummer::default();
    c.byte(b.dir);
    c.int(
        (pz_anim::math::pzmath::wrap_range(b.angle, 0.0, std::f32::consts::PI * 2.0) * 57.295_776)
            as i32,
    );
    c.str(model_name);
    c.bool(b.female);
    c.bool(b.skeleton);
    c.str(Some(set));
    c.str(Some(state));
    c.int((b.animal_size * 100.0) as i32);
    c.str(skin_texture);
    for (s, p, m, blood) in attached {
        c.str(s.as_deref());
        c.str(p.as_deref());
        c.str(m.as_deref());
        c.int((blood * 100.0) as i32);
    }
    c.bool(b.fall_on_front);
    c.bool(false);
    c.bool(b.outside);
    c.bool(b.room);
    for _ in 0..3 {
        c.unit(((1.0f32 * 10.0) as i32) as f32 / 10.0);
    }
    c.int(track_time as i32);
    for l in &b.lights {
        if l.radius <= 0 {
            continue;
        }
        c.int((l.x as f32 - b.x) as i32);
        c.int((l.y as f32 - b.y) as i32);
        c.int((l.z as f32 - b.z) as i32);
        l.rgb.iter().for_each(|&x| c.unit(x));
        c.byte(l.radius as u8);
    }
    c.finish()
}

/// The render job's `AnimatedModel` (`setAnimate(false)`): `setAngle`,
/// `setState`, the variables, then `AdvancedAnimator.update` with the fps
/// multiplier at 100 (blends finish at once) and `AnimationPlayer.Update(0)`.
pub fn pose(player: &mut AnimationPlayer, set: Rc<AnimationSet>, body: &DeadBody) {
    let (_, state, track_time) = body.anim_state();
    let a = body.angle as f64;
    player.set_target_and_current_direction(a.cos() as f32, a.sin() as f32);
    let mut vars = MapVariableSource::new();
    vars.insert_slot("FallOnFront", VariableValue::Bool(body.fall_on_front));
    vars.insert_slot("KilledByFall", VariableValue::Bool(body.killed_by_fall));
    let mut animator = AdvancedAnimator::new();
    animator.set_anim_set(Some(set));
    animator.set_state(player, &mut vars, state, &[]);
    // The blends' times are scaled by fpsMultiplier = 100.
    animator.update(player, &mut vars, 100.0);
    animator.update(player, &mut vars, 100.0);
    if track_time > 0.0 {
        if let Some(id) = player.tracks().first().map(|t| t.id) {
            if let Some(t) = player.track_mut(id) {
                t.set_current_time_value(track_time);
            }
        }
    }
    player.update(0.0, Some(&vars));
}

/// `BodyTexture` + its `AtlasEntry`.
#[derive(Clone, Debug)]
pub struct BodyTexture {
    pub entry: Option<AtlasEntry>,
    /// `entry.shadowParams` (`calculateShadowParams` at `renderMain`).
    pub shadow: ShadowParams,
}

/// `DeadBodyAtlas.instance`.
pub struct DeadBodyAtlas {
    pub atlases: Atlases,
    /// `ModelManager.bitmap`'s texture id.
    pub bitmap: u64,
    pub bodies: HashMap<String, BodyTexture>,
}

impl DeadBodyAtlas {
    /// Atlas textures get ids from `first_id + 1` up; `first_id` is the
    /// bitmap.
    pub fn new(first_id: u64) -> Self {
        DeadBodyAtlas {
            atlases: Atlases::new(ATLAS_SIZE, first_id + 1),
            bitmap: first_id,
            bodies: HashMap::new(),
        }
    }

    /// `getBodyTexture` + `render()`: the cached texture for `key`, or the
    /// render job run now (posed `player`, the body's model `slot`),
    /// drawing into the bitmap and then the atlas.
    pub fn body_texture(
        &mut self,
        key: &str,
        body: &DeadBody,
        slot: &ModelSlot,
        player: &AnimationPlayer,
        res: &dyn ModelResources,
        out: &mut Vec<DrawCmd>,
    ) -> &BodyTexture {
        if !self.bodies.contains_key(key) {
            let t = self.render_job(body, slot, player, res, out);
            self.bodies.insert(key.to_string(), t);
        }
        &self.bodies[key]
    }

    fn render_job(
        &mut self,
        body: &DeadBody,
        slot: &ModelSlot,
        player: &AnimationPlayer,
        res: &dyn ModelResources,
        out: &mut Vec<DrawCmd>,
    ) -> BodyTexture {
        let scale = body.animal_size;
        // renderMain.
        let angle = player.rendered_angle();
        let shadow = shadow_params(Some(player), scale, false);
        // calcModelOffset: the bones' model-space positions.
        let skip = player.skinning_bone_index("Translation_Data", -1);
        let bones: Vec<[f32; 3]> = (0..player.num_bones())
            .filter(|&i| i as i32 != skip)
            .map(|i| {
                let m = player.model_transform(i);
                [m.m03, m.m13, m.m23]
            })
            .collect();
        let pl = body_atlas::placement(angle, scale, &bones);
        // AnimatedModel.DoRender(0, 0, 1024, 1024, SIZE_V, angle) into the
        // cleared bitmap.
        let (p, mv) = body_atlas::bitmap_camera(angle, pl.offset);
        let u = SlotUniforms {
            mvp: p.mul(&mv).m,
            target_depth: 0.5,
            lights: bitmap_lights(body, angle),
            ambient: [0.45; 3],
            tint: [1.0; 3],
            alpha: 1.0,
            final_scale: scale,
        };
        out.push(DrawCmd::AtlasBegin {
            id: self.bitmap,
            width: BITMAP_SIZE as u32,
            height: BITMAP_SIZE as u32,
            clear: true,
        });
        out.extend(
            render_slot(slot, player, &u, res)
                .into_iter()
                .map(|d| DrawCmd::Model(Box::new(animated_model_pass(d)))),
        );
        out.push(DrawCmd::AtlasEnd);
        // assignEntryToAtlas, toBodyAtlas.
        let (ai, mut e) = self.atlases.assign(pl.entry_w, pl.entry_h);
        e.offset_x = pl.entry_offset[0];
        e.offset_y = pl.entry_offset[1];
        let atlas = &mut self.atlases.list[ai];
        out.extend(body_atlas::to_body_atlas(
            &e,
            atlas.clear,
            false,
            self.bitmap,
        ));
        atlas.clear = false;
        BodyTexture {
            entry: Some(e),
            shadow,
        }
    }
}

/// A slot draw as `AnimatedModel.DoRender` blends it (`StartCharacter`'s
/// separate alpha function): the bitmap's alpha stays 1 wherever a layer
/// covers an opaque one, so the atlas has no see-through texels inside
/// the body.
pub fn animated_model_pass(mut d: ModelDraw) -> ModelDraw {
    if let ModelPass::Character { cull } = d.pass {
        d.pass = ModelPass::AnimatedModel { cull };
    }
    d
}

/// `DrawCharSingular`'s lights for the atlas render: the square's result
/// lights (0.7 x), and the fill lights from the white ambient outside (3
/// and 4) or in a room (4).
fn bitmap_lights(b: &DeadBody, angle: f32) -> [ModelLight; 5] {
    let none = ModelLight {
        direction: [0.0, 1.0, 0.0],
        colour: [0.0; 3],
    };
    let mut l = [none; 5];
    let o = [b.x, b.y, b.z];
    for (i, r) in b.lights.iter().take(5).enumerate() {
        if r.radius <= 0 {
            continue;
        }
        let pos = [r.x as f32 + 0.5, r.y as f32 + 0.5, r.z as f32 + 0.5];
        l[i] = encode_light(pos, r.rgb.map(|c| c * 0.7), r.radius as f32, o, Some(angle));
    }
    let fill = [1.7 / 4.0; 3];
    if b.outside {
        l[3] = encode_light(
            [b.x - 2.0, b.y - 2.0, b.z + 1.0],
            fill,
            5000.0,
            o,
            Some(angle),
        );
        l[4] = encode_light(
            [b.x + 2.0, b.y + 2.0, b.z + 1.0],
            fill,
            5000.0,
            o,
            Some(angle),
        );
    } else if b.room {
        l[4] = encode_light(
            [b.x + 2.0, b.y + 2.0, b.z + 1.0],
            fill,
            5000.0,
            o,
            Some(angle),
        );
    }
    l
}

/// `IsoDeadBody.render`'s draw of a ready atlas entry; `light` is the
/// square's light interpolated at the body (`interpolateLight`), with its
/// alpha.
pub fn corpse_world_draw(
    t: &BodyTexture,
    cam: &Camera,
    body: &DeadBody,
    light: [f32; 4],
) -> Vec<DrawCmd> {
    match &t.entry {
        Some(e) => body_atlas::body_world_draw(e, cam, [body.x, body.y, body.z], light),
        None => Vec::new(),
    }
}

/// `IsoDeadBody.renderShadow` (`FBORenderShadows.addShadow`): along the
/// body's angle, sized by the atlas entry's shadow parameters when there
/// is one; `light` is the square's light colour.
pub fn corpse_shadow(
    body: &DeadBody,
    atlas: Option<&BodyTexture>,
    light: [f32; 3],
) -> ShadowRequest {
    let t = body.angle as f64 + 4.712_388_980_384_69;
    let forward = [(-t.sin()) as f32, t.cos() as f32];
    let sp = match atlas {
        Some(a) if !body.animal => a.shadow,
        _ => ShadowParams {
            w: 0.45,
            fm: 1.4,
            bm: 1.125,
        },
    };
    let mut params = ShadowParams {
        w: 0.45,
        fm: 1.4,
        bm: 1.125,
    };
    if sp.bm > 0.0 {
        params.bm = sp.bm * body.animal_size;
    }
    if sp.fm > 0.0 {
        params.fm = sp.fm * body.animal_size;
    }
    if sp.w > 0.0 {
        params.w = sp.w * body.animal_size;
    }
    let skeleton_alpha = if body.skeleton { 0.5 } else { 1.0 };
    ShadowRequest::Character {
        pos: [body.x, body.y, body.z],
        forward,
        params,
        rgb: light,
        alpha: body.alpha * skeleton_alpha,
        animal: body.animal,
        slope: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anim_states() {
        let mut b = DeadBody {
            zombie: true,
            ..Default::default()
        };
        assert_eq!(b.anim_state(), ("zombie", "onground", 0.0));
        b.zombie = false;
        b.killed_by_fall = true;
        assert_eq!(b.anim_state(), ("player", "deadbody", 1.0));
        b.killed_by_fall = false;
        b.has_hand_item = true;
        assert_eq!(b.anim_state(), ("player", "deadbody", 0.0));
    }

    #[test]
    fn shadow_follows_the_angle() {
        let b = DeadBody {
            x: 3.0,
            y: 4.0,
            angle: 0.0,
            alpha: 1.0,
            animal_size: 1.0,
            ..Default::default()
        };
        let ShadowRequest::Character {
            forward, params, ..
        } = corpse_shadow(&b, None, [1.0; 3])
        else {
            panic!()
        };
        assert!((forward[0] - 1.0).abs() < 1e-6 && forward[1].abs() < 1e-6);
        assert_eq!(
            params,
            ShadowParams {
                w: 0.45,
                fm: 1.4,
                bm: 1.125
            }
        );
    }
}
