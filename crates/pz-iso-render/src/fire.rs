//! `IsoFire`'s visuals: the flame and smoke animations it attaches
//! (`AttachAnim("Fire"|"Smoke", …)`), the life-stage machine that swaps them
//! (`updateFromTimer`, `load`, `setLifeStage`), its light source, and how
//! the translucent pass draws it (`IsoFire.renderAnimatedAttachments`
//! through `FBORenderCell.renderAnimatedAttachments(object)`), with the
//! billboard depth texture `IsoSprite.setupTileDepth` gives fire sprites.
//!
//! The fire's data and life-stage machine are `pz_world::fire::IsoFire`
//! (re-exported here as [`FireState`]): the one struct the world thread
//! steps (`IsoFireManager`, spreading, burning the square) and this crate
//! draws. [`FireState::update_from_timer`] reports the gameplay moments as
//! [`FireEvent`]s; the world's fire manager acts on them where they happen
//! (`update_from_timer_with`). A smoke-only fire
//! (`IsoFireManager.StartSmoke`) is an `IsoFire` with `smoke` set and only
//! smoke attached.
//!
//! Random choices go through `pz_world::GameRand` in the original's call
//! order (`Rand.Next` on `RandStandard.INSTANCE`); [`RandAdapter`] lends a
//! [`RandSource`] to it.

use std::rc::Rc;

use pz_core::random::Rand;
use pz_render::iso::TILE_SCALE;
use pz_render::{gl, DrawCmd, Texture};
use pz_world::GameRand;

use crate::color::ColorInfo;
use crate::sprite::{self, Dir, Modifier, SpriteCtx, SpriteDraw};
use crate::world::{AttachedSprite, ObjectKind, RenderObject, RenderSquare, SpriteInfo};

pub use pz_world::fire::{
    fire_to_evict, AnimKind, FireAnim, FireEvent, FireLight, FireOscillators, FireSave,
    FireSquare, IsoFire, FIRE_ANIM_DELAY, FIRE_COLOR, FIRE_TINT_MOD, LIGHT_RADIUS_HIGH,
    LIGHT_RADIUS_LOW, LIGHT_RADIUS_MEDIUM, LIGHT_RADIUS_MINIMUM, LIGHT_RGB, MAX_FIRE_OBJECTS,
    MAX_LIFE, MIN_LIFE, NUM_FRAMES_FIRE, NUM_FRAMES_SMOKE, SMOKE_ANIM_DELAY, SMOKE_TINT_MOD,
};

/// An `IsoFire`'s state, as drawing it goes (the world's `IsoFire`).
pub type FireState = IsoFire;

/// The `Rand` calls fires make.
pub trait RandSource {
    /// `Rand.Next(int max)`.
    fn next(&self, max: i32) -> i32;
    /// `Rand.Next(int min, int max)`.
    fn next_range(&self, min: i32, max: i32) -> i32;
    /// `Rand.Next(float min, float max)`.
    fn next_float(&self, min: f32, max: f32) -> f32;
}

impl RandSource for Rand {
    fn next(&self, max: i32) -> i32 {
        Rand::next(self, max)
    }

    fn next_range(&self, min: i32, max: i32) -> i32 {
        Rand::next_range(self, min, max)
    }

    fn next_float(&self, min: f32, max: f32) -> f32 {
        Rand::next_float(self, min, max)
    }
}

/// `RandStandard.INSTANCE`, the game's shared generator.
#[derive(Clone, Copy, Debug, Default)]
pub struct StandardRand;

impl RandSource for StandardRand {
    fn next(&self, max: i32) -> i32 {
        pz_core::random::next(max)
    }

    fn next_range(&self, min: i32, max: i32) -> i32 {
        pz_core::random::next_range(min, max)
    }

    fn next_float(&self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
}

/// A [`RandSource`] as the world's [`GameRand`], with the same range
/// handling (`RandAbstract`).
pub struct RandAdapter<'a>(pub &'a dyn RandSource);

impl GameRand for RandAdapter<'_> {
    fn next(&mut self, max: i32) -> i32 {
        self.0.next(max)
    }

    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        self.0.next_range(min, max)
    }

    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        self.0.next_float(min, max)
    }
}

/// [`RandAdapter`] over `r`.
pub fn rand(r: &dyn RandSource) -> RandAdapter<'_> {
    RandAdapter(r)
}

/// The fire as the renderer reads it: `campfire` is `isCampfire()`
/// (a `Campfire` object on the square), `texture` looks a frame
/// texture up by name (`Texture.getSharedTexture`).
pub fn render_object(
fire: &FireState,
    campfire: bool,
    texture: &dyn Fn(&str) -> Option<Texture>,
) -> RenderObject {
    let mut o = RenderObject::new(ObjectKind::Fire, None);
    o.attached = fire
        .anims
        .iter()
        .map(|a| {
            let sprite = SpriteInfo {
                name: a.sprite_name(),
                texture: texture(&a.texture_name()),
                soff_x: a.soff_x,
                soff_y: a.soff_y,
                tint_mod: a.tint,
                ..Default::default()
            };
            AttachedSprite {
                sprite: Rc::new(sprite),
                alpha: 1.0,
            }
        })
        .collect();
    o.fire = Some(FireRender {
        campfire,
        life_stage: fire.life_stage,
        life_stage_timer: fire.life_stage_timer,
        life_stage_duration: fire.life_stage_duration,
    });
    o
}

/// What `IsoFire.renderAnimatedAttachments` reads beyond the attached
/// sprites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireRender {
    /// `isCampfire()`: drawn at three quarters size.
    pub campfire: bool,
    pub life_stage: i32,
    pub life_stage_timer: i32,
    pub life_stage_duration: i32,
}

impl FireRender {
    /// The smoke's fade in the last stage: the object alpha
    /// `renderAnimatedAttachments` sets (`1 - 2^(10f - 10)`).
    pub fn stage5_alpha(&self) -> Option<f32> {
        if self.life_stage != 5 {
            return None;
        }
        let duration = self.life_stage_duration as f32 / 2.0;
        let f = 1.0 - self.life_stage_timer as f32 / duration;
        let a = if f == 0.0 {
            0.0
        } else {
            2.0f32.powf(10.0 * f - 10.0)
        };
        Some(1.0 - a)
    }
}

/// `(short)` of a float.
fn to_short(v: f32) -> i32 {
    v as i32 as i16 as i32
}

/// `FBORenderCell.renderAnimatedAttachments(object)` for a fire in the
/// translucent pass: the default blend, then `IsoFire.renderAnimatedAttachments`
/// with the square's light. `obj` carries the alpha the pass gave it.
pub fn render_animated_attachments(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let Some(fire) = obj.fire else { return };
    if obj.attached.is_empty() {
        return;
    }
    ctx.out
        .push(DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA));
    let ts = TILE_SCALE;
    // IsoFire.render set offsetX/Y to 32 and 96 tiles earlier this frame.
    let offset_x = (32 * ts) as f32;
    let offset_y = (96 * ts) as f32 + (16 * ts) as f32;
    let mut scale = ts as f32 / 2.0;
    if fire.campfire {
        scale *= 0.75;
    }
    let col = sq.light_info;
    let mut alpha = obj.alpha;
    for a in &obj.attached {
        let s = &a.sprite;
        let Some(tex) = s.texture.as_ref() else {
            continue;
        };
        let c = if s.name.starts_with("Fire") {
            ColorInfo {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: col.a,
            }
        } else {
            if let Some(f) = fire.stage5_alpha() {
                alpha = f;
            }
            col
        };
        let mut sp = (**s).clone();
        sp.soff_x = to_short((s.soff_x + 32 * ts) as f32 - tex.width_orig() as f32 / 2.0 * scale);
        sp.soff_y = to_short((s.soff_y + 128 * ts) as f32 - tex.height_orig() as f32 * scale);
        let d = SpriteDraw {
            sprite: &sp,
            obj: Some(obj),
            main: false,
            x: sq.x as f32,
            y: sq.y as f32,
            z: sq.z as f32,
            dir: Dir::Other,
            offset_x,
            offset_y,
            col: c,
            alpha,
            flip: false,
            modifier: Modifier::None,
            seam: None,
        };
        sprite::render_scaled(ctx, &d, scale);
    }
}

#[cfg(test)]
mod tests;
