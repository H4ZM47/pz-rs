//! The non-rendering simulation state of `zombie.characters.IsoGameCharacter`
//! (and the `IsoPlayer` parts of it): stats, moodles, traits, XP and perk
//! levels, the descriptor, and the timers and flags the stat update reads.
//!
//! # Layout
//!
//! * [`stats`]: `CharacterStat`, `Stats`.
//! * [`moodles`]: `Moodle`, `MoodleStat`, `Moodles`.
//! * [`xp`]: `IsoGameCharacter.XP`, `PerkInfo`, `XPMultiplier` and the
//!   character's perk-level methods (`LevelPerk`, `applyTraits`, ...).
//! * [`update`]: the per-tick stat update (`calculateStats` and the
//!   `IsoPlayer` overrides) and the moodle update.
//! * [`context`]: [`SimContext`] (the singletons: `GameTime`,
//!   `SandboxOptions`, `ZomboidGlobals`, network flags, Lua events) and
//!   [`CharacterEnvironment`] (per-character inputs from unported world
//!   systems).
//! * [`items`]: the inventory, hand and worn items (`pz-inventory` items)
//!   and the `IsoGameCharacter` methods reading them; [`clothing_wetness`]
//!   (`ClothingWetness`).
//! * The body systems, each owned by [`GameCharacter`]: [`body_damage`]
//!   (`BodyDamage`), [`body_part`] (`BodyPart`, `BodyPartType`,
//!   `BodyPartLast`, `BodyPartContacts`), [`nutrition`],
//!   [`thermoregulator`] (inside `BodyDamage`, as in the original) and
//!   [`fitness`], with [`metabolics`] (the jar's `Metabolics` table).
//! * [`save`]: `IsoPlayer.load`/`save` (the `players.db` record) with the
//!   `IsoGameCharacter`/`IsoMovingObject` parts.
//!
//! The original's statics the simulation needs (`CharacterStat.REGISTRY`,
//! `MoodleStat`'s thresholds, `LevelUpLevels`, `CheatType`) are in
//! [`SimStatics`], read from the installed jar; the `PerkFactory` statics
//! are [`crate::Characters::perks`].
//!
//! Code names game constants by their Java field (`"FITNESS"`,
//! `"FAST_LEARNER"`, `"Fitness"`); the ids behind them come from the jar.

pub mod body_damage;
pub mod body_part;
pub mod clothing_wetness;
pub mod collide;
pub mod collision;
pub mod combat;
pub mod context;
pub mod fire;
pub mod firearm;
pub mod fitness;
pub mod item_owner;
pub mod items;
pub mod metabolics;
pub mod moodles;
pub mod nutrition;
pub mod path_mover;
pub mod player;
pub mod save;
pub mod state;
pub mod states;
pub mod stats;
pub mod thermoregulator;
pub mod update;
pub mod xp;
pub mod zombie;

use std::rc::Rc;

use pz_scripts::objects::items::CharacterTrait;
use pz_scripts::registry::builtin::BuiltinTables;
use pz_scripts::{ScriptManager, ScriptResult};

pub use body_damage::{BodyDamage, WeaponUser, ZombieAttack, ZombieAttackOutcome};
pub use clothing_wetness::ClothingWetness;
pub use item_owner::CharacterItemContext;
pub use items::CharacterItems;
pub use body_part::{BodyPart, BodyPartContacts, BodyPartLast, BodyPartTypes, InjuryEnv};
pub use context::{
    CharacterEnvironment, DefaultSimContext, HaloText, SimContext, SimEvent, ZomboidGlobals,
};
pub use fitness::{Fitness, FitnessExercise};
pub use metabolics::Metabolics;
pub use moodles::{ChevronColor, Moodle, MoodleLevel, MoodleStat, MoodleStatRef, MoodleStats, Moodles};
pub use nutrition::Nutrition;
pub use stats::{CharacterStat, CharacterStatRef, CharacterStats, Stats};
pub use thermoregulator::{ThermalNode, Thermoregulator};
pub use xp::{PerkInfo, XPMultiplier, XP};

use crate::survivor::SurvivorDescRef;
use crate::traits::CharacterTraits;
use crate::Characters;

/// The simulation statics read from the jar.
#[derive(Clone, Debug, Default)]
pub struct SimStatics {
    /// `CharacterStat`'s statics.
    pub character_stats: CharacterStats,
    /// `MoodleStat`'s statics.
    pub moodle_stats: MoodleStats,
    /// `IsoGameCharacter.LevelUpLevels`.
    pub level_up_levels: Vec<i32>,
    /// `CheatType.values()` names.
    pub cheat_types: Vec<String>,
    /// `BodyPartType`'s constants.
    pub body_part_types: BodyPartTypes,
    /// `IsoZombie.temporaryMapCloseSneakBonusDir`/`Value`: (sprite,
    /// directions, value).
    pub zombie_close_sneak_bonus: Vec<(String, String, f32)>,
}

impl SimStatics {
    pub fn from_tables(tables: &BuiltinTables) -> ScriptResult<Self> {
        Ok(SimStatics {
            character_stats: CharacterStats::from_tables(tables)?,
            moodle_stats: MoodleStats::from_tables(tables),
            level_up_levels: tables.level_up_levels.clone(),
            cheat_types: tables.cheat_types.clone(),
            body_part_types: BodyPartTypes::from_tables(tables),
            zombie_close_sneak_bonus: tables.zombie_close_sneak_bonus.clone(),
        })
    }
}

/// `PlayerCheats`: the set `CheatType` constants (by name).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerCheats {
    cheats: Vec<String>,
}

impl PlayerCheats {
    /// `isCheatAllowed`: debug, client or server.
    fn is_cheat_allowed(ctx: &dyn SimContext) -> bool {
        ctx.is_debug() || ctx.is_client() || ctx.is_server()
    }

    /// `isSet`.
    pub fn is_set(&self, ctx: &dyn SimContext, cheat: &str) -> bool {
        Self::is_cheat_allowed(ctx) && self.cheats.iter().any(|c| c == cheat)
    }

    /// `set(CheatType, boolean)`: ignored unless cheats are allowed.
    pub fn set(&mut self, ctx: &dyn SimContext, cheat: &str, on: bool) {
        if !Self::is_cheat_allowed(ctx) {
            return;
        }
        let has = self.cheats.iter().position(|c| c == cheat);
        match (on, has) {
            (true, None) => self.cheats.push(cheat.to_string()),
            (false, Some(i)) => {
                self.cheats.remove(i);
            }
            _ => {}
        }
    }

    /// `clear`.
    pub fn clear(&mut self) {
        self.cheats.clear();
    }

    /// The set cheats (by `CheatType` name), whether allowed or not.
    pub fn names(&self) -> &[String] {
        &self.cheats
    }

    /// `load`: adds each saved `CheatType` (by ordinal into `cheat_types`,
    /// `CheatType.values()`), bypassing `isCheatAllowed`; nothing is
    /// cleared first.
    pub fn load(&mut self, cheat_types: &[String], r: &mut pz_inventory::io::Reader<'_>) -> ScriptResult {
        let n = r.i32().map_err(save::io_error)?;
        for _ in 0..n {
            let id = r.i8().map_err(save::io_error)?;
            let name = usize::try_from(id).ok().and_then(|i| cheat_types.get(i)).ok_or_else(|| {
                pz_scripts::ScriptError::new(format!(
                    "java.lang.ArrayIndexOutOfBoundsException: Index {id} out of bounds for length {}",
                    cheat_types.len()
                ))
            })?;
            if !self.cheats.contains(name) {
                self.cheats.push(name.clone());
            }
        }
        Ok(())
    }

    /// `save`: the set cheats' ordinals in `EnumSet` (ordinal) order.
    pub fn save(&self, cheat_types: &[String], w: &mut pz_inventory::io::Writer) {
        let mut ids: Vec<usize> = self.cheats.iter().filter_map(|c| cheat_types.iter().position(|t| t == c)).collect();
        ids.sort_unstable();
        w.i32(ids.len() as i32);
        for i in ids {
            w.i8(i as i8);
        }
    }
}

/// What kind of `IsoGameCharacter` subclass this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CharacterKind {
    /// `IsoPlayer`.
    #[default]
    Player,
    /// `IsoSurvivor` and other non-player humans.
    Survivor,
    /// `IsoZombie`.
    Zombie,
    /// `IsoAnimal`.
    Animal,
}

/// `IsoGameCharacter`'s simulation state.
pub struct GameCharacter<I: 'static> {
    /// `IsoMovingObject.getID()`.
    pub id: i32,
    pub kind: CharacterKind,
    /// The player is one of `IsoPlayer.players` (see `isLocalPlayer`).
    pub in_players: bool,
    /// The player is `IsoPlayer.getInstance()`.
    pub player_instance: bool,
    /// `stats`.
    pub stats: Stats,
    /// `moodles`.
    pub moodles: Moodles,
    /// `characterTraits`.
    pub character_traits: CharacterTraits,
    /// `xp`.
    pub xp: XP,
    /// `perkList`.
    pub perk_list: Vec<PerkInfo>,
    /// `descriptor`.
    pub descriptor: Option<SurvivorDescRef<I>>,
    /// `cheats`.
    pub cheats: PlayerCheats,
    /// `bodyDamage`.
    pub body_damage: BodyDamage,
    /// `IsoPlayer.nutrition` (players only).
    pub nutrition: Option<Nutrition>,
    /// `IsoPlayer.fitness` (players only).
    pub fitness: Option<Fitness>,
    /// Inputs from unported world systems.
    pub environment: CharacterEnvironment,
    /// `inventory`, the hand items and `wornItems`.
    pub items: CharacterItems,
    /// `clothingWetness` (players only).
    pub clothing_wetness: Option<ClothingWetness>,
    /// `health`.
    pub health: f32,
    /// `dead`.
    pub dead: bool,
    /// `asleep`.
    pub asleep: bool,
    /// `isResting`.
    pub is_resting: bool,
    /// `isSitOnGround()`.
    pub sit_on_ground: bool,
    /// `isSittingOnFurniture()`.
    pub sitting_on_furniture: bool,
    /// `running` (see [`GameCharacter::is_running`]).
    pub running: bool,
    /// `IsoPlayer.isPlayerMoving`.
    pub player_moving: bool,
    /// `IsoPlayer.moodleCantSprint`.
    pub moodle_cant_sprint: bool,
    /// `levelUpMultiplier`.
    pub level_up_multiplier: f32,
    /// `maxWeight`.
    pub max_weight: i32,
    /// `maxWeightBase`.
    pub max_weight_base: i32,
    /// `IsoPlayer.hoursSurvived` (other characters use the world age).
    pub hours_survived: f64,
    /// `idleSquareTime`.
    pub idle_square_time: f32,
    /// `bedType`.
    pub bed_type: String,
    /// `timeOfSleep`.
    pub time_of_sleep: f32,
    /// `delayToActuallySleep`.
    pub delay_to_actually_sleep: f32,
    /// `forceWakeUpTime`.
    pub force_wake_up_time: f32,
    /// `forceWakeUp`.
    pub force_wake_up: bool,
    /// `sleepingTabletEffect`.
    pub sleeping_tablet_effect: f32,
    /// `sleepingTabletDelta`.
    pub sleeping_tablet_delta: f32,
    /// `betaEffect`.
    pub beta_effect: f32,
    /// `betaDelta`.
    pub beta_delta: f32,
    /// `depressEffect`.
    pub depress_effect: f32,
    /// `depressDelta`.
    pub depress_delta: f32,
    /// `depressFirstTakeTime`.
    pub depress_first_take_time: f32,
    /// `painEffect`.
    pub pain_effect: f32,
    /// `painDelta`.
    pub pain_delta: f32,
    /// `corpseSicknessRate`.
    pub corpse_sickness_rate: f32,
    /// `age`.
    pub age: i32,
    /// `zombieKills`.
    pub zombie_kills: i32,
    /// `survivorKills`.
    pub survivor_kills: i32,
    /// `knownRecipes`.
    pub known_recipes: Vec<String>,
    /// `reduceInfectionPower`.
    pub reduce_infection_power: f32,
    /// `timeSinceLastSmoke`.
    pub time_since_last_smoke: f32,
    /// `isReading`.
    pub is_reading: bool,
    /// `numSurvivorsInVicinity`.
    pub num_survivors_in_vicinity: i32,
    /// `clothingDiscomfortModifier`.
    pub clothing_discomfort_modifier: f32,
    /// `IsoPlayer.sleepingPillsTaken`.
    pub sleeping_pills_taken: i32,
    /// `IsoPlayer.lastPillsTaken` (calendar millis).
    pub last_pills_taken: i64,
    /// `IsoPlayer.maxWeightDelta`.
    pub max_weight_delta: f32,
    /// `IsoPlayer.bleedingLevel`.
    pub bleeding_level: i8,
    /// `IsoPlayer.moveSpeed`.
    pub move_speed: f32,
    /// `IsoZombie.isSkeleton` (zombies only).
    pub skeleton: bool,
    /// The state machine, its parameters and the action context
    /// (`StateMachineComponent`).
    pub state: state::StateMachineComponent<I>,
    /// The fields behind the callback variables and the state and combat
    /// code.
    pub vars: state::CharacterVars,
    /// `gameVariables`: the plain variable slots.
    pub game_variables: pz_anim::MapVariableSource,
    /// The pathfinding fields (`finder`, `path2`, ...).
    pub path: path_mover::PathMoverState,
    /// `attackVars`.
    pub attack_vars: states::swipe::AttackVars,
    /// `IsoLivingCharacter.bareHands` (`Base.BareHands`), when created.
    pub bare_hands: Option<pz_inventory::InventoryItem>,
    /// `IsoZombie`'s fields (defaults for other characters).
    pub zombie: Box<zombie::ZombieFields>,
    /// The fields zombies read and write on the characters they hunt.
    pub seen: zombie::SeenFields,
    /// `IsoMovingObject.table` (the mod data).
    pub mod_data: Option<pz_inventory::lua_table::LuaTable>,
    /// `getVisual()` (`IsoPlayer.humanVisual`) as saved; `None` until loaded.
    pub human_visual: Option<pz_world::save::visual::HumanVisual>,
    /// `savedInventoryItems`: the list saved indices refer to, as the ids of
    /// the inventory's items (`None` for an item that could not be loaded).
    pub saved_inventory_items: Vec<Option<i32>>,
    /// `readBooks`.
    pub read_books: Vec<save::ReadBook>,
    /// `lastHourSleeped`.
    pub last_hour_sleeped: i32,
    /// `beardGrowTiming`.
    pub beard_grow_timing: f32,
    /// `hairGrowTiming`.
    pub hair_grow_timing: f32,
    /// `deathDragDown`.
    pub death_drag_down: bool,
    /// `readLiterature`: title -> day read.
    pub read_literature: pz_scripts::java::JHashMap<i32>,
    /// `readPrintMedia` (a `HashSet`).
    pub read_print_media: pz_scripts::java::JHashMap<()>,
    /// `lastAnimalPet`.
    pub last_animal_pet: i64,
    /// `knownMediaLines` (a `HashSet`).
    pub known_media_lines: pz_scripts::java::JHashMap<()>,
    /// `savedVehicleX`/`Y`/`Seat`/`Running`: the vehicle a loaded player
    /// sat in (`None`: seat -1).
    pub saved_vehicle: Option<save::SavedVehicle>,
    /// `IsoPlayer`'s saved fields not kept elsewhere.
    pub player_fields: Box<save::PlayerFields>,
    /// `IsoPlayer`'s control-loop state (players; defaults otherwise).
    pub player: Box<player::PlayerControl>,
    /// `ballisticsController` (a shooter aiming a firearm).
    pub ballistics_controller: Option<Box<firearm::BallisticsController>>,
    /// `ballisticsTarget` (a character a ballistics query may hit).
    pub ballistics_target: Option<Box<firearm::BallisticsTarget>>,
    /// What the character side tracks of its ragdoll (the controller is
    /// pz-ragdoll's, beside the animation player).
    pub ragdoll: firearm::ragdoll::RagdollTracking,
    /// `hitInfoList` (the ranged path's targets).
    pub hit_info_list: Vec<states::swipe::HitInfo>,
    /// `wornClothingCanRagdoll` (no worn item has the `NO_RAGDOLL` tag).
    pub worn_clothing_can_ragdoll: bool,
}

impl<I: 'static> GameCharacter<I> {
    /// The field initializers and the parts of the constructors that set up
    /// simulation state: every registered trait unset, one moodle per
    /// registered `MoodleType`, and for players a `Nutrition`, a `Fitness`
    /// and a `Thermoregulator`.
    pub fn new(sm: &ScriptManager, kind: CharacterKind) -> Self {
        let player = kind == CharacterKind::Player;
        let body_damage = BodyDamage::new(sm, player);
        GameCharacter {
            id: 0,
            kind,
            in_players: false,
            player_instance: false,
            stats: Stats::default(),
            moodles: Moodles::new(sm),
            character_traits: CharacterTraits::new(sm),
            xp: XP::default(),
            perk_list: Vec::new(),
            descriptor: None,
            cheats: PlayerCheats::default(),
            body_damage,
            nutrition: player.then(Nutrition::default),
            fitness: player.then(Fitness::default),
            environment: CharacterEnvironment::default(),
            items: CharacterItems::default(),
            clothing_wetness: player.then(ClothingWetness::default),
            health: 1.0,
            dead: false,
            asleep: false,
            is_resting: false,
            sit_on_ground: false,
            sitting_on_furniture: false,
            running: false,
            player_moving: false,
            moodle_cant_sprint: false,
            level_up_multiplier: 2.5,
            max_weight: 8,
            max_weight_base: 8,
            hours_survived: 0.0,
            idle_square_time: 0.0,
            bed_type: "averageBed".to_string(),
            time_of_sleep: 0.0,
            delay_to_actually_sleep: 0.0,
            force_wake_up_time: -1.0,
            force_wake_up: false,
            sleeping_tablet_effect: 0.0,
            sleeping_tablet_delta: 1.0,
            beta_effect: 0.0,
            beta_delta: 0.0,
            depress_effect: 0.0,
            depress_delta: 0.0,
            depress_first_take_time: -1.0,
            pain_effect: 0.0,
            pain_delta: 0.0,
            corpse_sickness_rate: 0.0,
            age: 25,
            zombie_kills: 0,
            survivor_kills: 0,
            known_recipes: Vec::new(),
            reduce_infection_power: 0.0,
            time_since_last_smoke: 0.0,
            is_reading: false,
            num_survivors_in_vicinity: 0,
            clothing_discomfort_modifier: 0.0,
            sleeping_pills_taken: 0,
            last_pills_taken: 0,
            max_weight_delta: 1.0,
            bleeding_level: 0,
            move_speed: 0.06,
            skeleton: false,
            state: state::StateMachineComponent::default(),
            vars: state::CharacterVars::default(),
            game_variables: pz_anim::MapVariableSource::new(),
            path: path_mover::PathMoverState::default(),
            attack_vars: states::swipe::AttackVars::default(),
            bare_hands: None,
            zombie: Box::default(),
            seen: zombie::SeenFields::default(),
            mod_data: None,
            human_visual: None,
            saved_inventory_items: Vec::new(),
            read_books: Vec::new(),
            last_hour_sleeped: 0,
            beard_grow_timing: -1.0,
            hair_grow_timing: -1.0,
            death_drag_down: false,
            read_literature: pz_scripts::java::JHashMap::new(),
            read_print_media: pz_scripts::java::JHashMap::new(),
            last_animal_pet: 0,
            known_media_lines: pz_scripts::java::JHashMap::new(),
            saved_vehicle: None,
            player_fields: Box::default(),
            player: Box::default(),
            ballistics_controller: None,
            ballistics_target: None,
            ragdoll: Default::default(),
            hit_info_list: Vec::new(),
            worn_clothing_can_ragdoll: true,
        }
    }

    /// `instanceof IsoPlayer`.
    pub fn is_player(&self) -> bool {
        self.kind == CharacterKind::Player
    }
    /// `isZombie`.
    pub fn is_zombie(&self) -> bool {
        self.kind == CharacterKind::Zombie
    }
    /// `isAnimal`.
    pub fn is_animal(&self) -> bool {
        self.kind == CharacterKind::Animal
    }
    /// `IsoPlayer.isLocalPlayer`: not on a server, and one of the players.
    pub fn is_local_player(&self, ctx: &dyn SimContext) -> bool {
        self.is_player() && !ctx.is_server() && self.in_players
    }
    /// `isDead`.
    pub fn is_dead(&self) -> bool {
        self.health <= 0.0 || self.body_damage.health() <= 0.0
    }
    /// `isAsleep`.
    pub fn is_asleep(&self) -> bool {
        self.asleep
    }
    /// `isRunning`: not with an endurance moodle at level 3 or more.
    pub fn is_running(&self, sm: &ScriptManager) -> bool {
        if self.moodles.moodle_level_of(sm, "ENDURANCE").is_ok_and(|l| l >= 3) {
            return false;
        }
        self.running
    }
    /// `canSprint`: not for a player that may not sprint, always in the
    /// tutorial, not with a no-sprint hand item, and only below endurance
    /// moodle level 2.
    pub fn can_sprint(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> bool {
        if self.is_player() && !self.environment.allow_sprint {
            return false;
        }
        if ctx.game_mode() == "Tutorial" {
            return true;
        }
        if self.items.hand_item_equipped_no_sprint(sm) {
            return false;
        }
        self.moodles.moodle_level_of(sm, "ENDURANCE").map_or(true, |l| l < 2)
    }
    /// `isSprinting`.
    pub fn is_sprinting(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> bool {
        if self.environment.sprinting && !self.can_sprint(sm, ctx) {
            return false;
        }
        self.environment.sprinting
    }
    /// `isSneaking`.
    pub fn is_sneaking(&self) -> bool {
        self.environment.sneaking
    }
    /// `isInCombat`: a very close zombie or at least three chasing.
    pub fn is_in_combat(&self) -> bool {
        self.stats.num_very_close_zombies() > 0 || self.stats.num_chasing_zombies() >= 3
    }
    /// `isSitting`.
    pub fn is_sitting(&self) -> bool {
        self.sit_on_ground || self.sitting_on_furniture
    }
    /// `isGodMod`.
    pub fn is_god_mod(&self, ctx: &dyn SimContext) -> bool {
        self.cheats.is_set(ctx, "GOD_MODE")
    }

    /// `IsoPlayer.setSleepingPillsTaken`: one more while intoxicated, and
    /// the time of the last pill.
    pub fn set_sleeping_pills_taken(&mut self, chars: &Characters, ctx: &dyn SimContext, n: i32) -> ScriptResult {
        if self.is_god_mod(ctx) {
            self.sleeping_pills_taken = 0;
            return Ok(());
        }
        self.sleeping_pills_taken = n;
        if self.stats.get(&chars.sim.character_stats.field("INTOXICATION")?) > 10.0 {
            self.sleeping_pills_taken += 1;
        }
        self.last_pills_taken = ctx.calendar_time_in_millis();
        Ok(())
    }

    /// `IsoPlayer.updateSleepingPillsTaken` (from the player update, its
    /// owner's): one pill wears off two hours after the last.
    pub fn update_sleeping_pills_taken(&mut self, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.sleeping_pills_taken > 0
            && self.last_pills_taken > 0
            && ctx.calendar_time_in_millis() - self.last_pills_taken > 0x6DDD00
        {
            let n = self.sleeping_pills_taken - 1;
            self.set_sleeping_pills_taken(chars, ctx, n)?;
        }
        Ok(())
    }
    /// `isUnlimitedEndurance`.
    pub fn is_unlimited_endurance(&self, ctx: &dyn SimContext) -> bool {
        self.cheats.is_set(ctx, "UNLIMITED_ENDURANCE")
    }
    /// `IsoPlayer.isGhostMode`: `isInvisible`.
    pub fn is_ghost_mode(&self, ctx: &dyn SimContext) -> bool {
        self.cheats.is_set(ctx, "INVISIBLE")
    }
    /// `getHoursSurvived`.
    pub fn hours_survived(&self, ctx: &dyn SimContext) -> f64 {
        if self.is_player() {
            self.hours_survived
        } else {
            ctx.world_age_hours()
        }
    }
    /// `getMaxWeight`.
    pub fn max_weight(&self) -> i32 {
        self.max_weight
    }

    /// `hasTrait` / `characterTraits.get` for the `CharacterTrait` constant
    /// `field`.
    pub fn has_trait(&self, sm: &ScriptManager, field: &str) -> ScriptResult<bool> {
        self.character_traits.get(CharacterTrait::constant(sm, field).as_ref())
    }

    /// `getStats().get(CharacterStat.<field>)`.
    pub fn stat(&self, chars: &Characters, field: &str) -> ScriptResult<f32> {
        Ok(self.stats.get(&chars.sim.character_stats.field(field)?))
    }

    /// `getRecoveryMod`.
    pub fn recovery_mod(&self, sm: &ScriptManager, chars: &Characters) -> ScriptResult<f32> {
        let level = self.perk_level(chars.perks.constant("Fitness").as_ref());
        let mut m = match level {
            0 => 0.7,
            1 => 0.8,
            2 => 0.9,
            3 => 1.0,
            4 => 1.1,
            5 => 1.2,
            6 => 1.3,
            7 => 1.4,
            8 => 1.5,
            9 => 1.55,
            10 => 1.6,
            _ => 0.0f32,
        };
        if self.has_trait(sm, "OBESE")? {
            m *= 0.4;
        }
        if self.has_trait(sm, "OVERWEIGHT")? {
            m *= 0.7;
        }
        if self.has_trait(sm, "VERY_UNDERWEIGHT")? {
            m *= 0.7;
        }
        if self.has_trait(sm, "EMACIATED")? {
            m *= 0.3;
        }
        if let (true, Some(n)) = (self.is_player(), &self.nutrition) {
            if n.lipids() < -1500.0 {
                m *= 0.2;
            } else if n.lipids() < -1000.0 {
                m *= 0.5;
            }
            if n.proteins() < -1500.0 {
                m *= 0.2;
            } else if n.proteins() < -1000.0 {
                m *= 0.5;
            }
        }
        Ok(m)
    }

    /// `getThirstMultiplier`: the thermoregulator's fluids multiplier.
    pub fn thirst_multiplier(&self) -> f64 {
        self.body_damage.thermoregulator().map_or(1.0, |t| t.fluids_multiplier())
    }
    /// `getFatiqueMultiplier`: the thermoregulator's fatigue multiplier.
    pub fn fatique_multiplier(&self) -> f64 {
        self.body_damage.thermoregulator().map_or(1.0, |t| t.fatigue_multiplier())
    }
    /// `getHungerMultiplier`.
    pub fn hunger_multiplier(&self) -> f64 {
        1.0
    }
}

/// Shared handle used where the original passes the character around.
pub type GameCharacterRef<I> = Rc<std::cell::RefCell<GameCharacter<I>>>;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod combat_tests;
