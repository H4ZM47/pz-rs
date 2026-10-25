//! The non-rendering simulation of `zombie.characters.IsoZombie`:
//! construction from a descriptor (the outfit comes from its owner), the
//! lore-driven stats and speeds, the senses ([`senses`]: sight and
//! hearing through [`crate::world_sound`]), memory and target selection,
//! the update loop ([`update`]), thumping and the zombie animation
//! variables ([`vars`]).
//!
//! The zombie's own fields are [`ZombieFields`] (`GameCharacter::zombie`);
//! the ones the shared combat code also reads (`target`, `crawling`,
//! `becomeCrawler`, `staggerBack`, ...) stay in `CharacterVars`. The fields
//! zombies read and write on the characters they hunt (`targetedByZombie`,
//! `slowFactor`, `movementLastFrame`, ...) are [`SeenFields`]
//! (`GameCharacter::seen`).
//!
//! Everything outside the zombie (squares, light, sounds, doors, corpses,
//! the path finder) goes through the `StateWorld` hooks of the state
//! context; pz-lighting supplies the light levels and pz-world the
//! squares.

pub mod math;
pub mod mover;
pub mod population;
pub mod senses;
pub mod update;
pub mod vars;

use pz_scripts::ScriptManager;
use pz_world::square::IsoDirections;

use super::state::{StateCx, ThumpTarget};
use super::states::ids;
use super::{CharacterKind, GameCharacter, SimContext};
use crate::survivor::SurvivorDescRef;
use math::*;

/// `IsoZombie.SPEED_SPRINTER`.
pub const SPEED_SPRINTER: i32 = 1;
/// `SPEED_FAST_SHAMBLER`.
pub const SPEED_FAST_SHAMBLER: i32 = 2;
/// `SPEED_SHAMBLER`.
pub const SPEED_SHAMBLER: i32 = 3;
/// `SPEED_RANDOM`.
pub const SPEED_RANDOM: i32 = 4;
/// `HEARING_PINPOINT`.
pub const HEARING_PINPOINT: i32 = 1;
/// `HEARING_NORMAL`.
pub const HEARING_NORMAL: i32 = 2;
/// `HEARING_POOR`.
pub const HEARING_POOR: i32 = 3;
/// `EAT_BODY_DIST`.
pub const EAT_BODY_DIST: f32 = 1.0;
/// `EAT_BODY_TIME`.
pub const EAT_BODY_TIME: f32 = 3600.0;
/// `LUNGE_TIME`.
pub const LUNGE_TIME: f32 = 180.0;
/// `CRAWLER_DAMAGE_DOT`.
pub const CRAWLER_DAMAGE_DOT: f32 = 0.9;
/// `CRAWLER_DAMAGE_RANGE`.
pub const CRAWLER_DAMAGE_RANGE: f32 = 1.5;
/// `VISION_RADIUS_MAX`.
pub const VISION_RADIUS_MAX: f32 = 20.0;
/// `VISION_RADIUS_MIN`.
pub const VISION_RADIUS_MIN: f32 = 10.0;
/// `AllowRepathDelayMax`.
pub const ALLOW_REPATH_DELAY_MAX: i32 = 120;
/// The `THUMP_FLAG_*` constants.
pub const THUMP_FLAG_GENERIC: i32 = 1;
pub const THUMP_FLAG_WINDOW_EXTRA: i32 = 2;
pub const THUMP_FLAG_WINDOW: i32 = 3;
pub const THUMP_FLAG_METAL: i32 = 4;
pub const THUMP_FLAG_GARAGE_DOOR: i32 = 5;
pub const THUMP_FLAG_CHAINLINK_FENCE: i32 = 6;
pub const THUMP_FLAG_METAL_POLE_GATE: i32 = 7;
pub const THUMP_FLAG_WOOD: i32 = 8;

/// `IsoZombie.Aggro` (server only): who hurt the zombie and how much.
#[derive(Clone, Debug, PartialEq)]
pub struct Aggro {
    pub obj: i32,
    pub damage: f32,
    /// `GameTime` millis of the last damage.
    pub last_damage: i64,
}

impl Aggro {
    /// `getAggro()`: the damage, fading out between 5 and 10 seconds.
    pub fn aggro(&self, now_millis: i64) -> f32 {
        let dt = (now_millis - self.last_damage) as f32;
        let a = ((10000.0 - dt) / 5000.0).clamp_java(0.0, 1.0);
        (a * self.damage * 0.5).clamp_java(0.0, 1.0)
    }
}

trait ClampJava {
    fn clamp_java(self, lo: f32, hi: f32) -> f32;
}

impl ClampJava for f32 {
    /// `Math.min(hi, Math.max(lo, v))`.
    fn clamp_java(self, lo: f32, hi: f32) -> f32 {
        let m = if self >= lo { self } else { lo };
        if m <= hi {
            m
        } else {
            hi
        }
    }
}

/// The fields of the character a zombie targets that zombies read and
/// write (`IsoMovingObject`, `IsoGameCharacter`, `IsoPlayer`).
#[derive(Clone, Debug, PartialEq)]
pub struct SeenFields {
    /// `getMovementLastFrame()` (from the movement code).
    pub movement_last_frame: (f32, f32),
    /// `IsoPlayer.playerIndex`.
    pub player_index: i32,
    /// `IsoPlayer.remoteId`.
    pub remote_id: i32,
    /// `getTorchStrength()` (the active light item's strength).
    pub torch_strength: f32,
    /// `IsoPlayer.targetedByZombie`, `lastTargeted`.
    pub targeted_by_zombie: bool,
    pub last_targeted: f32,
    /// `couldBeSeenThisFrame`, `seenThisFrame`, `isJustMoved()`.
    pub could_be_seen_this_frame: bool,
    pub seen_this_frame: bool,
    pub just_moved: bool,
    /// `slowTimer`, `slowFactor`.
    pub slow_timer: f32,
    pub slow_factor: f32,
    /// `timeSinceZombieAttack`.
    pub time_since_zombie_attack: i32,
    /// `lastTargettedBy` (a zombie id).
    pub last_targetted_by: Option<i32>,
    /// `reanimatedCorpse` (the zombie id).
    pub reanimated_corpse: Option<i32>,
    /// `checkIsNearWall()` (from the squares).
    pub near_wall: f32,
    /// `getEatingZombies()`.
    pub eating_zombies: Vec<i32>,
    /// `getCurrentBuilding()` by id.
    pub building: Option<i64>,
}

impl Default for SeenFields {
    fn default() -> Self {
        SeenFields {
            movement_last_frame: (0.0, 0.0),
            player_index: 0,
            remote_id: -1,
            torch_strength: 0.0,
            targeted_by_zombie: false,
            last_targeted: 0.0,
            could_be_seen_this_frame: false,
            seen_this_frame: false,
            just_moved: false,
            slow_timer: 0.0,
            slow_factor: 0.0,
            time_since_zombie_attack: 0,
            last_targetted_by: None,
            reanimated_corpse: None,
            near_wall: 1.0,
            eating_zombies: Vec::new(),
            building: None,
        }
    }
}

/// `IsoZombie`'s fields (those not in `CharacterVars`).
#[derive(Clone, Debug, PartialEq)]
pub struct ZombieFields {
    pub no_teeth: bool,
    pub force_eating_animation: bool,
    pub last_target_seen_x: i32,
    pub last_target_seen_y: i32,
    pub last_target_seen_z: i32,
    pub ghost: bool,
    pub lunge_timer: f32,
    pub lunge_sound_time: i64,
    pub target_seen_time: f32,
    pub can_see_target: bool,
    pub follow_count: i32,
    pub zombie_id: i32,
    pub persistent_id: i32,
    /// `persistentOutfitId` (`IsoGameCharacter`; only zombies get one here):
    /// the `PersistentOutfits` id the population manager stores.
    pub persistent_outfit_id: i32,
    pub bonus_spot_time: f32,
    pub knife_death: bool,
    pub jaw_stab_attach: bool,
    pub fake_dead: bool,
    pub force_fake_dead: bool,
    pub was_fake_dead: bool,
    pub reanimate: bool,
    pub reanimated_player: bool,
    pub indoor_zombie: bool,
    pub thump_flag: i32,
    pub thump_sent: bool,
    pub thump_condition: f32,
    pub useless: bool,
    pub speed_type: i32,
    pub inactive: bool,
    pub strength: i32,
    pub cognition: i32,
    pub memory: i32,
    pub sight: i32,
    pub hearing: i32,
    pub voice_choice: i32,
    pub sound_react_delay: f32,
    /// `delayedSound` (x, y, z; x is -1 when none).
    pub delayed_sound: (i32, i32, i32),
    pub sound_source_repeating: bool,
    pub sound_source_is_player: bool,
    pub sound_source_is_player_base: bool,
    /// `soundSourceTarget`: the sound source's id
    /// ([`GameCharacter::sound_source_id`] for characters).
    pub sound_source_target: Option<u64>,
    /// The source of `soundSourceTarget` is a zombie.
    pub sound_source_target_is_zombie: bool,
    pub sound_attract: f32,
    pub sound_attract_timeout: f32,
    pub alerted: bool,
    pub walk_type: Option<String>,
    pub footstep_volume: f32,
    pub dress_in_random_outfit: bool,
    pub pending_outfit_name: Option<String>,
    /// The outfit name (`getHumanVisual().getOutfit().name`), set by the
    /// outfit system.
    pub outfit_name: Option<String>,
    pub crawler_type: i32,
    pub eat_speed: f32,
    pub check_for_corpse_timer: f32,
    /// `bodyToEat` (a corpse id).
    pub body_to_eat: Option<i64>,
    /// `eatBodyTarget`.
    pub eat_body_target: Option<super::state::EatTarget>,
    pub thump_timer: i32,
    pub collide_while_hit: bool,
    pub character_texture_anim_time: f32,
    pub character_texture_anim_duration: f32,
    pub last_player_hit: i32,
    pub vision_radius_result: f32,
    pub attack_did_damage: bool,
    pub attack_outcome: String,
    pub reanimated_for_grapple_only: bool,
    /// `spottedLast` (by id).
    pub spotted_last: Option<i32>,
    pub spot_sound_delay: i32,
    pub movex: f32,
    pub movey: f32,
    pub online_id: i16,
    pub sprite_name: String,
    pub vector_to_target: (f32, f32),
    pub allow_repath_delay: f32,
    pub keep_it_real: bool,
    pub is_skeleton: bool,
    pub scratch: bool,
    pub laceration: bool,
    pub bloodsplat_amount: i32,
    pub time_since_respond_to_sound: f32,
    pub walk_variant_use: Option<String>,
    pub walk_variant: String,
    pub lunger: bool,
    /// `IsoZombie.running` (hides `IsoGameCharacter.running`).
    pub running: bool,
    pub can_crawl_under_vehicle: bool,
    pub can_walk: bool,
    pub remote: bool,
    pub immortal_tutorial_zombie: bool,
    pub palette: i32,
    pub aggro_list: [Option<Aggro>; 4],
    pub unbalanced_level: f32,
    pub female: bool,
    /// `IsoGameCharacter.thumpTarget`.
    pub thump_target: Option<ThumpTarget>,
    /// `IsoGameCharacter.timeThumping`.
    pub time_thumping: i32,
    /// `IsoGameCharacter.lastHeardSound`.
    pub last_heard_sound: (i32, i32, i32),
    /// `hurtSound`.
    pub hurt_sound: String,
    /// `weight`, `width`.
    pub weight: f32,
    pub width: f32,
    /// `getWornItemsVisionModifier()` / `getWornItemsHearingModifier()`
    /// (from the item visuals the outfit gives).
    pub worn_items_vision_modifier: f32,
    pub worn_items_hearing_modifier: f32,
    /// `parameterZombieState` (the FMOD parameter's state name).
    pub sound_state: &'static str,
    /// `sprite.def.animFrameIncrease` and `setFrameSpeedPerFrame` (the
    /// legacy sprite animation the speed code still scales).
    pub anim_frame_increase: f32,
    pub frame_speed_per_frame: f32,
    /// `isBeingSteppedOn()`, `isUnderVehicle()` inputs.
    pub being_stepped_on: bool,
    /// The `turnalertedvalue` direction set by `setTurnAlertedValues`.
    pub turn_alerted_value: String,
    /// Getter side effects not yet applied (`setTarget(null)` from the
    /// `bHasTarget`/`battack`/`blunge` slots).
    /// The `bthump` slot found the thump target gone (or the thump timer
    /// running): `setThumpTarget(null)`, applied with the other getter
    /// effects.
    pub(crate) thump_clear_requested: std::cell::Cell<bool>,
    pub(crate) clear_target_requested: std::cell::Cell<bool>,
    /// `lungeTimer = 0` from the `blunge` slot.
    pub(crate) lunge_timer_reset_requested: std::cell::Cell<bool>,
    /// What the variable slots read about the target (refreshed before the
    /// action context runs).
    pub target_view: vars::TargetView,
}

impl Default for ZombieFields {
    fn default() -> Self {
        ZombieFields {
            no_teeth: false,
            force_eating_animation: false,
            last_target_seen_x: -1,
            last_target_seen_y: -1,
            last_target_seen_z: -1,
            ghost: false,
            lunge_timer: 0.0,
            lunge_sound_time: 0,
            target_seen_time: 0.0,
            can_see_target: false,
            follow_count: 0,
            zombie_id: 0,
            persistent_id: 0,
            persistent_outfit_id: 0,
            bonus_spot_time: 0.0,
            knife_death: false,
            jaw_stab_attach: false,
            fake_dead: false,
            force_fake_dead: false,
            was_fake_dead: false,
            reanimate: false,
            reanimated_player: false,
            indoor_zombie: false,
            thump_flag: 0,
            thump_sent: false,
            thump_condition: 1.0,
            useless: false,
            speed_type: -1,
            inactive: false,
            strength: -1,
            cognition: -1,
            memory: -1,
            sight: -1,
            hearing: -1,
            voice_choice: -1,
            sound_react_delay: 0.0,
            delayed_sound: (-1, -1, -1),
            sound_source_repeating: false,
            sound_source_is_player: false,
            sound_source_is_player_base: false,
            sound_source_target: None,
            sound_source_target_is_zombie: false,
            sound_attract: 0.0,
            sound_attract_timeout: 0.0,
            alerted: false,
            walk_type: None,
            footstep_volume: 1.0,
            dress_in_random_outfit: false,
            pending_outfit_name: None,
            outfit_name: None,
            crawler_type: 0,
            eat_speed: 1.0,
            check_for_corpse_timer: 10000.0,
            body_to_eat: None,
            eat_body_target: None,
            thump_timer: 0,
            collide_while_hit: true,
            character_texture_anim_time: 0.0,
            character_texture_anim_duration: 1.0,
            last_player_hit: -1,
            vision_radius_result: 20.0,
            attack_did_damage: false,
            attack_outcome: String::new(),
            reanimated_for_grapple_only: false,
            spotted_last: None,
            spot_sound_delay: 0,
            movex: 0.0,
            movey: 0.0,
            online_id: -1,
            sprite_name: "BobZ".to_string(),
            vector_to_target: (0.0, 0.0),
            allow_repath_delay: 0.0,
            keep_it_real: false,
            is_skeleton: false,
            scratch: false,
            laceration: false,
            bloodsplat_amount: 1000,
            time_since_respond_to_sound: 1_000_000.0,
            walk_variant_use: None,
            walk_variant: "ZombieWalk".to_string(),
            lunger: false,
            running: false,
            can_crawl_under_vehicle: true,
            can_walk: true,
            remote: false,
            immortal_tutorial_zombie: false,
            palette: 1,
            aggro_list: [None, None, None, None],
            unbalanced_level: 0.0,
            female: false,
            thump_target: None,
            time_thumping: 0,
            last_heard_sound: (-1, -1, -1),
            hurt_sound: String::new(),
            weight: 1.0,
            width: 0.25,
            worn_items_vision_modifier: 1.0,
            worn_items_hearing_modifier: 1.0,
            sound_state: "Idle",
            anim_frame_increase: 1.0,
            frame_speed_per_frame: 0.0,
            being_stepped_on: false,
            turn_alerted_value: String::new(),
            clear_target_requested: std::cell::Cell::new(false),
            thump_clear_requested: std::cell::Cell::new(false),
            lunge_timer_reset_requested: std::cell::Cell::new(false),
            target_view: vars::TargetView::default(),
        }
    }
}

/// What a new zombie is built from (`new IsoZombie(cell, desc, palette)`).
pub struct ZombieSpawn<'a, I: 'static> {
    /// The descriptor and palette; `None` makes one with
    /// `create_survivor` (`SurvivorFactory.CreateSurvivor()`) and draws the
    /// palette.
    pub descriptor: Option<(SurvivorDescRef<I>, i32)>,
    /// `SurvivorFactory.CreateSurvivor()`; `Err` is the exception it
    /// throws (an empty name list: `IndexOutOfBoundsException`).
    pub create_survivor: &'a mut dyn FnMut() -> pz_scripts::ScriptResult<SurvivorDescRef<I>>,
}

impl<I: 'static> GameCharacter<I> {
    /// `new IsoZombie(cell, desc, palette)`: the simulation part of the
    /// constructor, in its `Rand` order (health, forward direction, blood,
    /// descriptor and palette, the zombie id, sprite tints, stats and
    /// speed, `initCanCrawlUnderVehicle`). The action group is the
    /// caller's (`ActionGroup.getActionGroup("zombie")`);
    /// [`Self::initialize_zombie_states`] registers the states.
    ///
    /// The zombie gets its `HumanVisual` (`randomBlood()` draws from
    /// `OutfitRNG`). `Err` is the exception `CreateSurvivor()` threw.
    pub fn new_zombie(sm: &ScriptManager, ctx: &dyn SimContext, spawn: ZombieSpawn<'_, I>) -> pz_scripts::ScriptResult<Self> {
        let mut z = GameCharacter::new(sm, CharacterKind::Zombie);
        z.health = 1.8 + ctx.rand_next_float(0.0, 0.3);
        z.zombie.weight = 0.7;
        let dir = IsoDirections::ALL[ctx.rand_next(8) as usize];
        z.set_forward_iso_direction(dir);
        let mut hv = pz_inventory::visual::HumanVisual::new(crate::character::items::blood_part::MAX);
        hv.random_blood(&crate::population::OutfitRngStyles);
        z.human_visual = Some(hv);
        match spawn.descriptor {
            Some((d, palette)) => {
                z.descriptor = Some(d);
                z.zombie.palette = palette;
            }
            None => {
                z.descriptor = Some((spawn.create_survivor)()?);
                z.zombie.palette = ctx.rand_next(3) + 1;
            }
        }
        z.zombie.female = z.descriptor.as_ref().is_some_and(|d| d.borrow().is_female());
        let female = z.zombie.female;
        let prefix = z.voice_prefix();
        if !prefix.contains("Zombie") {
            let p = if prefix == "VoiceFemale" { "FemaleZombie" } else { "MaleZombie" };
            z.set_voice_prefix(p);
        }
        z.zombie.sprite_name = if female { "KateZ" } else { "BobZ" }.to_string();
        if z.zombie.palette != 1 {
            z.zombie.sprite_name = format!("{}{}", z.zombie.sprite_name, z.zombie.palette);
        }
        // InitSpritePartsZombie.
        z.zombie.zombie_id = ctx.rand_next(10000);
        // The sprite's tints.
        for _ in 0..3 {
            ctx.rand_next(5);
        }
        z.set_fake_dead(ctx, false);
        z.zombie.inactive = ctx.is_zombie_inactivity_phase();
        z.do_zombie_stats(ctx);
        z.zombie.width = 0.3;
        z.zombie.hurt_sound = format!("{}Hurt", z.voice_prefix());
        z.clear_aggro_list();
        Ok(z)
    }

    /// `getDescriptor().getVoicePrefix()`.
    pub fn voice_prefix(&self) -> String {
        self.descriptor.as_ref().and_then(|d| d.borrow().voice_prefix.clone()).unwrap_or_else(|| "null".into())
    }

    fn set_voice_prefix(&mut self, p: &str) {
        if let Some(d) = &self.descriptor {
            d.borrow_mut().voice_prefix = Some(p.to_string());
        }
    }

    /// The stable id this character has as a sound source
    /// (`pz_audio::world_sound::SoundSource::id`): its mover id.
    pub fn sound_source_id(&self) -> u64 {
        use pz_pathfind::mover::PathMover;
        self.mover_id().0
    }

    /// This character as a sound source.
    pub fn sound_source(&self) -> pz_audio::world_sound::SoundSource {
        pz_audio::world_sound::SoundSource {
            id: self.sound_source_id(),
            is_player: self.is_player(),
            is_zombie: self.is_zombie(),
            is_animal: self.is_animal(),
            ..Default::default()
        }
    }

    /// `setForwardIsoDirection(dir)`.
    pub fn set_forward_iso_direction(&mut self, dir: IsoDirections) {
        let (x, y) = dir_vector(dir);
        self.set_forward_direction(x, y);
        self.vars.dir = dir;
    }

    /// `getForwardIsoDirection()`.
    pub fn forward_iso_direction(&self) -> IsoDirections {
        self.vars.dir
    }

    // ---- lore ----

    /// `DoZombieStats()`.
    pub fn do_zombie_stats(&mut self, ctx: &dyn SimContext) {
        let z = &mut self.zombie;
        if ctx.lore_cognition() == 1 {
            z.cognition = 1;
        }
        if ctx.lore_cognition() == 4 {
            z.cognition = if ctx.rand_next(100) < ctx.lore_door_opening_percentage() { 1 } else { 0 };
        }
        let strength = ctx.lore_strength();
        if z.strength == -1 && strength == 1 {
            z.strength = 5;
        }
        if z.strength == -1 && strength == 2 {
            z.strength = 3;
        }
        if z.strength == -1 && strength == 3 {
            z.strength = 1;
        }
        if z.strength == -1 && strength == 4 {
            z.strength = ctx.rand_next_range(1, 5);
        }
        let memory = ctx.lore_memory();
        let mut random_memory = -1;
        if memory == 5 {
            random_memory = ctx.rand_next(4);
        } else if memory == 6 {
            random_memory = ctx.rand_next(3) + 1;
        }
        if z.memory == -1 && memory == 1 || random_memory == 0 {
            z.memory = 1250;
        }
        if z.memory == -1 && memory == 2 || random_memory == 1 {
            z.memory = 800;
        }
        if z.memory == -1 && memory == 3 || random_memory == 2 {
            z.memory = 500;
        }
        if z.memory == -1 && memory == 4 || random_memory == 3 {
            z.memory = 25;
        }
        z.sight = match ctx.lore_sight() {
            4 => ctx.rand_next(3) + 1,
            5 => ctx.rand_next(2) + 2,
            v => v,
        };
        z.hearing = match ctx.lore_hearing() {
            4 => ctx.rand_next(3) + 1,
            5 => ctx.rand_next(2) + 2,
            v => v,
        };
        self.do_zombie_speed(ctx);
        self.init_can_crawl_under_vehicle(ctx);
    }

    /// `initCanCrawlUnderVehicle()`.
    pub fn init_can_crawl_under_vehicle(&mut self, ctx: &dyn SimContext) {
        let chance = match ctx.lore_crawl_under_vehicle() {
            1 => 0,
            2 => 5,
            3 => 10,
            4 => 25,
            5 => 50,
            6 => 75,
            _ => 100,
        };
        self.zombie.can_crawl_under_vehicle = ctx.rand_next(100) < chance;
    }

    /// `doZombieSpeed()`.
    pub fn do_zombie_speed(&mut self, ctx: &dyn SimContext) {
        let s = self.zombie.speed_type;
        self.do_zombie_speed_internal(ctx, s);
    }

    /// `doZombieSpeed(zombieSpeed)`.
    pub fn do_zombie_speed_with(&mut self, ctx: &dyn SimContext, speed: i32) {
        self.do_zombie_speed_internal(ctx, speed);
    }

    fn do_zombie_speed_internal(&mut self, ctx: &dyn SimContext, speed: i32) {
        let speed = self.determine_zombie_speed(ctx, speed);
        let lore = ctx.lore_speed();
        if self.vars.crawling {
            self.do_crawler_speed(ctx, speed);
        } else if lore == 3 || speed == 3 {
            self.do_shambler(ctx);
        } else if ctx.rand_next(3) != 0 {
            self.do_fake_shambler(ctx, speed);
        } else if lore == 2 || speed == 2 {
            self.do_fast_shambler(ctx);
        } else if lore == 1 || speed == 1 {
            self.do_sprinter(ctx);
        }
    }

    fn determine_zombie_speed(&self, ctx: &dyn SimContext, speed: i32) -> i32 {
        if speed != -1 {
            return speed;
        }
        let mut speed = ctx.lore_speed();
        if self.zombie.inactive {
            speed = 3;
        } else if speed == 4 {
            speed = if ctx.rand_next(100) < ctx.lore_sprinter_percentage() { 1 } else { ctx.rand_next(2) + 2 };
        }
        speed
    }

    fn speed_mod_draw(&mut self, ctx: &dyn SimContext, base: f32) {
        self.vars.speed_mod = base;
        self.vars.speed_mod += ctx.rand_next(1500) as f32 / 10000.0;
    }

    /// `doCrawlerSpeed(zombieSpeed)`.
    pub fn do_crawler_speed(&mut self, ctx: &dyn SimContext, speed: i32) {
        self.speed_mod_draw(ctx, 0.3);
        self.zombie.anim_frame_increase *= 0.8;
        self.do_zombie_speed_internal2(ctx, speed);
    }

    fn walk_setup(&mut self, variant: &str) {
        self.zombie.walk_variant = variant.to_string();
        self.zombie.frame_speed_per_frame = 0.24;
        self.zombie.anim_frame_increase *= self.vars.speed_mod;
    }

    /// `doSprinter()`.
    pub fn do_sprinter(&mut self, ctx: &dyn SimContext) {
        self.zombie.lunger = true;
        self.vars.speed_mod = 0.85;
        self.zombie.walk_variant = "ZombieWalk2".into();
        self.vars.speed_mod += ctx.rand_next(1500) as f32 / 10000.0;
        self.walk_setup("ZombieWalk2");
        self.zombie.speed_type = 1;
        self.do_zombie_speed_internal2(ctx, 1);
    }

    /// `doFastShambler()`.
    pub fn do_fast_shambler(&mut self, ctx: &dyn SimContext) {
        self.zombie.lunger = true;
        self.vars.speed_mod = 0.85;
        self.vars.speed_mod += ctx.rand_next(1500) as f32 / 10000.0;
        self.walk_setup("ZombieWalk2");
        self.zombie.speed_type = 2;
        self.do_zombie_speed_internal2(ctx, 2);
    }

    /// `doFakeShambler(zombieSpeed)`.
    pub fn do_fake_shambler(&mut self, ctx: &dyn SimContext, speed: i32) {
        self.speed_mod_draw(ctx, 0.55);
        self.walk_setup("ZombieWalk1");
        self.do_zombie_speed_internal2(ctx, speed);
    }

    /// `doShambler()`.
    pub fn do_shambler(&mut self, ctx: &dyn SimContext) {
        self.speed_mod_draw(ctx, 0.55);
        self.walk_setup("ZombieWalk1");
        self.zombie.speed_type = 3;
        self.do_zombie_speed_internal2(ctx, 3);
    }

    fn do_zombie_speed_internal2(&mut self, ctx: &dyn SimContext, speed: i32) {
        if speed == 3 {
            self.zombie.walk_type = Some(format!("slow{}", ctx.rand_next(3) + 1));
        } else {
            self.zombie.walk_type = Some((ctx.rand_next(5) + 1).to_string());
        }
        if speed == 1 {
            self.vars.turn_delta_normal = 1.0;
            self.zombie.walk_type = Some(format!("sprint{}", self.zombie.walk_type.as_deref().unwrap_or("")));
        }
        self.zombie.speed_type = speed;
    }

    /// `getSpeedTypeFromWalkType(walkType)`.
    pub fn speed_type_from_walk_type(walk_type: Option<&str>) -> i32 {
        match walk_type {
            None => 2,
            Some(w) if w.trim().is_empty() => 2,
            Some(w) if w.starts_with("sprint") => 1,
            Some(w) if w.starts_with("slow") => 3,
            _ => 2,
        }
    }

    /// `DoZombieSpeeds(spMod)` (the old speed setup Lua still calls).
    pub fn do_zombie_speeds(&mut self, ctx: &dyn SimContext, sp_mod: f32) {
        if self.vars.crawling {
            self.vars.speed_mod = sp_mod;
            self.zombie.anim_frame_increase *= 0.8;
        } else if ctx.rand_next(3) != 0 || ctx.lore_speed() == 3 {
            self.vars.speed_mod = sp_mod;
            self.vars.speed_mod += ctx.rand_next(1500) as f32 / 10000.0;
            let v = format!("{}1", self.zombie.walk_variant);
            self.walk_setup(&v);
        } else if ctx.lore_speed() != 3 {
            self.zombie.lunger = true;
            self.vars.speed_mod = sp_mod;
            let v = format!("{}2", self.zombie.walk_variant);
            self.walk_setup(&v);
        }
    }

    /// `makeInactive(binactive)`.
    pub fn make_inactive(&mut self, ctx: &dyn SimContext, inactive: bool) {
        if inactive == self.zombie.inactive {
            return;
        }
        if inactive {
            self.zombie.walk_type = Some(format!("slow{}", ctx.rand_next(3) + 1));
            self.zombie.running = false;
            self.zombie.inactive = true;
            self.zombie.speed_type = 3;
            self.do_zombie_speed(ctx);
        } else {
            self.zombie.speed_type = -1;
            self.zombie.inactive = false;
            self.do_zombie_stats(ctx);
        }
    }

    /// `getSandboxMemoryDuration()`.
    pub fn sandbox_memory_duration(&self, ctx: &dyn SimContext) -> i32 {
        let mut memory = 160;
        if self.zombie.inactive {
            memory = 5;
        } else {
            match ctx.lore_memory() {
                1 => memory = 250,
                3 => memory = 100,
                4 => memory = 5,
                _ => {}
            }
        }
        memory * 5
    }

    // ---- simple state ----

    /// `isFakeDead()`: never with `disableFakeDead` 3 or on stairs.
    pub fn is_fake_dead(&mut self, ctx: &dyn SimContext, on_stairs: bool) -> bool {
        if ctx.lore_disable_fake_dead() == 3 {
            self.zombie.fake_dead = false;
        }
        if on_stairs && self.zombie.fake_dead {
            self.zombie.fake_dead = false;
        }
        self.zombie.fake_dead
    }

    /// `setFakeDead(b)` (a square with stairs is the caller's: use
    /// [`Self::set_fake_dead_on`]).
    pub fn set_fake_dead(&mut self, ctx: &dyn SimContext, fake_dead: bool) {
        self.set_fake_dead_on(ctx, fake_dead, false);
    }

    /// `setFakeDead(b)` with `getSquare().HasStairs()`.
    pub fn set_fake_dead_on(&mut self, ctx: &dyn SimContext, fake_dead: bool, on_stairs: bool) {
        if on_stairs {
            self.zombie.fake_dead = false;
            return;
        }
        if fake_dead && ctx.rand_next(2) == 0 {
            self.zombie.crawler_type = 2;
        }
        self.zombie.fake_dead = fake_dead;
    }

    /// `setBecomeCrawler(b)`: not when invulnerable.
    pub fn set_become_crawler(&mut self, ctx: &dyn SimContext, crawler: bool) {
        if !self.is_invulnerable(ctx) {
            self.vars.become_crawler = crawler;
        }
    }

    /// `setTarget(t)`.
    pub fn set_zombie_target(&mut self, t: Option<i32>) {
        self.vars.target = t;
    }

    /// `isTargetLocationKnown()`.
    pub fn is_target_location_known(&self) -> bool {
        if self.vars.target.is_none() {
            return false;
        }
        if self.zombie.bonus_spot_time > 0.0 {
            return true;
        }
        self.vars.time_since_seen_flesh < 1.0
    }

    /// `toggleCrawling()`.
    pub fn toggle_crawling(&mut self, ctx: &dyn SimContext) {
        let can = self.zombie.can_crawl_under_vehicle;
        if self.vars.crawling {
            self.vars.crawling = false;
            self.vars.knocked_down = false;
            self.vars.stagger_back = false;
            self.environment.fall_on_front = false;
            self.vars.on_floor = false;
            self.do_zombie_stats(ctx);
        } else {
            self.vars.crawling = true;
            self.vars.on_floor = true;
            self.do_zombie_stats(ctx);
            self.zombie.walk_variant = "ZombieWalk".into();
        }
        self.zombie.can_crawl_under_vehicle = can;
    }

    /// `knockDown(hitFromBehind)`.
    pub fn zombie_knock_down(&mut self, hit_from_behind: bool) {
        self.vars.knocked_down = true;
        self.vars.stagger_back = true;
        self.vars.hit_reaction = String::new();
        self.vars.player_attack_position = hit_from_behind.then(|| "BEHIND".to_string());
        self.vars.hit_force = 1.0;
        self.report_event("wasHit");
    }

    /// `getVoiceChoice()`.
    pub fn voice_choice(&mut self, ctx: &dyn SimContext) -> i32 {
        let z = &mut self.zombie;
        if z.speed_type == -1 {
            return 1;
        }
        if z.voice_choice == -1 {
            z.voice_choice = ctx.rand_next(if z.speed_type == 1 { 2 } else { 3 }) + 1;
        }
        if z.speed_type == 1 && z.voice_choice == 3 {
            z.voice_choice = ctx.rand_next(2) + 1;
        }
        z.voice_choice
    }

    fn voice_name(&mut self, ctx: &dyn SimContext, kind: &str) -> String {
        let p1 = self.voice_prefix();
        let p2 = if self.zombie.speed_type == 1 { "Sprinter" } else { "" };
        let p4 = match self.voice_choice(ctx) {
            2 => "B",
            3 => "C",
            _ => "A",
        };
        format!("{p1}{p2}{kind}{p4}")
    }

    /// `getVoiceSoundName()`.
    pub fn voice_sound_name(&mut self, ctx: &dyn SimContext) -> String {
        self.voice_name(ctx, "Voice")
    }

    /// `getBiteSoundName()`.
    pub fn bite_sound_name(&mut self, ctx: &dyn SimContext) -> String {
        self.voice_name(ctx, "Bite")
    }

    /// `setThumpCondition(float)`.
    pub fn set_thump_condition(&mut self, c: f32) {
        self.zombie.thump_condition = c.clamp(0.0, 1.0);
    }

    /// `setThumpCondition(condition, maxCondition)`.
    pub fn set_thump_condition_of(&mut self, condition: i32, max: i32) {
        if max <= 0 {
            self.zombie.thump_condition = 0.0;
            return;
        }
        let c = condition.clamp(0, max);
        self.zombie.thump_condition = c as f32 / max as f32;
    }

    /// `getWornItemsHearingMultiplier()`.
    pub fn worn_items_hearing_multiplier(&self) -> f32 {
        1.0 / self.zombie.worn_items_hearing_modifier
    }

    /// `getWornItemsVisionMultiplier()`.
    pub fn worn_items_vision_multiplier(&self) -> f32 {
        1.0 / self.zombie.worn_items_vision_modifier
    }

    /// `getTurnDelta()` for a zombie: `turnDeltaNormal`.
    pub fn zombie_turn_delta(&self) -> f32 {
        self.vars.turn_delta_normal
    }

    /// `isZombieAttacking()`: the current state attacks.
    pub fn is_zombie_attacking(&self) -> bool {
        self.current_state().is_some_and(|s| s.is_attacking(self))
    }

    /// `isZombieAttacking(other)`.
    pub fn is_zombie_attacking_target(&self, other: i32) -> bool {
        self.vars.target == Some(other) && self.is_current_state_id(ids::ATTACK)
    }

    /// `isTargetInCone(dist, dot)` with the target at `(tx, ty)`.
    pub fn is_target_in_cone(&self, target: Option<(f32, f32)>, dist: f32, dot: f32) -> bool {
        let Some((tx, ty)) = target else { return false };
        let (dx, dy) = (tx - self.vars.x, ty - self.vars.y);
        let len = length(dx, dy);
        if len == 0.0 {
            return true;
        }
        if len > dist {
            return false;
        }
        let (nx, ny) = normalize(dx, dy);
        let (fx, fy) = vector_from_direction(self.vars.dir);
        nx * fx + ny * fy >= dot
    }

    /// `getLookVector()`: the forward direction's angle (no animation
    /// player).
    pub fn look_vector_exact(&self) -> (f32, f32) {
        from_direction(self.direction_angle_radians(), 1.0)
    }

    /// `isFacingLocation(x, y, dot)`.
    pub fn is_facing_location(&self, x: f32, y: f32, dot: f32) -> bool {
        let (vx, vy) = normalize(x - self.vars.x, y - self.vars.y);
        let (lx, ly) = self.look_vector_exact();
        vx * lx + vy * ly >= dot
    }

    /// `isFacingTarget()` with the target at `(tx, ty)`.
    pub fn is_facing_target_at(&self, target: Option<(f32, f32)>) -> bool {
        let Some((tx, ty)) = target else { return false };
        let (nx, ny) = normalize(tx - self.vars.x, ty - self.vars.y);
        if length(nx, ny) == 0.0 {
            return true;
        }
        let (lx, ly) = self.look_vector_exact();
        nx * lx + ny * ly >= 0.8
    }

    /// `DistTo(x, y)` (Manhattan, as the original).
    pub fn dist_to_xy(&self, x: f32, y: f32) -> f32 {
        distance_manhatten(self.vars.x, self.vars.y, x, y)
    }

    /// `DistToSquared(x, y)`.
    pub fn dist_to_squared_xy(&self, x: f32, y: f32) -> f32 {
        distance_to_squared(x, y, self.vars.x, self.vars.y)
    }

    /// `getCurrentSquare()` from the position (the world keeps squares
    /// where characters stand).
    pub fn current_square_xyz(&self) -> Option<(i32, i32, i32)> {
        self.path.current_square
    }

    // ---- aggro (server) ----

    /// `clearAggroList()`.
    pub fn clear_aggro_list(&mut self) {
        self.zombie.aggro_list = [None, None, None, None];
    }

    /// `addAggro(other, damage)`.
    pub fn add_aggro(&mut self, other: i32, damage: f32, now_millis: i64) {
        let l = &mut self.zombie.aggro_list;
        if l[0].is_none() {
            l[0] = Some(Aggro { obj: other, damage, last_damage: now_millis });
            return;
        }
        for a in l.iter_mut().flatten() {
            if a.obj == other {
                a.damage += damage;
                a.last_damage = now_millis;
                return;
            }
        }
        for a in l.iter_mut() {
            if a.is_none() {
                *a = Some(Aggro { obj: other, damage, last_damage: now_millis });
                return;
            }
        }
    }

    /// `isLeadAggro(other)`.
    pub fn is_lead_aggro(&mut self, other: i32, now_millis: i64) -> bool {
        if self.zombie.aggro_list[0].is_none() {
            return false;
        }
        for a in self.zombie.aggro_list.iter_mut() {
            if a.as_ref().is_some_and(|x| x.aggro(now_millis) <= 0.0) {
                *a = None;
                break;
            }
        }
        let Some(first) = &self.zombie.aggro_list[0] else { return false };
        let mut lead = first.obj;
        let mut lead_aggro = first.aggro(now_millis);
        for a in self.zombie.aggro_list[1..].iter().flatten() {
            let v = a.aggro(now_millis);
            if lead_aggro >= 1.0 && v >= 1.0 {
                return false;
            }
            if lead_aggro < v {
                lead = a.obj;
                lead_aggro = v;
            }
        }
        other == lead && lead_aggro == 1.0
    }

    /// `pathToCharacter(target)` with `IsoZombie`'s repath delay, then
    /// `pathToAux`.
    pub fn zombie_path_to_character(&mut self, target: i32, cx: &mut StateCx<'_, I>) {
        if self.zombie_repath_blocked() {
            return;
        }
        let Some((tx, ty, tz, mid)) = cx.world.character_mut(target).map(|t| (t.vars.x, t.vars.y, t.vars.z, t.mover_id_of())) else {
            return;
        };
        cx.world.path_to_character(self, mid);
        self.path_to_aux(tx, ty, tz, cx);
    }

    /// `pathToLocation(x, y, z)` (no repath delay in the original).
    pub fn zombie_path_to_location(&mut self, x: i32, y: i32, z: i32, cx: &mut StateCx<'_, I>) {
        cx.world.path_to_location(self, x, y, z);
        self.path_to_aux(x as f32 + 0.5, y as f32 + 0.5, z as f32, cx);
    }

    /// `pathToLocationF(x, y, z)` with the repath delay.
    pub fn zombie_path_to_location_f(&mut self, x: f32, y: f32, z: f32, cx: &mut StateCx<'_, I>) {
        if self.zombie_repath_blocked() {
            return;
        }
        cx.world.path_to_location_f(self, x, y, z);
        self.path_to_aux(x, y, z, cx);
    }

    /// `pathToSound(x, y, z)`.
    pub fn zombie_path_to_sound(&mut self, x: i32, y: i32, z: i32, cx: &mut StateCx<'_, I>) {
        cx.world.path_to_sound(self, x, y, z);
        self.path_to_aux(x as f32 + 0.5, y as f32 + 0.5, z as f32, cx);
    }

    fn zombie_repath_blocked(&self) -> bool {
        self.zombie.allow_repath_delay > 0.0
            && (self.is_current_state_id(ids::PATH_FIND)
                || self.is_current_state_id(ids::WALK_TOWARD)
                || self.is_current_state_id(ids::WALK_TOWARD_NETWORK))
    }

    /// `pathToAux(x, y, z)`: walk straight when the line is clear, else
    /// path find.
    pub fn path_to_aux(&mut self, x: f32, y: f32, z: f32, cx: &mut StateCx<'_, I>) {
        let mut clear = true;
        if fastfloor(z) == fastfloor(self.vars.z) && distance_manhatten(x, y, self.vars.x, self.vars.y) <= 30.0 {
            let (cx_, cy_) = (fastfloor(x) / 8, fastfloor(y) / 8);
            if cx.world.chunk_loaded(cx_, cy_) {
                let mut flags = 1 | 2;
                if self.is_animal() {
                    flags &= !1;
                }
                if !self.is_zombie() {
                    flags |= 4;
                }
                let ignore = cx.world.path_behavior(self.id).and_then(|b| b.target_char()).map(|m| m.0 as u32 as i32);
                clear = !cx.world.line_clear_collide(self.vars.x, self.vars.y, x, y, fastfloor(z), ignore, flags);
            }
        }
        if clear {
            if let Some(sq) = self.current_square_xyz() {
                let stairs = cx.world.square(sq.0, sq.1, sq.2).is_some_and(|s| s.has_stairs);
                if stairs && !cx.world.same_staircase(sq, fastfloor(x), fastfloor(y), fastfloor(z)) {
                    clear = false;
                }
            }
        }
        let can_use = !cx.ctx.is_server();
        if clear {
            if can_use {
                self.set_var_bool("bPathfind", false);
            }
            self.vars.moving = true;
        } else {
            if can_use {
                self.set_var_bool("bPathfind", true);
            }
            self.vars.moving = false;
        }
    }

    /// `getPathTargetX/Y/Z()`.
    pub fn path_target(&self, cx: &StateCx<'_, I>) -> (i32, i32, i32) {
        cx.world
            .path_behavior(self.id)
            .map_or((0, 0, 0), |b| (fastfloor(b.target_x()), fastfloor(b.target_y()), fastfloor(b.target_z())))
    }

    /// `getPathFindBehavior2().getTargetX/Y()`.
    pub fn path_target_f(&self, cx: &StateCx<'_, I>) -> (f32, f32, f32) {
        cx.world.path_behavior(self.id).map_or((0.0, 0.0, 0.0), |b| (b.target_x(), b.target_y(), b.target_z()))
    }

    /// `getPathFindBehavior2().getTargetChar()` by id.
    pub fn path_target_char(&self, cx: &StateCx<'_, I>) -> Option<i32> {
        cx.world.path_behavior(self.id).and_then(|b| b.target_char()).map(|m| m.0 as u32 as i32)
    }

    /// `getPathFindBehavior2().isGoalSound()`.
    pub fn path_goal_sound(&self, cx: &StateCx<'_, I>) -> bool {
        cx.world.path_behavior(self.id).is_some_and(|b| b.is_goal_sound())
    }

    /// `getPathFindBehavior2().isGoalLocation()`.
    pub fn path_goal_location(&self, cx: &StateCx<'_, I>) -> bool {
        cx.world.path_behavior(self.id).is_some_and(|b| b.is_goal_location())
    }

    /// `setPath2(null)`.
    pub fn clear_path2(&mut self) {
        self.path.path2 = None;
    }
}


#[cfg(test)]
mod tests;
