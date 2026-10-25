//! Fire: `zombie.iso.objects.IsoFire` (the one fire data struct, which
//! `pz-iso-render` draws), `IsoFireManager` ([`manager`]), the square
//! burning it causes (`IsoGridSquare.Burn`, `BurnWalls`, `BurnTick`,
//! [`burn`]) and what characters standing in it do ([`character`]).
//!
//! [`IsoFire`] is an object on its square (`ObjectKind::Fire`); its life
//! stage machine ([`IsoFire::update_from_timer_with`]) calls back at the
//! moments the original burns the square, spreads, goes out or replaces its
//! light ([`FireEvent`]), so the manager can act with the fire's own `Rand`
//! order. Renderers read the same struct (attached animations, light).
//!
//! Random choices go through [`GameRand`] in the original's call order
//! (`Rand.Next` on `RandStandard.INSTANCE`).

pub mod burn;
pub mod character;
pub mod fighting;
pub mod hearth;
pub mod manager;
pub mod sound;

use crate::java::GameRand;

pub use manager::{
    FireFrame, FireHooks, FireMutation, FireRef, FireSettings, FireWorld, IsoFireManager,
};

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
/// `IsoFire`'s heat source temperature (`new IsoHeatSource(x, y, z,
/// lightRadius, 35)`).
pub const HEAT_SOURCE_TEMPERATURE: i32 = 35;
/// `IsoFireManager.maxFireObjects`.
pub const MAX_FIRE_OBJECTS: usize = 75;

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
    pub fn attach(
        kind: AnimKind,
        anim: u8,
        frame_increase: f32,
        offset_x: i32,
        offset_y: i32,
        tint: [f32; 3],
        random_frame: bool,
        rand: &mut dyn GameRand,
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
            a.frame = rand.next_float_range(0.0, a.frame_count() as f32);
        }
        a
    }

    fn fire(anim: u8, rand: &mut dyn GameRand) -> Self {
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

    fn smoke(anim: u8, rand: &mut dyn GameRand) -> Self {
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

/// The gameplay moments of [`IsoFire::update_from_timer_with`], in order.
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

/// What [`IsoFire::update_from_timer_with`] calls at each [`FireEvent`],
/// while the fire is being stepped: `square.Burn()`, `Spread()` (which
/// spends the fire's energy, so it gets the fire) and `extinctFire()`.
pub trait FireCallbacks {
    fn on_event(&mut self, fire: &mut IsoFire, event: FireEvent, rand: &mut dyn GameRand);
}

impl<F: FnMut(&mut IsoFire, FireEvent, &mut dyn GameRand)> FireCallbacks for F {
    fn on_event(&mut self, fire: &mut IsoFire, event: FireEvent, rand: &mut dyn GameRand) {
        self(fire, event, rand)
    }
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

/// `IsoFire`: the fields the life-stage machine, saving and drawing use.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoFire {
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
    /// `savedUpdateTime`: the calendar time of the save it was loaded
    /// from, until the first update catches up; `-1` otherwise.
    pub saved_update_time: i64,
    /// `heatSource`'s radius: a permanent fire's `IsoHeatSource` (at
    /// [`HEAT_SOURCE_TEMPERATURE`]), added by its first update and
    /// resized to `lightRadius` by the later ones.
    pub heat_source: Option<i32>,
}

impl IsoFire {
    fn blank() -> Self {
        IsoFire {
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
            heat_source: None,
        }
    }

    /// `new IsoFire(cell, square)`: a permanent fire (campfires), drawn as
    /// [`IsoFire::set_life_stage`] says.
    pub fn permanent() -> Self {
        IsoFire {
            perm: true,
            ..Self::blank()
        }
    }

    /// `new IsoFire(cell, square, canBurnAnywhere, energy, setLife)` (the
    /// square's `burning` flag and the `OnNewFire` event are the caller's).
    pub fn new_fire(
        sq: FireSquare,
        starting_energy: i32,
        set_life: i32,
        rand: &mut dyn GameRand,
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
        rand: &mut dyn GameRand,
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

    /// `IsoFire.load`'s fields: the saved stage is advanced to the next one
    /// (`lifeStage` is saved minus one) with its timer, without the
    /// animations ([`IsoFire::attach_loaded_anims`]).
    pub fn from_save(s: &FireSave) -> Self {
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
            return f;
        }
        if f.num_flame_particles == 0 {
            f.num_flame_particles = 1;
        }
        let d = f.life_stage_duration;
        match f.life_stage {
            -1 => f.life_stage = 0,
            0 => (f.life_stage, f.life_stage_timer) = (1, d),
            1 => (f.life_stage, f.life_stage_timer) = (2, d),
            2 => (f.life_stage, f.life_stage_timer) = (3, d / 3),
            3 => (f.life_stage, f.life_stage_timer) = (4, d / 3),
            4 => (f.life_stage, f.life_stage_timer) = (5, d / 3),
            _ => {}
        }
        f
    }

    /// `IsoFire.load`'s `AttachAnim` calls for a fire made by
    /// [`IsoFire::from_save`] from a save with `saved_stage`; `tile_scale`
    /// is `Core.tileScale` (a permanent fire's offset).
    pub fn attach_loaded_anims(
        &mut self,
        saved_stage: i32,
        tile_scale: i32,
        rand: &mut dyn GameRand,
    ) {
        if self.perm {
            let ts = tile_scale;
            self.anims.push(FireAnim::attach(
                AnimKind::Fire,
                1,
                FIRE_ANIM_DELAY,
                ts,
                -ts,
                FIRE_TINT_MOD,
                true,
                rand,
            ));
            return;
        }
        match saved_stage {
            -1 => {
                for _ in 0..self.num_flame_particles {
                    self.anims.push(FireAnim::fire(1, rand));
                }
            }
            0 => self.anims.push(FireAnim::fire(2, rand)),
            1 => {
                self.anims.push(FireAnim::smoke(1, rand));
                self.anims.push(FireAnim::fire(3, rand));
            }
            2 => {
                self.anims.clear();
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
            }
            3 => {
                self.anims.clear();
                self.anims.push(FireAnim::smoke(3, rand));
                if !self.smoke {
                    self.anims.push(FireAnim::fire(1, rand));
                }
            }
            4 => {
                self.anims.clear();
                self.anims.push(FireAnim::smoke(1, rand));
            }
            _ => {}
        }
    }

    /// `IsoFire.load`'s fields and animations.
    pub fn load(s: &FireSave, tile_scale: i32, rand: &mut dyn GameRand) -> Self {
        let mut f = Self::from_save(s);
        f.attach_loaded_anims(s.life_stage, tile_scale, rand);
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

    /// `IsoFire.update` from the attached animations on (the sound, square
    /// flags, `BurnTick` and heat source are the manager's): `dt` is
    /// `GameTime.getMultipliedSecondsSinceLastUpdate() * 60`,
    /// `thirty_fps` `getThirtyFPSMultiplier()`, `now` the calendar time in
    /// milliseconds and `alpha` the object's alpha for the player
    /// (`getAlpha(playerIndex)`, which the stage-5 fade sets).
    pub fn update_with(
        &mut self,
        dt: f32,
        thirty_fps: f32,
        now: i64,
        alpha: f32,
        rand: &mut dyn GameRand,
        cb: &mut dyn FireCallbacks,
    ) {
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
            // addHeatSource on the first update, setRadius after.
            self.heat_source = Some(self.light_radius);
            return;
        }
        // updateSinceLastLoaded.
        if self.saved_update_time != -1 {
            if self.saved_update_time > now {
                self.saved_update_time = now;
            }
            let delta = (now - self.saved_update_time) as f32 / 30.0;
            self.saved_update_time = -1;
            self.update_from_timer_with(delta, rand, cb);
        }
        self.update_from_timer_with(thirty_fps, rand, cb);
    }

    /// [`IsoFire::update_with`], collecting the events instead of acting on
    /// them.
    pub fn update(
        &mut self,
        dt: f32,
        thirty_fps: f32,
        now: i64,
        alpha: f32,
        rand: &mut dyn GameRand,
    ) -> Vec<FireEvent> {
        let mut events = Vec::new();
        let mut cb = |_: &mut IsoFire, e: FireEvent, _: &mut dyn GameRand| events.push(e);
        self.update_with(dt, thirty_fps, now, alpha, rand, &mut cb);
        events
    }

    /// `updateFromTimer(timer)`: whole ticks of the life-stage machine,
    /// calling `cb` where the original burns the square, spreads or goes
    /// out.
    pub fn update_from_timer_with(
        &mut self,
        timer: f32,
        rand: &mut dyn GameRand,
        cb: &mut dyn FireCallbacks,
    ) {
        self.accum += timer;
        while self.accum > 1.0 {
            self.accum -= 1.0;
            self.age += 1;
            if self.life_stage_timer > 0 {
                self.life_stage_timer -= 1;
                if self.life_stage_timer <= 0 {
                    self.next_stage(rand, cb);
                }
            }
            if self.life <= 0 {
                cb.on_event(self, FireEvent::Extinct, rand);
                break;
            }
            self.life -= 1;
            if self.life_stage > 0 && self.spread_timer > 0 {
                self.spread_timer -= 1;
                if self.spread_timer <= 0 {
                    if self.life_stage != 5 {
                        cb.on_event(self, FireEvent::Spread, rand);
                    }
                    self.spread_timer = self.spread_delay;
                }
            }
            if self.energy <= 0 {
                cb.on_event(self, FireEvent::Extinct, rand);
                break;
            }
        }
    }

    /// [`IsoFire::update_from_timer_with`], collecting the events.
    pub fn update_from_timer(&mut self, timer: f32, rand: &mut dyn GameRand) -> Vec<FireEvent> {
        let mut events = Vec::new();
        let mut cb = |_: &mut IsoFire, e: FireEvent, _: &mut dyn GameRand| events.push(e);
        self.update_from_timer_with(timer, rand, &mut cb);
        events
    }

    fn next_stage(&mut self, rand: &mut dyn GameRand, cb: &mut dyn FireCallbacks) {
        let light = self.light.is_some();
        match self.life_stage {
            0 => {
                self.life_stage = 1;
                self.life_stage_timer = self.life_stage_duration;
                cb.on_event(self, FireEvent::Burn, rand);
                if light {
                    self.set_light_radius_with(LIGHT_RADIUS_LOW, rand, cb);
                }
            }
            1 => {
                self.life_stage = 2;
                self.life_stage_timer = self.life_stage_duration;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
                cb.on_event(self, FireEvent::Burn, rand);
                if light {
                    self.set_light_radius_with(LIGHT_RADIUS_MEDIUM, rand, cb);
                }
            }
            2 => {
                self.life_stage = 3;
                self.life_stage_timer = self.life_stage_duration;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(3, rand));
                self.anims.push(FireAnim::fire(3, rand));
                if light {
                    self.set_light_radius_with(LIGHT_RADIUS_HIGH, rand, cb);
                }
            }
            3 => {
                self.life_stage = 4;
                self.life_stage_timer = self.life_stage_duration / 2;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(2, rand));
                self.anims.push(FireAnim::fire(2, rand));
                if light {
                    self.set_light_radius_with(LIGHT_RADIUS_MEDIUM, rand, cb);
                }
            }
            4 => {
                self.life_stage = 5;
                self.life_stage_timer = self.life_stage_duration / 2;
                self.anims.clear();
                self.anims.push(FireAnim::smoke(1, rand));
                if light {
                    self.set_light_radius_with(LIGHT_RADIUS_MINIMUM, rand, cb);
                }
            }
            _ => {}
        }
    }

    fn set_light_radius_with(
        &mut self,
        radius: i32,
        rand: &mut dyn GameRand,
        cb: &mut dyn FireCallbacks,
    ) {
        if self.replace_light(radius) {
            cb.on_event(self, FireEvent::LightChanged, rand);
        }
    }

    /// `setLightRadius(radius)` without the relight: a different radius
    /// replaces the light, now in `fireColor`. True when it was replaced.
    pub fn replace_light(&mut self, radius: i32) -> bool {
        self.light_radius = radius;
        if let Some(l) = self.light {
            if radius != l.radius {
                let [r, g, b] = FIRE_COLOR;
                self.light = Some(FireLight { r, g, b, radius });
                return true;
            }
        }
        false
    }

    /// `setLightRadius(radius)`, pushing [`FireEvent::LightChanged`] when
    /// the light was replaced.
    pub fn set_light_radius(&mut self, radius: i32, events: &mut Vec<FireEvent>) {
        if self.replace_light(radius) {
            events.push(FireEvent::LightChanged);
        }
    }

    /// `setLifeStage(stage)` for a permanent fire.
    pub fn set_life_stage(&mut self, stage: i32, rand: &mut dyn GameRand) {
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

    /// `isPermanent()`.
    pub fn is_permanent(&self) -> bool {
        self.perm
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
