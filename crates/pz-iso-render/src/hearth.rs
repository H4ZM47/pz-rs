//! `IsoFireplace` and `IsoBarbecue` visuals: the flame, smoke and fuel
//! sprites they attach while lit or fuelled (`updateFuelSprite`,
//! `updateFireSprite`, `IsoBarbecue.update`'s smoke), the fireplace's light
//! source (`updateLightSource`), and how they are drawn: the fuel logs with
//! the object (`IsoFireplace.render`), the animated flames and smoke per
//! frame (`FBORenderCell.renderAnimatedAttachments`, then
//! `IsoFireplace/IsoBarbecue.renderAnimatedAttachments`).
//!
//! Burning fuel, smouldering, heat and sound are the world thread's; it
//! hands the result in as [`FireplaceInput`] (lit, fuel left) or the
//! barbecue's lit flag. The barbecue's propane-tank sprite swap
//! (`updateSprite`) is the main sprite and stays with the world too.

use std::rc::Rc;

use pz_render::iso::TILE_SCALE;
use pz_render::{gl, DrawCmd, Texture};

use crate::color::ColorInfo;
use crate::fire::{
    AnimKind, FireAnim, FireLight, RandAdapter, RandSource, FIRE_ANIM_DELAY, FIRE_COLOR, FIRE_TINT_MOD,
    SMOKE_ANIM_DELAY,
};
use crate::sprite::{self, Dir, Modifier, SpriteCtx, SpriteDraw};
use crate::world::{AttachedSprite, RenderObject, RenderSquare, SpriteInfo};

/// `IsoObject.offsetX/offsetY` of a fireplace (never changed).
const OFFSET_X: f32 = (32 * TILE_SCALE) as f32;
const OFFSET_Y: f32 = (96 * TILE_SCALE) as f32;

/// The flame's instance scale (`0.5 * (tileScale / 2)`).
pub const FIREPLACE_FIRE_SCALE: f32 = 0.5 * (TILE_SCALE as f32 / 2.0);

/// The fuel logs' texture (`LoadSingleTexture("Item_Logs")`).
pub const FUEL_TEXTURE: &str = "Item_Logs";

/// How the renderer draws a hearth's attached sprites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HearthRender {
    /// `IsoFireplace`: `fireSpriteIndex`, `fuelSpriteIndex` (-1 when
    /// none; the fuel logs are the only sprite that does not animate) and
    /// `isFireSpriteUsingOurDepthTexture()`.
    Fireplace {
        fire_index: i32,
        fuel_index: i32,
        uses_our_depth: bool,
    },
    /// `IsoBarbecue`: its smoke.
    Barbecue,
}

/// `hasAnimatedAttachments()` of a fireplace or barbecue.
pub fn has_animated_attachments(obj: &RenderObject) -> bool {
    obj.hearth.is_some() && !obj.attached.is_empty()
}

/// What `IsoFireplace.update` reads besides its own sprites.
#[derive(Clone, Copy, Debug, Default)]
pub struct FireplaceInput<'a> {
    /// `isLit()`.
    pub lit: bool,
    /// `getFuelAmount()` (minutes).
    pub fuel_amount: i32,
    /// `getSprite().getName()`.
    pub sprite_name: &'a str,
    /// The sprite's `CustomName` is `Metal Drum`.
    pub metal_drum: bool,
    /// `container.getType()`, `None` without a container.
    pub container_type: Option<&'a str>,
    /// `getProperties().has(collideW)` / `has(collideN)`.
    pub collide_w: bool,
    pub collide_n: bool,
}

/// One entry of a fireplace's `attachedAnimSprite`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HearthSprite {
    /// The fuel logs (`AttachExistingAnim(fuelSprite, …)`), not animated.
    Fuel { soff_x: i32, soff_y: i32 },
    /// A flame (`AttachAnim("Fire", "01", …)`).
    Anim(FireAnim),
}

/// `IsoFireplace`'s visual state.
#[derive(Clone, Debug, PartialEq)]
pub struct FireplaceState {
    /// `attachedAnimSprite`, in order.
    pub attached: Vec<HearthSprite>,
    /// `fuelSpriteIndex` / `fireSpriteIndex`, -1 when none. Kept as the
    /// original keeps them (an index can point past the list).
    pub fuel_index: i32,
    pub fire_index: i32,
    /// `setOverlaySprite(name)`; `None` after `setOverlaySprite(null)`.
    pub overlay: Option<String>,
    /// `fireSpriteUsesOurDepthTexture`.
    pub fire_uses_our_depth: bool,
    /// `lightSource`, while lit.
    pub light: Option<FireLight>,
}

impl Default for FireplaceState {
    fn default() -> Self {
        FireplaceState {
            attached: Vec::new(),
            fuel_index: -1,
            fire_index: -1,
            overlay: None,
            fire_uses_our_depth: true,
            light: None,
        }
    }
}

/// `(short)` of a float.
fn to_short(v: f32) -> i32 {
    v as i32 as i16 as i32
}

/// `calcLightRadius()`: 1 to 8 squares over the first hour of fuel.
pub fn fireplace_light_radius(fuel_amount: i32) -> i32 {
    let delta = (fuel_amount as f32).min(60.0) / 60.0;
    // GameTime.Lerp(1, 8, delta).
    let delta = delta.clamp(0.0, 1.0);
    (1.0 + 7.0 * delta) as i32
}

impl FireplaceState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The visual part of `IsoFireplace.update` on a client:
    /// `updateFuelSprite`, `updateFireSprite`, `updateLightSource`, then the
    /// animation step. `dt` is `GameTime.getMultipliedSecondsSinceLastUpdate()
    /// * 60`; `frame_size` gives a texture's original size
    /// (`getWidthOrig/getHeightOrig`), read when a flame is attached.
    pub fn update(
        &mut self,
        input: &FireplaceInput<'_>,
        dt: f32,
        rand: &dyn RandSource,
        frame_size: &dyn Fn(&str) -> Option<(i32, i32)>,
    ) {
        self.update_fuel_sprite(input);
        self.update_fire_sprite(input, rand, frame_size);
        self.update_light_source(input);
        for a in &mut self.attached {
            if let HearthSprite::Anim(a) = a {
                a.advance(dt);
            }
        }
    }

    fn remove_at(&mut self, i: i32) {
        // ArrayList.remove(index); the original throws past the end.
        if (0..self.attached.len() as i32).contains(&i) {
            self.attached.remove(i as usize);
        }
    }

    fn update_fuel_sprite(&mut self, input: &FireplaceInput<'_>) {
        if input.metal_drum {
            return;
        }
        if matches!(input.container_type, Some("woodstove" | "campfire")) {
            return;
        }
        let has_fuel = input.fuel_amount > 0;
        let ts = TILE_SCALE;
        if has_fuel && !input.lit && input.sprite_name == "crafted_02_42" {
            self.overlay = Some("crafted_02_43".into());
        } else if has_fuel && input.container_type.is_some() && (input.collide_w || input.collide_n)
        {
            if self.fuel_index == -1 {
                self.fuel_index = self.attached.len() as i32;
                // AttachExistingAnim(fuel, offX, offY, …): soff = -offset.
                // (At tile scale 1 the logs are also halved.)
                let soff = if input.collide_w {
                    (10 * ts, 90 * ts)
                } else {
                    (35 * ts, 90 * ts)
                };
                self.attached.push(HearthSprite::Fuel {
                    soff_x: soff.0,
                    soff_y: soff.1,
                });
            }
        } else if self.fuel_index != -1 {
            self.remove_at(self.fuel_index);
            if self.fire_index > self.fuel_index {
                self.fire_index -= 1;
            }
            self.fuel_index = -1;
        }
    }

    fn update_fire_sprite(
        &mut self,
        input: &FireplaceInput<'_>,
        rand: &dyn RandSource,
        frame_size: &dyn Fn(&str) -> Option<(i32, i32)>,
    ) {
        let ts = TILE_SCALE;
        if !input.lit {
            if self.fire_index != -1 {
                self.remove_at(self.fire_index);
                if self.fuel_index > self.fire_index {
                    self.fuel_index -= 1;
                }
                self.fire_index = -1;
                self.overlay = None;
            }
            return;
        }
        if self.fire_index != -1 {
            return;
        }
        self.fire_index = self.attached.len() as i32;
        // AttachAnim("Fire", "01", 30, 0.5, offX, offY, true, 0, false,
        // 0.7, FIRE_TINT_MOD): no random frame yet.
        let flame = |ox: i32, oy: i32| {
            HearthSprite::Anim(FireAnim::attach(
                AnimKind::Fire,
                1,
                FIRE_ANIM_DELAY,
                ox * ts,
                oy * ts,
                FIRE_TINT_MOD,
                false,
                &mut RandAdapter(rand),
            ))
        };
        let name = input.sprite_name;
        match name {
            "camping_03_16" | "camping_03_19" | "crafted_02_42" => {
                let (overlay, oy) = match name {
                    "camping_03_16" => ("camping_03_17", -13),
                    "camping_03_19" => ("camping_03_21", -7),
                    _ => ("crafted_02_44", -2),
                };
                self.overlay = Some(overlay.into());
                self.attached.push(flame(2, oy));
                self.fire_uses_our_depth = false;
            }
            "crafted_05_6" => self.overlay = Some("crafted_05_14".into()),
            "crafted_05_7" => self.overlay = Some("crafted_05_15".into()),
            "appliances_cooking_01_16" | "appliances_cooking_01_17" => {
                let p: Vec<&str> = name.split('_').collect();
                self.overlay = Some(format!("{}_{}_{}_on_{}", p[0], p[1], p[2], p[3]));
            }
            _ if input.metal_drum => {
                match name {
                    "crafted_01_26" => self.overlay = Some("crafted_01_27".into()),
                    "crafted_01_30" => self.overlay = Some("crafted_01_31".into()),
                    _ => {}
                }
                self.attached.push(flame(0, 10));
                self.fire_uses_our_depth = false;
            }
            _ if input.collide_w => self.attached.push(flame(15, -8)),
            _ if input.collide_n => self.attached.push(flame(-11, -8)),
            _ => {}
        }
        if let Some(HearthSprite::Anim(a)) = self.attached.get_mut(self.fire_index as usize) {
            // A random frame, the scale, and the frame centred on the
            // object's offsets (`sprite1.soffX/Y += …`).
            a.frame = rand.next_float(0.0, a.frame_count() as f32);
            let scale = FIREPLACE_FIRE_SCALE;
            if let Some((w, h)) = frame_size(&a.texture_name()) {
                a.soff_x =
                    to_short((a.soff_x + to_short(OFFSET_X - w as f32 * scale / 2.0)) as f32);
                a.soff_y = to_short((a.soff_y + to_short(OFFSET_Y - h as f32 * scale)) as f32);
            }
        }
    }

    fn update_light_source(&mut self, input: &FireplaceInput<'_>) {
        if !input.lit {
            self.light = None;
            return;
        }
        let radius = fireplace_light_radius(input.fuel_amount);
        if self.light.is_some_and(|l| l.radius != radius) {
            self.light = None;
        }
        if self.light.is_none() {
            let [r, g, b] = FIRE_COLOR;
            self.light = Some(FireLight { r, g, b, radius });
        }
    }

    /// Fills `obj` (built from the fireplace's own sprite) with the
    /// attached sprites; `texture` looks a texture up by name
    /// (`Texture.getSharedTexture`). The overlay sprite ([`Self::overlay`])
    /// is the caller's to resolve.
    pub fn apply(&self, obj: &mut RenderObject, texture: &dyn Fn(&str) -> Option<Texture>) {
        obj.attached = self
            .attached
            .iter()
            .map(|a| {
                let sprite = match a {
                    HearthSprite::Fuel { soff_x, soff_y } => SpriteInfo {
                        name: FUEL_TEXTURE.into(),
                        texture: texture(FUEL_TEXTURE),
                        soff_x: *soff_x,
                        soff_y: *soff_y,
                        ..Default::default()
                    },
                    HearthSprite::Anim(a) => anim_sprite(a, texture),
                };
                AttachedSprite {
                    sprite: Rc::new(sprite),
                    alpha: 1.0,
                }
            })
            .collect();
        obj.hearth = Some(HearthRender::Fireplace {
            fire_index: self.fire_index,
            fuel_index: self.fuel_index,
            uses_our_depth: self.fire_uses_our_depth,
        });
    }
}

fn anim_sprite(a: &FireAnim, texture: &dyn Fn(&str) -> Option<Texture>) -> SpriteInfo {
    SpriteInfo {
        name: a.sprite_name(),
        texture: texture(&a.texture_name()),
        soff_x: a.soff_x,
        soff_y: a.soff_y,
        tint_mod: a.tint,
        ..Default::default()
    }
}

/// `IsoBarbecue.SmokeTint`.
pub const BARBECUE_SMOKE_TINT: [f32; 3] = [1.0, 1.0, 1.0];

/// `IsoBarbecue`'s visual state: its smoke while lit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarbecueState {
    /// `attachedAnimSprite`.
    pub attached: Vec<FireAnim>,
}

impl BarbecueState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The visual part of `IsoBarbecue.update` on a client: smoke
    /// (`AttachAnim("Smoke", "01", 30, smokeAnimDelay, 20, 58, …)` at full
    /// alpha) while lit, removed when not, then the animation step.
    pub fn update(&mut self, lit: bool, dt: f32) {
        if lit && self.attached.is_empty() {
            self.attached.push(FireAnim {
                kind: AnimKind::Smoke,
                anim: 1,
                frame: 0.0,
                frame_increase: SMOKE_ANIM_DELAY,
                soff_x: -20,
                soff_y: -58,
                tint: BARBECUE_SMOKE_TINT,
            });
        } else if !lit && !self.attached.is_empty() {
            self.attached.clear();
        }
        for a in &mut self.attached {
            a.advance(dt);
        }
    }

    /// Fills `obj` with the smoke.
    pub fn apply(&self, obj: &mut RenderObject, texture: &dyn Fn(&str) -> Option<Texture>) {
        obj.attached = self
            .attached
            .iter()
            .map(|a| AttachedSprite {
                sprite: Rc::new(anim_sprite(a, texture)),
                alpha: 1.0,
            })
            .collect();
        obj.hearth = Some(HearthRender::Barbecue);
    }
}

/// `IsoFireplace.render`'s own loop while recording the chunk texture:
/// the sprites that do not animate (the fuel logs), in the colour the
/// object was drawn with. (The overlay and the other attached sprites are
/// skipped by `IsoObject.render` once there are animated attachments.)
pub fn render_static(
    ctx: &mut SpriteCtx<'_>,
    sq: &RenderSquare,
    obj: &RenderObject,
    col: ColorInfo,
) {
    let Some(HearthRender::Fireplace { fuel_index, .. }) = obj.hearth else {
        return;
    };
    for (i, a) in obj.attached.iter().enumerate() {
        if i as i32 != fuel_index {
            continue;
        }
        let d = SpriteDraw {
            sprite: &a.sprite,
            obj: Some(obj),
            main: false,
            x: sq.x as f32,
            y: sq.y as f32,
            z: sq.z as f32,
            dir: Dir::Other,
            offset_x: obj.offset_x,
            offset_y: obj.offset_y,
            col,
            alpha: obj.alpha,
            flip: false,
            modifier: Modifier::None,
            seam: None,
        };
        sprite::render(ctx, &d);
    }
}

/// `FBORenderCell.renderAnimatedAttachments(object)` for a fireplace or
/// barbecue: the default blend, then the object's
/// `renderAnimatedAttachments` in the square's light.
pub fn render_animated_attachments(ctx: &mut SpriteCtx<'_>, sq: &RenderSquare, obj: &RenderObject) {
    let Some(hearth) = obj.hearth else { return };
    if obj.attached.is_empty() {
        return;
    }
    ctx.out
        .push(DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA));
    let ts = TILE_SCALE;
    let mut col = sq.light_info;
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    for (i, a) in obj.attached.iter().enumerate() {
        let draw = |ctx: &mut SpriteCtx<'_>,
                    sprite: &SpriteInfo,
                    col: ColorInfo,
                    alpha: f32,
                    scale: f32| {
            let d = SpriteDraw {
                sprite,
                obj: Some(obj),
                main: false,
                x,
                y,
                z,
                dir: Dir::Other,
                offset_x: obj.offset_x,
                offset_y: obj.offset_y,
                col,
                alpha,
                flip: false,
                modifier: Modifier::None,
                seam: None,
            };
            sprite::render_scaled(ctx, &d, scale);
        };
        match hearth {
            HearthRender::Fireplace {
                fire_index,
                fuel_index,
                ..
            } => {
                if i as i32 == fuel_index {
                    continue;
                }
                // The colour is saved as (a, g, b) and restored after the
                // draw, so later sprites see red = alpha. (The original
                // writes into the square's light info itself.)
                let saved = (col.a, col.g, col.b);
                let mut c = col;
                if i as i32 == fire_index {
                    c.r = 1.0;
                    c.g = 1.0;
                    c.b = 1.0;
                }
                draw(ctx, &a.sprite, c, obj.alpha, FIREPLACE_FIRE_SCALE);
                col.r = saved.0;
                col.g = saved.1;
                col.b = saved.2;
            }
            HearthRender::Barbecue => {
                // soffX = 0, soffY = -58 * tileScale, setScale(tileScale).
                let mut sp = (*a.sprite).clone();
                sp.soff_x = 0;
                sp.soff_y = -58 * ts;
                draw(ctx, &sp, col, 1.0, ts as f32);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
