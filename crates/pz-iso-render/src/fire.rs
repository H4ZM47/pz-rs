//! `IsoFire`'s visuals: the flame and smoke animations it attaches
//! (`AttachAnim("Fire"|"Smoke", …)`), the life-stage machine that swaps them
//! (`updateFromTimer`, `load`, `setLifeStage`), its light source, and how
//! the translucent pass draws it (`IsoFire.renderAnimatedAttachments`
//! through `FBORenderCell.renderAnimatedAttachments(object)`), with the
//! billboard depth texture `IsoSprite.setupTileDepth` gives fire sprites.
//!
//! The gameplay side (spreading, burning the square, damage, sound) is the
//! caller's: [`FireState::update_from_timer`] reports those moments as
//! [`FireEvent`]s. A smoke-only fire (`IsoFireManager.StartSmoke`) is an
//! `IsoFire` with `smoke` set and only smoke attached.
//!
//! Random choices go through a [`RandSource`] in the original's call
//! order (`Rand.Next` on `RandStandard.INSTANCE`).

use std::rc::Rc;

use pz_core::random::Rand;
use pz_render::iso::TILE_SCALE;
use pz_render::{gl, DrawCmd, Texture};

use crate::color::ColorInfo;
use crate::sprite::{self, Dir, Modifier, SpriteCtx, SpriteDraw};
use crate::world::{AttachedSprite, ObjectKind, RenderObject, RenderSquare, SpriteInfo};

/// `IsoFire.LIGHT_RADIUS_*`.
pub const LIGHT_RADIUS_MINIMUM: i32 = 1;
pub const LIGHT_RADIUS_LOW: i32 = 5;
pub const LIGHT_RADIUS_MEDIUM: i32 = 8;
pub const LIGHT_RADIUS_HIGH: i32 = 12;
/// `IsoFire.NUM_FRAMES_FIRE` / `NUM_FRAMES_SMOKE`.
pub const NUM_FRAMES_FIRE: i32 = 30;
pub const NUM_FRAMES_SMOKE: i32 = 30;
/// `IsoFire.MaxLife` / `MinLife`.
pub const MAX_LIFE: i32 = 3000;
pub const MIN_LIFE: i32 = 800;
/// `IsoFire.LIGHT_R/G/B`: the light a new fire adds.
pub const LIGHT_RGB: [f32; 3] = [0.61, 0.175, 0.0];
/// `IsoObject.fireColor`: the light's colour once its radius changes.
pub const FIRE_COLOR: [f32; 3] = [0.51, 0.27, 0.18];
/// `IsoFireManager.FIRE_ANIM_DELAY` / `smokeAnimDelay`: frames advanced per
/// 1/60 s.
pub const FIRE_ANIM_DELAY: f32 = 0.5;
pub const SMOKE_ANIM_DELAY: f32 = 0.5;
/// `IsoFireManager.FIRE_TINT_MOD` / `smokeTintMod`.
pub const FIRE_TINT_MOD: [f32; 3] = [1.0, 1.0, 1.0];
pub const SMOKE_TINT_MOD: [f32; 3] = [0.5, 0.5, 0.5];
/// `IsoFireManager.maxFireObjects`.
pub const MAX_FIRE_OBJECTS: usize = 75;

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

/// Which animation an attached sprite plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimKind {
    Fire,
    Smoke,
}

impl AnimKind {
    fn object_name(self) -> &'static str {
        match self {
            AnimKind::Fire => "Fire",
            AnimKind::Smoke => "Smoke",
        }
    }
}

/// One `AttachAnim(objectName, animName, 30, frameIncrease, offsetX,
/// offsetY, true, 0, false, 0.7, tintMod[, randomFrame])`: its sprite
/// (`IsoSprite.CreateSpriteUsingCache`) and instance (`def`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireAnim {
    pub kind: AnimKind,
    /// The anim name's number (`"01"`..`"03"`).
    pub anim: u8,
    /// `IsoSpriteInstance.frame`.
    pub frame: f32,
    /// `animFrameIncrease`.
    pub frame_increase: f32,
    /// `soffX` / `soffY` (the negated `AttachAnim` offsets).
    pub soff_x: i32,
    pub soff_y: i32,
    /// `tintMod`.
    pub tint: [f32; 3],
}

impl FireAnim {
    /// `AttachAnim`: `random_frame` starts at `Rand.Next(0, frameCount)`.
    #[allow(clippy::too_many_arguments)]
    fn attach(
        kind: AnimKind,
        anim: u8,
        frame_increase: f32,
        offset_x: i32,
        offset_y: i32,
        tint: [f32; 3],
        random_frame: bool,
        rand: &dyn RandSource,
    ) -> Self {
        let mut a = FireAnim {
            kind,
            anim,
            frame: 0.0,
            frame_increase,
            soff_x: -offset_x,
            soff_y: -offset_y,
            tint,
        };
        if random_frame && a.frame_count() > 1 {
            a.frame = rand.next_float(0.0, a.frame_count() as f32);
        }
        a
    }

    fn fire(anim: u8, rand: &dyn RandSource) -> Self {
        Self::attach(
            AnimKind::Fire,
            anim,
            FIRE_ANIM_DELAY,
            0,
            0,
            FIRE_TINT_MOD,
            true,
            rand,
        )
    }

    fn smoke(anim: u8, rand: &dyn RandSource) -> Self {
        Self::attach(
            AnimKind::Smoke,
            anim,
            SMOKE_ANIM_DELAY,
            0,
            0,
            SMOKE_TINT_MOD,
            true,
            rand,
        )
    }

    /// `getFrameCount()`.
    pub fn frame_count(&self) -> i32 {
        match self.kind {
            AnimKind::Fire => NUM_FRAMES_FIRE,
            AnimKind::Smoke => NUM_FRAMES_SMOKE,
        }
    }

    /// The sprite's `name` (its anim cache key, `"Fire01"`).
    pub fn sprite_name(&self) -> String {
        format!("{}{:02}", self.kind.object_name(), self.anim)
    }

    /// `getAnimFrame((int) def.frame)`'s index, clamped to the frames.
    pub fn frame_index(&self) -> i32 {
        (self.frame as i32).clamp(0, self.frame_count() - 1)
    }

    /// The texture of frame `i` (`IsoAnim.LoadFramesNoDirPage`:
    /// `objectName_animName_i`).
    pub fn texture_name_for(&self, i: i32) -> String {
        format!("{}_{:02}_{i}", self.kind.object_name(), self.anim)
    }

    /// The current frame's texture name.
    pub fn texture_name(&self) -> String {
        self.texture_name_for(self.frame_index())
    }

    /// `IsoFire.update`'s per-sprite step: `dt` is
    /// `GameTime.getMultipliedSecondsSinceLastUpdate() * 60`; looped
    /// animations restart once past their last frame.
    pub fn advance(&mut self, dt: f32) {
        self.frame += self.frame_increase * dt;
        if self.frame as i32 >= self.frame_count() {
            self.frame = 0.0;
        }
    }
}

/// The light an `IsoFire` adds to its cell (`IsoLightSource`), as the
/// lighting code reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireLight {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub radius: i32,
}

/// The gameplay moments of [`FireState::update_from_timer`], in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireEvent {
    /// `square.Burn()` (entering stages 1 and 2).
    Burn,
    /// `Spread()` is due.
    Spread,
    /// `extinctFire()`: remove the fire.
    Extinct,
    /// The light source was replaced (`setLightRadius`); relight.
    LightChanged,
}

/// What `new IsoFire(cell, square, canBurnAnywhere, energy, life)` reads
/// from the square.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireSquare {
    /// `getProperties().has(vegitation)`.
    pub vegitation: bool,
    /// `getFloor().getSprite().firerequirement`, with a floor.
    pub floor_fire_requirement: Option<i32>,
    /// `TutorialManager.instance.active`.
    pub tutorial: bool,
}

/// The fields `IsoFire.save` writes after the `IsoObject` ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireSave {
    pub life: i32,
    pub spread_delay: i32,
    /// Saved as `lifeStage - 1`.
    pub life_stage: i32,
    pub life_stage_timer: i32,
    pub life_stage_duration: i32,
    pub energy: i32,
    pub num_flame_particles: i32,
    pub spread_timer: i32,
    pub age: i32,
    pub perm: bool,
    pub light_radius: i32,
    pub smoke: bool,
    /// World version 242 and later.
    pub saved_update_time: Option<i64>,
}

impl FireSave {
    /// Reads the fields (big-endian, as `ByteBuffer`), returning the bytes
    /// used.
    pub fn read(b: &[u8], world_version: i32) -> Option<(FireSave, usize)> {
        let mut at = 0;
        let mut int = || {
            let v = i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?);
            at += 4;
            Some(v)
        };
        let (life, spread_delay, life_stage, life_stage_timer, life_stage_duration) =
            (int()?, int()?, int()?, int()?, int()?);
        let (energy, num_flame_particles, spread_timer, age) = (int()?, int()?, int()?, int()?);
        let bytes = b.get(at..at + 3)?;
        at += 3;
        let saved_update_time = if world_version >= 242 {
            let v = i64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?);
            at += 8;
            Some(v)
        } else {
            None
        };
        let s = FireSave {
            life,
            spread_delay,
            life_stage,
            life_stage_timer,
            life_stage_duration,
            energy,
            num_flame_particles,
            spread_timer,
            age,
            perm: bytes[0] != 0,
            light_radius: i32::from(bytes[1]),
            smoke: bytes[2] != 0,
            saved_update_time,
        };
        Some((s, at))
    }
}

/// An `IsoFire`'s state as far as drawing it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct FireState {
    pub age: i32,
    pub energy: i32,
    pub life: i32,
    pub life_stage: i32,
    pub life_stage_duration: i32,
    pub life_stage_timer: i32,
    pub spread_delay: i32,
    pub spread_timer: i32,
    pub num_flame_particles: i32,
    pub perm: bool,
    pub smoke: bool,
    pub light_radius: i32,
    /// `lightSource` (none for smoke, or before the first update).
    pub light: Option<FireLight>,
    /// `attachedAnimSprite`.
    pub anims: Vec<FireAnim>,
    /// `accum`: the fraction of a tick `updateFromTimer` carries over.
    pub accum: f32,
    saved_update_time: i64,
}

impl FireState {
    fn blank() -> Self {
        FireState {
            age: 0,
            energy: 0,
            life: 0,
            life_stage: 0,
            life_stage_duration: 0,
            life_stage_timer: 0,
            spread_delay: 0,
            spread_timer: 0,
            num_flame_particles: 0,
            perm: false,
            smoke: false,
            light_radius: 1,
            light: None,
            anims: Vec::new(),
            accum: 0.0,
            saved_update_time: -1,
        }
    }

    /// `new IsoFire(cell, square)`: a permanent fire (campfires), drawn as
    /// [`FireState::set_life_stage`] says.
    pub fn permanent() -> Self {
        FireState {
            perm: true,
            ..Self::blank()
        }
    }

    /// `new IsoFire(cell, square, canBurnAnywhere, energy, setLife)`.
    pub fn new_fire(
        sq: FireSquare,
        starting_energy: i32,
        set_life: i32,
        rand: &dyn RandSource,
    ) -> Self {
        let mut f = Self::blank();
        f.num_flame_particles = 1;
        for _ in 0..f.num_flame_particles {
            f.anims.push(FireAnim::fire(1, rand));
        }
        f.life = 800 + rand.next(2200);
        if set_life > 0 {
            f.life = set_life;
        }
        if let (0, false, Some(req)) = (set_life, sq.vegitation, sq.floor_fire_requirement) {
            f.life -= req * 100;
            if f.life < 600 {
                f.life = rand.next_range(300, 600);
            }
        }
        f.spread_timer = rand.next(f.life - f.life / 2);
        f.spread_delay = f.spread_timer;
        f.life_stage = 0;
        f.life_stage_duration = f.life / 5;
        f.life_stage_timer = f.life_stage_duration;
        if sq.tutorial {
            f.life_stage_duration *= 2;
            f.life *= 2;
            f.spread_timer /= 4;
            f.spread_delay = f.spread_timer;
        }
        f.energy = starting_energy;
        if sq.vegitation {
            f.energy += 50;
        }
        f
    }

    /// `new IsoFire(cell, square, canBurnAnywhere, energy, setLife,
    /// isSmoke)` (`IsoFireManager.StartSmoke`): smoke from stage 4.
    pub fn new_smoke(
        starting_energy: i32,
        set_life: i32,
        is_smoke: bool,
        rand: &dyn RandSource,
    ) -> Self {
        let mut f = Self::blank();
        // The eleven-argument AttachAnim: no random frame.
        f.anims.push(FireAnim::attach(
            AnimKind::Smoke,
            3,
            SMOKE_ANIM_DELAY,
            0,
            0,
            SMOKE_TINT_MOD,
            false,
            rand,
        ));
        f.life = 800 + rand.next(2200);
        if set_life > 0 {
            f.life = set_life;
        }
        f.life_stage = 4;
        f.life_stage_duration = f.life / 4;
        f.life_stage_timer = f.life_stage_duration;
        f.energy = starting_energy;
        f.smoke = is_smoke;
        f
    }

    /// `IsoFire.load`'s fields and animations.
    pub fn load(s: &FireSave, rand: &dyn RandSource) -> Self {
        let mut f = Self::blank();
        f.life = s.life;
        f.spread_delay = s.spread_delay;
        f.life_stage = s.life_stage;
        f.life_stage_timer = s.life_stage_timer;
        f.life_stage_duration = s.life_stage_duration;
        f.energy = s.energy;
        f.num_flame_particles = s.num_flame_particles;
        f.spread_timer = s.spread_timer;
        f.age = s.age;
        f.perm = s.perm;
        f.light_radius = s.light_radius;
        f.smoke = s.smoke;
        if let Some(t) = s.saved_update_time {
            f.saved_update_time = t;
        }
        if f.perm {
            let ts = TILE_SCALE;
            f.anims.push(FireAnim::attach(
                AnimKind::Fire,
                1,
                FIRE_ANIM_DELAY,
                ts,
                -ts,
                FIRE_TINT_MOD,
                true,
                rand,
            ));
            return f;
        }
        if f.num_flame_particles == 0 {
            f.num_flame_particles = 1;
        }
        match f.life_stage {
            -1 => {
                f.life_stage = 0;
                for _ in 0..f.num_flame_particles {
                    f.anims.push(FireAnim::fire(1, rand));
                }
            }
            0 => {
                f.life_stage = 1;
                f.life_stage_timer = f.life_stage_duration;
                f.anims.push(FireAnim::fire(2, rand));
            }
            1 => {
                f.life_stage = 2;
                f.life_stage_timer = f.life_stage_duration;
                f.anims.push(FireAnim::smoke(1, rand));
                f.anims.push(FireAnim::fire(3, rand));
            }
            2 => {
                f.life_stage = 3;
                f.life_stage_timer = f.life_stage_duration / 3;
                f.anims.clear();
                f.anims.push(FireAnim::smoke(2, rand));
                f.anims.push(FireAnim::fire(2, rand));
            }
            3 => {
                f.life_stage = 4;
                f.life_stage_timer = f.life_stage_duration / 3;
                f.anims.clear();
                f.anims.push(FireAnim::smoke(3, rand));
                if !f.smoke {
                    f.anims.push(FireAnim::fire(1, rand));
                }
            }
            4 => {
                f.life_stage = 5;
                f.life_stage_timer = f.life_stage_duration / 3;
                f.anims.clear();
                f.anims.push(FireAnim::smoke(1, rand));
            }
            _ => {}
        }
        f
    }

    /// `IsoFire.save`'s fields; `now` is the calendar time in milliseconds.
    pub fn save(&self, now: i64) -> FireSave {
        FireSave {
            life: self.life,
            spread_delay: self.spread_delay,
            life_stage: self.life_stage - 1,
            life_stage_timer: self.life_stage_timer,
            life_stage_duration: self.life_stage_duration,
            energy: self.energy,
            num_flame_particles: self.num_flame_particles,
            spread_timer: self.spread_timer,
            age: self.age,
            perm: self.perm,
            light_radius: self.light_radius & 0xFF,
            smoke: self.smoke,
            saved_update_time: Some(now),
        }
    }

    /// The square flag the fire keeps set: `burning` before stage 4,
    /// `smoke` from it.
    pub fn burning(&self) -> bool {
        self.life_stage < 4
    }

    /// `IsoFire.update` without the sound, `BurnTick` and heat source:
    /// `dt` is `GameTime.getMultipliedSecondsSinceLastUpdate() * 60`,
    /// `thirty_fps` `getThirtyFPSMultiplier()`, `now` the calendar time in
    /// milliseconds and `alpha` the object's alpha for the player
    /// (`getAlpha(playerIndex)`, which the stage-5 fade sets).
    pub fn update(
        &mut self,
        dt: f32,
        thirty_fps: f32,
        now: i64,
        alpha: f32,
        rand: &dyn RandSource,
    ) -> Vec<FireEvent> {
        for a in &mut self.anims {
            a.advance(dt);
        }
        if !self.smoke && self.light.is_none() {
            let [r, g, b] = LIGHT_RGB;
            self.light = Some(FireLight {
                r,
                g,
                b,
                radius: if self.perm {
                    self.light_radius
                } else {
                    LIGHT_RADIUS_LOW
                },
            });
        }
        if let (Some(l), 5) = (self.light.as_mut(), self.life_stage) {
            l.r = LIGHT_RGB[0] * alpha;
            l.g = LIGHT_RGB[1] * alpha;
            l.b = LIGHT_RGB[2] * alpha;
        }
        if self.perm {
            return Vec::new();
        }
        let mut events = Vec::new();
        // updateSinceLastLoaded.
        if self.saved_update_time != -1 {
            if self.saved_update_time > now {
                self.saved_update_time = now;
            }
            let delta = (now - self.saved_update_time) as f32 / 30.0;
            self.saved_update_time = -1;
            events.extend(self.update_from_timer(delta, rand));
        }
        events.extend(self.update_from_timer(thirty_fps, rand));
        events
    }

    /// `updateFromTimer(timer)`: whole ticks of the life-stage machine.
    pub fn update_from_timer(&mut self, timer: f32, rand: &dyn RandSource) -> Vec<FireEvent> {
        let mut events = Vec::new();
        self.accum += timer;
        while self.accum > 1.0 {
            self.accum -= 1.0;
            self.age += 1;
            if self.life_stage_timer > 0 {
                self.life_stage_timer -= 1;
                if self.life_stage_timer <= 0 {
                    self.next_stage(&mut events, rand);
                }
            }
            if self.life <= 0 {
                events.push(FireEvent::Extinct);
                break;
            }
            self.life -= 1;
            if self.life_stage > 0 && self.spread_timer > 0 {
                self.spread_timer -= 1;
                if self.spread_timer <= 0 {
                    if self.life_stage != 5 {
                        events.push(FireEvent::Spread);
                    }
                    self.spread_timer = self.spread_delay;
                }
            }
            if self.energy <= 0 {
                events.push(FireEvent::Extinct);
                break;
            }
        }
        events
    }

    fn next_stage(&mut self, events: &mut Vec<FireEvent>, rand: &dyn RandSource) {
        let light = self.light.is_some();
        match self.life_stage {
            0 => {
                self.life_stage = 1;
                self.life_stage_timer = self.life_stage_duration;
                events.push(FireEvent::Burn);
                if light {
                    self.set_light_radius(LIGHT_RADIUS_LOW, events);
                }
            }
            1 => {
                self.life_stage = 2;
                self.life_stage_timer = self.life_stage_duration;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
                events.push(FireEvent::Burn);
                if light {
                    self.set_light_radius(LIGHT_RADIUS_MEDIUM, events);
                }
            }
            2 => {
                self.life_stage = 3;
                self.life_stage_timer = self.life_stage_duration;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(3, rand));
                self.anims.push(FireAnim::fire(3, rand));
                if light {
                    self.set_light_radius(LIGHT_RADIUS_HIGH, events);
                }
            }
            3 => {
                self.life_stage = 4;
                self.life_stage_timer = self.life_stage_duration / 2;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
                if light {
                    self.set_light_radius(LIGHT_RADIUS_MEDIUM, events);
                }
            }
            4 => {
                self.life_stage = 5;
                self.life_stage_timer = self.life_stage_duration / 2;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(1, rand));
                if light {
                    self.set_light_radius(LIGHT_RADIUS_MINIMUM, events);
                }
            }
            _ => {}
        }
    }

    /// `setLightRadius(radius)`: a different radius replaces the light,
    /// now in `fireColor`.
    pub fn set_light_radius(&mut self, radius: i32, events: &mut Vec<FireEvent>) {
        self.light_radius = radius;
        if let Some(l) = self.light {
            if radius != l.radius {
                let [r, g, b] = FIRE_COLOR;
                self.light = Some(FireLight { r, g, b, radius });
                events.push(FireEvent::LightChanged);
            }
        }
    }

    /// `setLifeStage(stage)` for a permanent fire.
    pub fn set_life_stage(&mut self, stage: i32, rand: &dyn RandSource) {
        if !self.perm {
            return;
        }
        self.anims.clear();
        match stage {
            0 => self.anims.push(FireAnim::fire(1, rand)),
            1 | 3 => {
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
            }
            2 => {
                self.anims.push(FireAnim::smoke(3, rand));
                self.anims.push(FireAnim::fire(3, rand));
            }
            4 => self.anims.push(FireAnim::smoke(1, rand)),
            _ => {}
        }
    }

    /// The fire as the renderer reads it: `campfire` is `isCampfire()`
    /// (a `Campfire` object on the square), `texture` looks a frame
    /// texture up by name (`Texture.getSharedTexture`).
    pub fn render_object(
        &self,
        campfire: bool,
        texture: &dyn Fn(&str) -> Option<Texture>,
    ) -> RenderObject {
        let mut o = RenderObject::new(ObjectKind::Fire, None);
        o.attached = self
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
            life_stage: self.life_stage,
            life_stage_timer: self.life_stage_timer,
            life_stage_duration: self.life_stage_duration,
        });
        o
    }
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

/// `IsoFireManager.Add`'s eviction: with `MAX_FIRE_OBJECTS` fires already,
/// the first oldest one (largest `age`, above 0) is removed for the new
/// one.
pub fn fire_to_evict(ages: &[i32]) -> Option<usize> {
    if ages.len() < MAX_FIRE_OBJECTS {
        return None;
    }
    let mut oldest = None;
    let mut oldest_age = 0;
    for (i, &a) in ages.iter().enumerate() {
        if a > oldest_age {
            oldest_age = a;
            oldest = Some(i);
        }
    }
    oldest
}

/// `IsoFireManager`'s colour oscillators, stepped once per `Update`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireOscillators {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub red_val: f64,
    pub green_val: f64,
    pub blue_val: f64,
}

impl FireOscillators {
    /// `IsoFireManager.Update`'s oscillator step (every channel advances by
    /// the blue rate).
    pub fn update(&mut self) {
        let rate = f64::from(0.0876f32) * f64::from(15.6f32);
        let effect = f64::from(0.0039f32);
        self.red += rate;
        self.green += rate;
        self.blue += rate;
        self.red_val = (self.red.sin() + 1.0) / 2.0 * effect;
        self.green_val = (self.green.sin() + 1.0) / 2.0 * effect;
        self.blue_val = (self.blue.sin() + 1.0) / 2.0 * effect;
    }
}

#[cfg(test)]
mod tests;
