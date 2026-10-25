//! `zombie.characters.animals.IsoAnimal` on [`GameCharacter`]: the
//! character side of an animal. The animal logic that does not need the
//! character runtime (`AnimalData`, genome, stress, babies, eggs, the save
//! record) is `pz-animals`' [`Animal`]; this module keeps it in
//! [`GameCharacter::animal`] and gives `pz-animals` the character through
//! [`AnimalCharacter`].
//!
//! * [`AnimalFields`]: `IsoAnimal`'s fields (the live [`Animal`] plus what
//!   the character side caches of its definition).
//! * [`update`]: `IsoAnimal.update` (`updateInternal`, then
//!   `IsoPlayer.updateInternal1`'s animal branch).
//! * [`save`]: `IsoAnimal.load`/`save` (no `IsoPlayer` part).
//! * The `IsoAnimal` overrides of character methods are `is_animal()`
//!   branches calling the `animal_*` methods here.
//!
//! The world reaches the animal through [`StateWorld::animal_host`]
//! ([`AnimalHost`]): the definition tables and `pz-animals`' [`AnimalWorld`]
//! bound to the animal.
//!
//! * [`behavior`]: `BaseAnimalBehavior` (its fields are `pz-animals`'
//!   `AnimalBehavior` on the animal).
//! * [`actions`]: the `IsoAnimal` methods the behaviour and the states
//!   call (path finding, stopping, luring, tree, mother, hook, sound,
//!   thumping, death, alerting).
//! * The twelve `ai/states/animals` states are
//!   [`crate::character::states::animal`].
//!
//! [`StateWorld::animal_host`]: crate::character::state::StateWorld::animal_host

pub mod actions;
pub mod behavior;
pub mod save;
pub mod update;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use pz_animals::env::{CharacterConstruction, CharacterValues};
use pz_animals::{
    Animal, AnimalBreed, AnimalCharacter, AnimalClock, AnimalCtx, AnimalDefinitionTables,
    AnimalDefinitions, AnimalEnv, AnimalSandbox, AnimalStat, AnimalWorld, ObjectRef,
};
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};
use pz_world::save::animal::CharacterStatDecl;
use pz_world::square::IsoDirections;

use super::fire::CtxRand;
use super::state::{ActionGroups, StateCx, StateRegistry};
use super::stats::{CharacterStatRef, CharacterStats};
use super::{CharacterKind, GameCharacter, SimContext};
use crate::worn_items::ItemIdentity;
use crate::Characters;

/// What the character side reads of the animal's `AnimalDefinitions`
/// (`adef`), copied when the animal is set up (the definitions do not
/// change after loading).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalDefValues {
    pub animset: Option<String>,
    pub turn_delta: f32,
    pub animal_size: f32,
    pub collision_size: f32,
    pub collidable: bool,
    pub can_thump: bool,
    pub can_climb_stairs: bool,
    pub can_climb_fences: bool,
    pub corpse_size: f32,
    pub corpse_length: f32,
    pub base_dmg: f32,
    pub attack_back: bool,
    pub wild_flee_time_until_dead_timer: f32,
    /// The whole definition, for the behaviour and the states.
    pub adef: Rc<AnimalDefinitions>,
}

impl AnimalDefValues {
    pub fn of(d: &AnimalDefinitions) -> Self {
        AnimalDefValues {
            adef: Rc::new(d.clone()),
            animset: d.animset.clone(),
            turn_delta: d.turn_delta,
            animal_size: d.animal_size,
            collision_size: d.collision_size,
            collidable: d.collidable,
            can_thump: d.can_thump,
            can_climb_stairs: d.can_climb_stairs,
            can_climb_fences: d.can_climb_fences,
            corpse_size: d.corpse_size,
            corpse_length: d.corpse_length,
            base_dmg: d.base_dmg,
            attack_back: d.attack_back,
            wild_flee_time_until_dead_timer: d.wild_flee_time_until_dead_timer,
        }
    }
}

/// One character `updateLOS` passed to `behavior.spotted(chr, false, dist)`
/// (the zombies and visible non-animal players of the cell), in the order
/// of the calls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spotted {
    pub chr: i32,
    pub dist: f32,
}

/// `IsoAnimal`'s fields: the live animal and the character-side state.
#[derive(Clone, Debug)]
pub struct AnimalFields {
    /// The animal logic's state (`pz-animals`).
    pub animal: Animal,
    /// `adef` (`None` for a type without a definition: `init` returned
    /// early and the animal has no `data` or `behavior`).
    pub def: Option<AnimalDefValues>,
    /// `behavior != null && data != null` (`init` ran to the end).
    pub initialized: bool,
    /// `CharacterStat.HUNGER` / `THIRST`.
    pub hunger: CharacterStatRef,
    pub thirst: CharacterStatRef,
    /// `CharacterStat.ORDERED_STATS` as `pz-animals` reads them.
    pub stat_defs: Rc<[CharacterStatDecl]>,
    /// What constructing the character drew.
    pub construction: CharacterConstruction,
    /// `IsoMovingObject.width` (0.24 until the update sets
    /// `adef.collisionSize * getAnimalSize()`).
    pub width: f32,
    /// `IsoPlayer.onlineID` (`AnimalInstanceManager`'s id).
    pub online_id: i16,
    /// `vehicle4testCollision`: the vehicle (by id) a collision is tested
    /// against this update.
    pub vehicle_for_collision_test: Option<i32>,
    /// The `behavior.spotted` calls of the last `updateLOS`.
    pub spotted: Vec<Spotted>,
    /// `spottedChr` (a character id).
    pub spotted_chr: Option<i32>,
    /// `alertedChr`.
    pub alerted_chr: Option<i32>,
    /// `atkTarget`.
    pub atk_target: Option<i32>,
    /// `thumpTarget` (an `IsoThumpable` or `IsoDoor`).
    pub thump_target: Option<pz_pathfind::mover::ObjectRef>,
    /// `fightingOpponent`.
    pub fighting_opponent: Option<i32>,
    /// `luredBy` (a player).
    pub lured_by: Option<i32>,
    pub walk_to_char_luring: bool,
    pub lured_start_timer: f32,
    /// `isAttackingOnClient`.
    pub attacking_on_client: bool,
    /// `lastSoundRespondedTo` (the copy the animal keeps).
    pub last_sound_responded_to: Option<pz_audio::world_sound::WorldSound>,
    pub time_since_flee_from_sound: f32,
    /// `isExistInTheWorld()`: on a square whose moving objects list the
    /// animal. The world sets it when it adds the animal to a square and
    /// clears it when it takes it off.
    pub exists_in_world: bool,
    /// `stopAllMovementNow()` asked for by the animal logic
    /// (`pz-animals`, which has no state machine): done after its call
    /// returns ([`GameCharacter::animal_env`]).
    pub pending_stop_all_movement: bool,
}

impl AnimalFields {
    /// The fields of `new IsoAnimal(cell)` before `init`.
    pub fn new(stats: &CharacterStats) -> ScriptResult<Self> {
        Ok(AnimalFields {
            animal: Animal::new(None),
            def: None,
            initialized: false,
            hunger: stats.field("HUNGER")?,
            thirst: stats.field("THIRST")?,
            stat_defs: ordered_stat_decls(stats)?,
            construction: CharacterConstruction::default(),
            width: 0.24,
            online_id: -1,
            vehicle_for_collision_test: None,
            spotted: Vec::new(),
            spotted_chr: None,
            alerted_chr: None,
            atk_target: None,
            thump_target: None,
            fighting_opponent: None,
            lured_by: None,
            walk_to_char_luring: false,
            lured_start_timer: -1.0,
            attacking_on_client: false,
            last_sound_responded_to: None,
            time_since_flee_from_sound: 0.0,
            exists_in_world: false,
            pending_stop_all_movement: false,
        })
    }

    fn def_values(&self) -> &AnimalDefValues {
        self.def
            .as_ref()
            .unwrap_or_else(|| panic!("animal {:?} has no definition", self.animal.animal_type))
    }
}

/// `CharacterStat.ORDERED_STATS` as `pz-world`'s declarations.
pub fn ordered_stat_decls(stats: &CharacterStats) -> ScriptResult<Rc<[CharacterStatDecl]>> {
    stats
        .ordered
        .iter()
        .map(|s| {
            let field = stats
                .constants()
                .iter()
                .find(|(_, c)| Rc::ptr_eq(c, s))
                .map(|(f, _)| f.clone())
                .ok_or_else(|| {
                    ScriptError::new(format!("CharacterStat {} has no field", s.id()))
                })?;
            Ok(CharacterStatDecl {
                field,
                id: s.id().to_string(),
                minimum: s.minimum_value(),
                maximum: s.maximum_value(),
                default: s.default_value(),
            })
        })
        .collect()
}

/// What the world gives the animal code: the definition tables and
/// `pz-animals`' world bound to this animal.
pub struct AnimalHost<'a> {
    pub defs: &'a AnimalDefinitionTables,
    pub world: &'a mut dyn AnimalWorld,
}

/// `GameTime` as the animal code reads it, from the [`SimContext`].
pub struct CtxClock<'a>(pub &'a dyn SimContext);

impl AnimalClock for CtxClock<'_> {
    /// `getHour()`: `floor(floor(timeOfDay * 3600) / 3600)`.
    fn hour(&self) -> i32 {
        let s = ((self.0.time_of_day() * 3600.0) as f64).floor();
        (s / 3600.0).floor() as i32
    }
    fn month(&self) -> i32 {
        self.0.month()
    }
    fn calendar_millis(&self) -> i64 {
        self.0.calendar_time_in_millis()
    }
    fn multiplier(&self) -> f32 {
        self.0.multiplier()
    }
    /// `getWorldAgeHours() / 24 + (getTimeSinceApo() - 1) * 30`.
    fn world_age_days_since_begin(&self) -> f64 {
        self.0.world_age_hours() / 24.0 + ((self.0.time_since_apo() - 1) * 30) as f64
    }
}

/// A `pz-animals` error as the character code's.
pub(crate) fn jerr(e: pz_animals::JavaException) -> ScriptError {
    ScriptError::new(format!("java.lang.{}", e.0))
}

impl<I: 'static> GameCharacter<I> {
    /// `new IsoAnimal(cell)`'s character: the `IsoPlayer` construction for
    /// an animal (`new SurvivorDesc()` with its six `doStats` draws, then
    /// the `IsoGameCharacter` patience draw), at `(0.5, 0.5, 0)` facing
    /// west, collidable, `AnimalIdleState`
    /// as the default state (set by [`GameCharacter::initialize_animal_states`]),
    /// and an animal that is not initialised yet. Never one of
    /// `IsoPlayer.players` (`in_players` stays false).
    pub fn new_animal(sm: &ScriptManager, chars: &mut Characters) -> ScriptResult<Self>
    where
        I: ItemIdentity,
    {
        let mut c = Self::new(sm, CharacterKind::Animal);
        let desc = crate::survivor::SurvivorDesc::<I>::new(chars, sm)?;
        let ds = [
            desc.bravery,
            desc.aggressiveness,
            desc.loner,
            desc.temper,
            desc.favourindoors,
            desc.loyalty,
        ];
        c.descriptor = Some(Rc::new(std::cell::RefCell::new(desc)));
        c.patience = pz_core::random::next_range(Self::PATIENCE_MIN, Self::PATIENCE_MAX);
        c.path.collidable = true;
        // IsoPlayer(cell, desc, 0, 0, 0): the square's centre, facing west.
        c.vars.x = 0.5;
        c.vars.y = 0.5;
        c.vars.last_x = 0.5;
        c.vars.last_y = 0.5;
        c.vars.next_x = 0.5;
        c.vars.next_y = 0.5;
        c.vars.dir = IsoDirections::W;
        let mut f = AnimalFields::new(&chars.sim.character_stats)?;
        f.construction = CharacterConstruction {
            descriptor_stats: ds,
            patience: c.patience,
        };
        c.animal = Some(Box::new(f));
        Ok(c)
    }

    /// The animal's fields (`None` for other characters).
    pub fn animal_fields(&self) -> Option<&AnimalFields> {
        self.animal.as_deref()
    }

    /// The animal's fields, mutably.
    pub fn animal_fields_mut(&mut self) -> Option<&mut AnimalFields> {
        self.animal.as_deref_mut()
    }

    /// Runs `f` with the live [`Animal`] beside the character as its
    /// [`AnimalCharacter`] (the animal is moved out of its fields for the
    /// call). `None` for a character that is not an animal.
    pub fn with_animal<R>(&mut self, f: impl FnOnce(&mut Animal, &mut Self) -> R) -> Option<R> {
        let fields = self.animal.as_mut()?;
        let mut a = std::mem::replace(&mut fields.animal, Animal::new(None));
        let r = f(&mut a, self);
        if let Some(fields) = self.animal.as_mut() {
            fields.animal = a;
        }
        Some(r)
    }

    /// [`GameCharacter::with_animal`] with the animal environment (the
    /// global `Rand`, the clock and sandbox of `ctx`, the host's definitions
    /// and world).
    pub fn with_animal_env<R>(
        &mut self,
        ctx: &dyn SimContext,
        host: &mut AnimalHost<'_>,
        f: impl FnOnce(&mut Animal, &mut Self, &mut AnimalEnv<'_>) -> R,
    ) -> Option<R> {
        let stat_defs = self.animal.as_ref()?.stat_defs.clone();
        let clock = CtxClock(ctx);
        let sandbox: AnimalSandbox = ctx.animal_sandbox();
        let mut rand = CtxRand(ctx);
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: host.defs,
                clock: &clock,
                sandbox: &sandbox,
            },
            rand: &mut rand,
            world: &mut *host.world,
            stat_defs,
            patience_range: (Self::PATIENCE_MIN, Self::PATIENCE_MAX),
        };
        self.with_animal(|a, c| f(a, c, &mut env))
    }

    /// `init(breed)` for a constructed animal of `animal_type`: the
    /// character parts (`adef`, the animation set's action group,
    /// `initializeStates`, `setTurnDelta(adef.turnDelta)`) and
    /// `pz-animals`' `init` with its `Rand` draws. `Ok(false)` for a type
    /// without a definition (the original returns early).
    pub fn init_animal(
        &mut self,
        animal_type: &str,
        breed: Option<&AnimalBreed>,
        registry: &mut StateRegistry<I>,
        groups: &mut ActionGroups,
        ctx: &dyn SimContext,
        host: &mut AnimalHost<'_>,
    ) -> ScriptResult<bool> {
        let Some(fields) = self.animal.as_mut() else {
            return Err(crate::npe());
        };
        fields.animal.animal_type = Some(animal_type.to_string());
        let Some(d) = host.defs.def(animal_type) else {
            fields.def = None;
            return Ok(false);
        };
        fields.def = Some(AnimalDefValues::of(d));
        self.animal_character_init(registry, groups);
        let ok = self
            .with_animal_env(ctx, host, |a, c, env| a.init(breed, c, env))
            .ok_or_else(crate::npe)?
            .map_err(jerr)?;
        if let Some(f) = self.animal.as_mut() {
            f.initialized = ok;
        }
        Ok(ok)
    }

    /// The character parts of `init`: the action group of `adef.animset`
    /// (`initType`), `initializeStates()` and the turn delta.
    pub(crate) fn animal_character_init(
        &mut self,
        registry: &mut StateRegistry<I>,
        groups: &mut ActionGroups,
    ) {
        let Some(def) = self.animal.as_ref().and_then(|f| f.def.clone()) else {
            return;
        };
        if let Some(set) = def.animset.as_deref() {
            let g = groups.get(set);
            self.set_action_group(g);
        }
        self.initialize_animal_states(registry);
        self.vars.turn_delta_normal = def.turn_delta;
    }

    /// `IsoAnimal.initializeStates()`: the twelve animal states by their
    /// names ([`crate::character::states::animal`]), `AnimalIdleState` as
    /// the default state.
    pub fn initialize_animal_states(&mut self, registry: &mut StateRegistry<I>) {
        use crate::character::states::ids;
        self.clear_ai_state_map();
        let states = [
            ("idle", ids::ANIMAL_IDLE),
            ("eating", ids::ANIMAL_EAT),
            ("attack", ids::ANIMAL_ATTACK),
            ("walk", ids::ANIMAL_WALK),
            ("pathfind", ids::ANIMAL_PATH_FIND),
            ("followwall", ids::ANIMAL_FOLLOW_WALL),
            ("hitreaction", ids::ANIMAL_HIT_REACTION),
            ("falldown", ids::ANIMAL_FALLDOWN),
            ("onground", ids::ANIMAL_ON_GROUND),
            ("zone", ids::ANIMAL_ZONE),
            ("alerted", ids::ANIMAL_ALERTED),
            ("climbfence", ids::ANIMAL_CLIMB_OVER_FENCE),
        ];
        for (name, id) in states {
            let s = registry.get_or_opaque(id.0);
            self.register_ai_state(name, s);
        }
        let idle = registry.get_or_opaque(ids::ANIMAL_IDLE.0);
        self.set_default_state_to(Some(idle));
    }

    // -------------------------------------------------------------------
    // the IsoAnimal overrides

    /// `IsoAnimal.getObjectName()`.
    pub fn animal_object_name(&self) -> &'static str {
        "Animal"
    }

    /// `IsoAnimal.GetAnimSetName()`: `adef.animset`, `"cow"` without a
    /// definition.
    pub fn animal_anim_set_name(&self) -> Option<&str> {
        match self.animal.as_ref().and_then(|f| f.def.as_ref()) {
            None => Some("cow"),
            Some(d) => d.animset.as_deref(),
        }
    }

    /// `IsoAnimal.isLocalPlayer()`: every animal is local outside a client
    /// (not through `IsoPlayer.players`).
    pub fn animal_is_local(&self, ctx: &dyn SimContext) -> bool {
        !ctx.is_client()
    }

    /// `IsoAnimal.CanUsePathfindState()`: `isLocalPlayer()`.
    pub fn animal_can_use_pathfind_state(&self, ctx: &dyn SimContext) -> bool {
        self.animal_is_local(ctx)
    }

    /// `IsoAnimal.shouldBecomeZombieAfterDeath()`: never.
    pub fn animal_should_become_zombie_after_death(&self) -> bool {
        false
    }

    /// `IsoAnimal.canRagdoll()`: never.
    pub fn animal_can_ragdoll(&self) -> bool {
        false
    }

    /// `IsoAnimal.allowsTwist()`: never (the head is the only twist bone).
    pub fn animal_allows_twist(&self) -> bool {
        false
    }

    /// `IsoAnimal.onAnimPlayerCreated(player)`'s own part: the head as the
    /// only twist bone, `Bip01` the counter-rotation bone. (The
    /// `IsoPlayer` part it calls first, the bone reparenting, is not for
    /// animals.)
    pub fn animal_on_anim_player_created(player: &mut pz_anim::AnimationPlayer) {
        player.set_twist_bones(&["Bip01_Head"]);
        player.set_counter_rotation_bone("Bip01");
    }

    /// `IsoAnimal.canUseCurrentPoseForCorpse()`: not on a hook, and the
    /// character's own answer (`pose`).
    pub fn animal_can_use_current_pose_for_corpse(&self, pose: bool) -> bool {
        !self.animal.as_ref().is_some_and(|f| f.animal.on_hook) && pose
    }

    /// `IsoAnimal.getCorpseSize()` / `getCorpseLength()`.
    pub fn animal_corpse_size(&self) -> Option<(f32, f32)> {
        let d = self.animal.as_ref()?.def.as_ref()?;
        Some((d.corpse_size, d.corpse_length))
    }

    /// `IsoAnimal.calcDamage()`: `adef.baseDmg`, times the `strength`
    /// gene's value.
    pub fn animal_calc_damage(&self) -> Option<f32> {
        let f = self.animal.as_ref()?;
        let mut d = f.def.as_ref()?.base_dmg;
        if let Some(g) = f.animal.used_gene("strength") {
            d *= g.current_value;
        }
        Some(d)
    }

    /// `IsoAnimal.setHealth(h)`: an invincible animal does not lose
    /// health. (The server's extra update is network.)
    pub fn animal_set_health(&mut self, health: f32) {
        if self.animal.as_ref().is_some_and(|f| f.animal.invincible) && health < self.health {
            return;
        }
        self.health = health;
    }

    /// `IsoAnimal.hitConsequences(weapon, wielder, ignoreDamage, damage,
    /// false)`, which replaces the character's: health lost through
    /// `data.getHealthLoss(0.025)`, a wild animal's drop-dead timer, blood,
    /// the attacker and attack time, the `hitreact` reaction, `killed(player)`
    /// (the zone's other animals are stressed and flee), stress and the
    /// server's `goAttack`.
    pub fn animal_hit_consequences(
        &mut self,
        wielder: &GameCharacter<I>,
        wielder_aiming_level: i32,
        ignore_damage: bool,
        damage: f32,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        let ctx = cx.ctx;
        if !ctx.is_client() && !ignore_damage {
            if let Some(host) = cx.world.animal_host(self.id) {
                let clock = CtxClock(ctx);
                let sandbox = ctx.animal_sandbox();
                let actx = AnimalCtx {
                    defs: host.defs,
                    clock: &clock,
                    sandbox: &sandbox,
                };
                let loss = self
                    .animal
                    .as_ref()
                    .ok_or_else(crate::npe)?
                    .animal
                    .data_health_loss(Some(0.025), actx);
                self.animal_set_health(self.health - damage * loss);
            }
        }
        let (wild, t) = {
            let f = self.animal.as_ref().ok_or_else(crate::npe)?;
            (
                f.animal.wild,
                f.def
                    .as_ref()
                    .map_or(0.0, |d| d.wild_flee_time_until_dead_timer),
            )
        };
        if wild && !ignore_damage {
            let mut v = ctx.rand_next_float(t / 2.0, t);
            v /= 1.0 + wielder_aiming_level as f32 / 10.0;
            if let Some(f) = self.animal.as_mut() {
                f.animal.behavior.wild_and_hurt = true;
                f.animal.behavior.wild_drop_dead_timer = v;
            }
        }
        if !ignore_damage {
            GameCharacter::splat_blood_floor_big(&*self, ctx);
        }
        self.vars.attacked_by = Some(wielder.id);
        let now = ctx.calendar_time_in_millis();
        if let Some(f) = self.animal.as_mut() {
            f.animal.attacked_timer = now;
        }
        if !ctx.is_client() {
            self.vars.hit_reaction = "hitreact".to_string();
            // instanceof IsoPlayer
            if (wielder.is_player() || wielder.is_animal()) && self.health <= 0.0 {
                self.animal_killed(Some(wielder.id), (wielder.vars.x, wielder.vars.y), cx);
            }
        }
        let r = ctx.rand_next_range(20, 40);
        if let Some(f) = self.animal.as_mut() {
            let s = f.animal.stress_level + r as f32;
            f.animal.set_debug_stress(s);
        }
        let attack_back = self
            .animal
            .as_ref()
            .and_then(|f| f.def.as_ref())
            .is_some_and(|d| d.attack_back);
        if ctx.is_server()
            && (wielder.is_player() || wielder.is_animal() || wielder.is_zombie())
            && attack_back
        {
            if let Some(f) = self.animal.as_mut() {
                f.atk_target = Some(wielder.id);
            }
            self.behavior_go_attack(wielder.id, cx);
        }
        Ok(())
    }

    /// `IsoAnimal.getAnimalSize()` (`data.size`).
    pub fn animal_size(&self) -> f32 {
        self.animal.as_ref().map_or(0.0, |f| f.animal.data.size)
    }

    /// `IsoMovingObject.isPushedByForSeparate(other)`'s animal rules: an
    /// animal that is not collidable, or whose behaviour blocks movement,
    /// neither is pushed nor pushes. `None` when neither is an animal (the
    /// general rule applies).
    pub fn animal_pushed_by_for_separate(&self, other: &GameCharacter<I>) -> Option<bool> {
        let blocks = |c: &GameCharacter<I>| {
            c.animal.as_ref().map(|f| {
                !f.def.as_ref().is_some_and(|d| d.collidable) || f.animal.behavior.block_movement
            })
        };
        match (blocks(self), blocks(other)) {
            (Some(true), _) | (_, Some(true)) => Some(false),
            (None, None) => None,
            _ => Some(true),
        }
    }

    /// `IsoMovingObject.doStairs()`'s animal check. The original tests
    /// `isAnimal() && !canClimbStairs()` with an empty body, so animals take
    /// stairs like other characters; this records that for callers.
    pub fn animal_stairs_rule_applies(&self) -> bool {
        true
    }

    /// `IsoAnimal.addToWorld()`: the `IsoPlayer` part is the world's; an
    /// animal on a hook gets the `onhook` variable.
    pub fn animal_add_to_world(&mut self, host: Option<&mut AnimalHost<'_>>) {
        if let Some(h) = host {
            h.world.added_to_world();
        }
        if self.animal.as_ref().is_some_and(|f| f.animal.on_hook) {
            self.set_var_bool("onhook", true);
        }
    }

    /// `IsoAnimal.removeFromWorld()`: off the leash, out of the cell, its
    /// designation zone and (keeping the hook's square to reattach) its
    /// hook; `AnimalManagerMain.removeFromWorld`; the behaviour targets
    /// cleared; then `AnimalPopulationManager.addToRecentlyRemoved`.
    /// (`IsoPlayer.removeFromWorld`'s `RecentlyRemoved` is not for animals.)
    pub fn animal_remove_from_world(&mut self, host: &mut AnimalHost<'_>) {
        let Some(f) = self.animal.as_mut() else {
            return;
        };
        let a = &mut f.animal;
        if a.data.attached_player.is_some() {
            host.world.detach_from_player();
        }
        host.world.removed_from_world();
        if let Some(z) = a.d_zone {
            host.world.move_animal_to_zone(Some(z), None);
        }
        if a.on_hook {
            if let Some((x, y, z)) = a.hook {
                a.attach_back_to_hook_x = x;
                a.attach_back_to_hook_y = y;
                a.attach_back_to_hook_z = z;
            }
        }
        a.alerted = false;
        a.drink_from_puddle = None;
        a.drink_from_river = None;
        a.drink_from_trough = None;
        a.eat_from_trough = None;
        a.eat_from_ground = None;
        a.following_wall = false;
        a.held_by = None;
        a.moving_to_food = None;
        a.should_follow_wall = false;
        f.spotted.clear();
        self.vars.attacked_by = None;
        host.world.add_to_recently_removed();
    }
}

/// `IsoAnimal` as the pathfinder's `AnimalMover`.
impl<I: 'static> pz_pathfind::mover::AnimalMover for GameCharacter<I> {
    /// `adef.canThump` and hunger or thirst above 0.8.
    fn should_break_obstacles_during_pathfinding(&self) -> bool {
        let Some(f) = self.animal.as_ref() else {
            return false;
        };
        if !f.def_values().can_thump {
            return false;
        }
        self.stats.get(&f.hunger) > 0.8 || self.stats.get(&f.thirst) > 0.8
    }
    fn can_climb_fences(&self) -> bool {
        self.animal
            .as_ref()
            .is_some_and(|f| f.def_values().can_climb_fences)
    }
    fn animal_size(&self) -> f32 {
        self.animal
            .as_ref()
            .map_or(0.0, |f| f.def_values().animal_size)
    }
}

impl<I: 'static> AnimalCharacter for GameCharacter<I> {
    fn health(&self) -> f32 {
        self.health
    }
    fn set_health_field(&mut self, health: f32) {
        self.health = health;
    }
    /// `IsoPlayer.hoursSurvived` (an animal is an `IsoPlayer`).
    fn hours_survived(&self) -> f64 {
        self.hours_survived
    }
    fn set_hours_survived(&mut self, hours: f64) {
        self.hours_survived = hours;
    }
    fn is_female(&self) -> bool {
        self.descriptor
            .as_ref()
            .is_some_and(|d| d.borrow().is_female())
    }
    fn set_female(&mut self, female: bool) {
        if let Some(d) = &self.descriptor {
            d.borrow_mut().set_female(female);
        }
    }
    fn stat(&self, stat: AnimalStat) -> f32 {
        self.stats.get(&self.animal_stat_ref(stat))
    }
    fn set_stat(&mut self, stat: AnimalStat, value: f32) {
        let s = self.animal_stat_ref(stat);
        self.stats.set(&s, value);
    }
    fn stat_default(&self, stat: AnimalStat) -> f32 {
        self.animal_stat_ref(stat).default_value()
    }
    fn variable(&self, name: &str) -> String {
        self.variable_string(name)
    }
    fn set_variable(&mut self, name: &str, value: &str) {
        self.set_var_str(name, value);
    }
    fn clear_variable(&mut self, name: &str) {
        self.clear_var(name);
    }
    fn state_event_delay_timer(&self) -> f32 {
        self.vars.state_event_delay_timer
    }
    fn set_state_event_delay_timer(&mut self, t: f32) {
        self.vars.state_event_delay_timer = t;
    }
    fn position(&self) -> (f32, f32, f32) {
        (self.vars.x, self.vars.y, self.vars.z)
    }
    fn dir(&self) -> i32 {
        self.vars.dir as i32
    }
    /// `setDir(dir)`: `setForwardIsoDirection`, which also sets the
    /// forward vector.
    fn set_dir(&mut self, dir: i32) {
        self.set_forward_iso_direction(IsoDirections::ALL[(dir & 7) as usize]);
    }
    fn vehicle(&self) -> Option<ObjectRef> {
        self.path.vehicle.map(|v| v as u16 as ObjectRef)
    }
    fn online_id(&self) -> i16 {
        self.animal.as_ref().map_or(-1, |f| f.online_id)
    }
    /// `splatBloodFloorBig()`: the blood type is drawn on a square with a
    /// chunk. (The splat itself reaches the world through
    /// [`GameCharacter::splat_blood_floor_big`] where a `SimContext` is at
    /// hand; `pz-animals` only needs the draw kept in order.)
    fn splat_blood_floor_big(&mut self, rand: &mut dyn pz_world::GameRand) {
        if self.environment.square_has_chunk {
            rand.next(20);
        }
    }
    fn set_hit_reaction(&mut self, reaction: &str) {
        self.vars.hit_reaction = reaction.to_string();
    }
    fn clear_attacked_by(&mut self) {
        self.vars.attacked_by = None;
    }
    /// The current square as a [`pz_animals::square_ref`].
    fn current_square(&self) -> Option<ObjectRef> {
        self.path
            .current_square
            .map(|(x, y, z)| pz_animals::square_ref(x, y, z))
    }
    fn exists_in_world(&self) -> bool {
        self.animal.as_ref().is_some_and(|f| f.exists_in_world)
    }
    /// `stopAllMovementNow()`: the parts without the world (not moving, no
    /// path); the path reset and the change to the default state follow
    /// when the `pz-animals` call returns
    /// ([`AnimalFields::pending_stop_all_movement`]).
    fn stop_all_movement_now(&mut self) {
        self.vars.moving = false;
        self.path.path2 = None;
        if let Some(f) = self.animal.as_mut() {
            f.pending_stop_all_movement = true;
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    fn animal_stat_ref(&self, stat: AnimalStat) -> CharacterStatRef {
        let f = self
            .animal
            .as_ref()
            .expect("an animal's stats without its fields");
        match stat {
            AnimalStat::Hunger => f.hunger.clone(),
            AnimalStat::Thirst => f.thirst.clone(),
        }
    }

    /// The character values `pz-animals` keeps for an animal outside the
    /// world (stats in `ORDERED_STATS` order), read from this character.
    pub fn animal_character_values(&self, chars: &Characters) -> Option<CharacterValues> {
        let f = self.animal.as_ref()?;
        let mut v = CharacterValues::new(f.stat_defs.clone());
        v.stats = chars
            .sim
            .character_stats
            .ordered
            .iter()
            .map(|s| self.stats.get(s))
            .collect();
        v.health = self.health;
        v.hours_survived = self.hours_survived;
        v.female = AnimalCharacter::is_female(self);
        v.state_event_delay_timer = self.vars.state_event_delay_timer;
        v.x = self.vars.x;
        v.y = self.vars.y;
        v.z = self.vars.z;
        v.dir = self.vars.dir as i32;
        v.online_id = f.online_id;
        v.construction = f.construction;
        Some(v)
    }

    /// Takes over the character values of an animal made outside the world
    /// (`pz-animals`' `construct`, `load`, babies): stats, health, hours
    /// survived, sex, position, direction, the variables and the state
    /// event timer.
    pub fn apply_animal_character_values(&mut self, chars: &Characters, v: &CharacterValues) {
        for (s, val) in chars.sim.character_stats.ordered.iter().zip(&v.stats) {
            self.stats.set(s, *val);
        }
        self.health = v.health;
        self.hours_survived = v.hours_survived;
        AnimalCharacter::set_female(self, v.female);
        self.vars.state_event_delay_timer = v.state_event_delay_timer;
        self.vars.x = v.x;
        self.vars.y = v.y;
        self.vars.z = v.z;
        self.vars.last_x = v.x;
        self.vars.last_y = v.y;
        self.vars.last_z = v.z;
        self.vars.next_x = v.x;
        self.vars.next_y = v.y;
        self.vars.dir = IsoDirections::ALL[(v.dir & 7) as usize];
        for (k, val) in &v.variables {
            self.set_var_str(k, val);
        }
        if let Some(f) = self.animal.as_mut() {
            f.online_id = v.online_id;
        }
    }
}
