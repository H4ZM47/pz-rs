//! What the animal logic reads from and does to the rest of the game, behind
//! small traits so the character side (`pz-characters`' animal module) and
//! the world can plug in later:
//!
//! * [`AnimalClock`]: `GameTime` and the `PZCalendar`s the original builds.
//! * [`AnimalSandbox`]: the sandbox and server options the animal code reads.
//! * [`AnimalCharacter`]: the `IsoGameCharacter`/`IsoPlayer` state of the
//!   animal itself (health, hours survived, sex, hunger and thirst, ...).
//!   [`CharacterValues`] is a stand-alone implementation for tools, tests
//!   and newly created animals.
//! * [`AnimalWorld`]: designation zones, hutches, troughs, items, vehicles,
//!   other animals and world sounds.
//! * `Rand` is `pz_world::GameRand`.
//!
//! Handles to world things (squares, zones, hutches, troughs, items, other
//! animals) are [`ObjectRef`]s; their meaning is the implementer's.

use crate::definitions::AnimalDefinitions;
use crate::AnimalDefinitionTables;
use pz_world::save::animal::CharacterStatDecl;
use pz_world::GameRand;
use std::rc::Rc;

/// An opaque handle to something the [`AnimalWorld`] implementer owns.
pub type ObjectRef = u64;

/// `GameTime.getInstance()` as the animal code reads it.
pub trait AnimalClock {
    /// `getHour()`.
    fn hour(&self) -> i32;
    /// `getMonth()` (0-based).
    fn month(&self) -> i32;
    /// `getCalender().getTimeInMillis()`.
    fn calendar_millis(&self) -> i64;
    /// `getMultiplier()`.
    fn multiplier(&self) -> f32;
    /// `getWorldAgeDaysSinceBegin()`.
    fn world_age_days_since_begin(&self) -> f64;
    /// `(MONTH, HOUR_OF_DAY)` of a `Calendar` in the JVM's default time zone
    /// set to `millis` (`PZCalendar.getInstance()` and `GameTime`'s own
    /// calendar are both `GregorianCalendar`s in that zone).
    fn calendar_month_hour(&self, millis: i64) -> (i32, i32) {
        let f =
            pz_core::calendar::calendar_fields(&pz_core::weather::calendar::default_zone(), millis);
        (f.month, f.hour_of_day)
    }
}

/// The options the animal code reads: `SandboxOptions.animal*` (enum
/// options are their 1-based values) and
/// `ServerOptions.ultraSpeedDoesnotAffectToAnimals`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimalSandbox {
    pub animal_stats_modifier: i32,
    pub animal_meta_stats_modifier: i32,
    pub animal_pregnancy_time: i32,
    pub animal_age_modifier: i32,
    pub animal_milk_inc_modifier: i32,
    pub animal_wool_inc_modifier: i32,
    pub animal_meta_predator: bool,
    pub animal_mating_season: bool,
    pub ultra_speed_doesnot_affect_to_animals: bool,
}

impl Default for AnimalSandbox {
    /// The options' defaults (`SandboxOptions`' `newEnumOption` /
    /// `newBooleanOption` values, `ServerOptions`' initialiser).
    fn default() -> Self {
        AnimalSandbox {
            animal_stats_modifier: 4,
            animal_meta_stats_modifier: 4,
            animal_pregnancy_time: 2,
            animal_age_modifier: 3,
            animal_milk_inc_modifier: 3,
            animal_wool_inc_modifier: 3,
            animal_meta_predator: false,
            animal_mating_season: true,
            ultra_speed_doesnot_affect_to_animals: false,
        }
    }
}

impl AnimalSandbox {
    /// The options by their Java names (`AnimalStatsModifier`, ...,
    /// `UltraSpeedDoesnotAffectToAnimals`), from any option store. `None`
    /// if one is missing.
    pub fn from_options(
        int: impl Fn(&str) -> Option<i32>,
        boolean: impl Fn(&str) -> Option<bool>,
    ) -> Option<Self> {
        Some(AnimalSandbox {
            animal_stats_modifier: int("AnimalStatsModifier")?,
            animal_meta_stats_modifier: int("AnimalMetaStatsModifier")?,
            animal_pregnancy_time: int("AnimalPregnancyTime")?,
            animal_age_modifier: int("AnimalAgeModifier")?,
            animal_milk_inc_modifier: int("AnimalMilkIncModifier")?,
            animal_wool_inc_modifier: int("AnimalWoolIncModifier")?,
            animal_meta_predator: boolean("AnimalMetaPredator")?,
            animal_mating_season: boolean("AnimalMatingSeason")?,
            ultra_speed_doesnot_affect_to_animals: boolean("UltraSpeedDoesnotAffectToAnimals")?,
        })
    }
}

/// The two `CharacterStat`s the animal code uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimalStat {
    Hunger,
    Thirst,
}

impl AnimalStat {
    /// The `CharacterStat` static field name.
    pub fn field(self) -> &'static str {
        match self {
            AnimalStat::Hunger => "HUNGER",
            AnimalStat::Thirst => "THIRST",
        }
    }
}

/// The character (`IsoGameCharacter`/`IsoPlayer`) side of one animal: the
/// state the original keeps on the character rather than on `IsoAnimal` or
/// `AnimalData`, and the character behaviour the animal code calls.
pub trait AnimalCharacter {
    /// The `health` field (`getHealth()`).
    fn health(&self) -> f32;
    /// Writes the `health` field (`IsoAnimal.setHealth` does the invincible
    /// check before calling this).
    fn set_health_field(&mut self, health: f32);
    /// `IsoPlayer.getHoursSurvived()`.
    fn hours_survived(&self) -> f64;
    /// `IsoPlayer.setHoursSurvived(hours)`.
    fn set_hours_survived(&mut self, hours: f64);
    /// `getDescriptor().isFemale()`.
    fn is_female(&self) -> bool;
    /// `getDescriptor().setFemale(b)` (`setFemale`).
    fn set_female(&mut self, female: bool);
    /// `getStats().get(stat)`.
    fn stat(&self, stat: AnimalStat) -> f32;
    /// `getStats().set(stat, value)`: clamped to the stat's range.
    fn set_stat(&mut self, stat: AnimalStat, value: f32);
    /// The stat's default value (`Stats.reset`).
    fn stat_default(&self, stat: AnimalStat) -> f32;
    /// `Stats.add`.
    fn add_stat(&mut self, stat: AnimalStat, amount: f32) {
        let v = self.stat(stat) + amount;
        self.set_stat(stat, v);
    }
    /// `Stats.remove`.
    fn remove_stat(&mut self, stat: AnimalStat, amount: f32) {
        self.add_stat(stat, -amount);
    }
    /// `Stats.reset`.
    fn reset_stat(&mut self, stat: AnimalStat) {
        let d = self.stat_default(stat);
        self.set_stat(stat, d);
    }
    /// `isDead()`: health at or below 0, or the body damage's health.
    fn is_dead(&self) -> bool {
        self.health() <= 0.0
    }
    /// `getVariableString(name)` (`""` when unset).
    fn variable(&self, name: &str) -> String;
    /// `setVariable(name, value)`.
    fn set_variable(&mut self, name: &str, value: &str);
    /// `clearVariable(name)`.
    fn clear_variable(&mut self, name: &str);
    /// `getStateEventDelayTimer()`.
    fn state_event_delay_timer(&self) -> f32;
    /// `setStateEventDelayTimer(t)`.
    fn set_state_event_delay_timer(&mut self, t: f32);
    /// `getX()`, `getY()`, `getZ()`.
    fn position(&self) -> (f32, f32, f32);
    /// `getForwardIsoDirection().ordinal()`.
    fn dir(&self) -> i32;
    /// `setDir(IsoDirections.values()[dir])`.
    fn set_dir(&mut self, dir: i32);
    /// `getCurrentSquare()` (`getSquare()`).
    fn current_square(&self) -> Option<ObjectRef> {
        None
    }
    /// `isExistInTheWorld()`: on a square that lists the animal.
    fn exists_in_world(&self) -> bool {
        false
    }
    /// `getVehicle()`.
    fn vehicle(&self) -> Option<ObjectRef> {
        None
    }
    /// `getContainer()`: the inventory the animal is carried in.
    fn container(&self) -> Option<ObjectRef> {
        None
    }
    /// `getOnlineID()`.
    fn online_id(&self) -> i16 {
        1
    }
    /// `splatBloodFloorBig()` (draws `Rand` when the animal is on a square).
    fn splat_blood_floor_big(&mut self, _rand: &mut dyn GameRand) {}
    /// `setHitReaction(reaction)`.
    fn set_hit_reaction(&mut self, _reaction: &str) {}
    /// `setAttackedBy(null)`.
    fn clear_attacked_by(&mut self) {}
    /// The wool crossed the fleece threshold (`setWoolQuantity`):
    /// `clearVariable("idleAction")`, `getPathFindBehavior2().reset()`,
    /// `resetModel()`.
    fn on_fleece_changed(&mut self) {
        self.clear_variable("idleAction");
    }
    /// `setStateEventDelayTimer(0)` and similar resets after eating
    /// (`AnimalData.resetEatingCheck` also clears `idleAction`).
    fn stop_all_movement_now(&mut self) {}
}

/// A plain-data [`AnimalCharacter`]: the character values of an animal that
/// is not (yet) a live game character, such as a baby or grown animal
/// before the world adds it, or a tool's animal. `stats` holds every
/// `CharacterStat` in `ORDERED_STATS` order (as the save writes them).
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterValues {
    pub stat_defs: Rc<[CharacterStatDecl]>,
    pub stats: Vec<f32>,
    pub health: f32,
    pub hours_survived: f64,
    pub female: bool,
    pub variables: Vec<(String, String)>,
    pub state_event_delay_timer: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub dir: i32,
    pub online_id: i16,
    pub square: Option<ObjectRef>,
    pub vehicle: Option<ObjectRef>,
    pub container: Option<ObjectRef>,
    pub exists_in_world: bool,
    /// What constructing the character drew (see
    /// [`character_construction_draws`]).
    pub construction: CharacterConstruction,
}

/// The `Rand` draws of constructing an animal's character
/// (`new IsoAnimal(...)` → `super(cell, new SurvivorDesc(), ...)`): the
/// six `SurvivorDesc.doStats` coin flips (bravery, aggressiveness, loner,
/// temper, favourindoors, loyalty: 10 or 0) and the `IsoGameCharacter`
/// patience (`Rand.Next(patienceMin, patienceMax)`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterConstruction {
    pub descriptor_stats: [f32; 6],
    pub patience: i32,
}

/// The draws `new IsoAnimal(cell, ...)` makes before any animal code runs.
/// `patience_min`/`patience_max` are `IsoGameCharacter`'s field
/// initialisers.
pub fn character_construction_draws(
    rand: &mut dyn GameRand,
    patience_min: i32,
    patience_max: i32,
) -> CharacterConstruction {
    let mut c = CharacterConstruction::default();
    for s in &mut c.descriptor_stats {
        *s = if rand.next(2) == 0 { 10.0 } else { 0.0 };
    }
    c.patience = rand.next_range(patience_min, patience_max);
    c
}

impl CharacterValues {
    /// A new character: every stat at its default, health 1 and the other
    /// fields as `new IsoPlayer` leaves them for an animal.
    pub fn new(stat_defs: Rc<[CharacterStatDecl]>) -> Self {
        let stats = stat_defs.iter().map(|d| d.default).collect();
        CharacterValues {
            stat_defs,
            stats,
            health: 1.0,
            hours_survived: 0.0,
            female: false,
            variables: Vec::new(),
            state_event_delay_timer: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dir: 0,
            online_id: 1,
            square: None,
            vehicle: None,
            container: None,
            exists_in_world: false,
            construction: CharacterConstruction::default(),
        }
    }

    fn index(&self, stat: AnimalStat) -> usize {
        self.stat_defs
            .iter()
            .position(|d| d.field == stat.field())
            .unwrap_or_else(|| panic!("CharacterStat {} not in the table", stat.field()))
    }
}

impl AnimalCharacter for CharacterValues {
    fn health(&self) -> f32 {
        self.health
    }
    fn set_health_field(&mut self, health: f32) {
        self.health = health;
    }
    fn hours_survived(&self) -> f64 {
        self.hours_survived
    }
    fn set_hours_survived(&mut self, hours: f64) {
        self.hours_survived = hours;
    }
    fn is_female(&self) -> bool {
        self.female
    }
    fn set_female(&mut self, female: bool) {
        self.female = female;
    }
    fn stat(&self, stat: AnimalStat) -> f32 {
        self.stats[self.index(stat)]
    }
    fn set_stat(&mut self, stat: AnimalStat, value: f32) {
        let i = self.index(stat);
        let d = &self.stat_defs[i];
        self.stats[i] = crate::java::clamp(value, d.minimum, d.maximum);
    }
    fn stat_default(&self, stat: AnimalStat) -> f32 {
        self.stat_defs[self.index(stat)].default
    }
    fn variable(&self, name: &str) -> String {
        self.variables
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
    fn set_variable(&mut self, name: &str, value: &str) {
        match self
            .variables
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
        {
            Some(e) => e.1 = value.to_string(),
            None => self.variables.push((name.to_string(), value.to_string())),
        }
    }
    fn clear_variable(&mut self, name: &str) {
        self.variables
            .retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
    fn state_event_delay_timer(&self) -> f32 {
        self.state_event_delay_timer
    }
    fn set_state_event_delay_timer(&mut self, t: f32) {
        self.state_event_delay_timer = t;
    }
    fn position(&self) -> (f32, f32, f32) {
        (self.x, self.y, self.z)
    }
    fn dir(&self) -> i32 {
        self.dir
    }
    fn set_dir(&mut self, dir: i32) {
        self.dir = dir;
    }
    fn current_square(&self) -> Option<ObjectRef> {
        self.square
    }
    fn exists_in_world(&self) -> bool {
        self.exists_in_world
    }
    fn vehicle(&self) -> Option<ObjectRef> {
        self.vehicle
    }
    fn container(&self) -> Option<ObjectRef> {
        self.container
    }
    fn online_id(&self) -> i16 {
        self.online_id
    }
}

/// What an item an animal may eat looks like (`Food`,
/// `DrainableComboItem` or another `InventoryItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct FeedItem {
    /// `getFullType()`.
    pub full_type: String,
    /// `getAnimalFeedType()`.
    pub animal_feed_type: Option<String>,
    pub kind: FeedItemKind,
    /// `getFluidContainer()` holds only the breed's milk or
    /// `Fluid.AnimalMilk` (`isPureFluid`).
    pub pure_milk: bool,
    /// The item is in a feeding trough's container.
    pub in_trough: Option<ObjectRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeedItemKind {
    /// `Food`: `getFoodType()`, `isRotten()`, `getHungerChange()`,
    /// `getHungChange()`, `getThirstChange()`.
    Food {
        food_type: Option<String>,
        rotten: bool,
        hunger_change: f32,
        hung_change: f32,
        thirst_change: f32,
    },
    /// `DrainableComboItem`: `getCurrentUses()`.
    Drainable {
        current_uses: i32,
    },
    Other,
}

/// A fertilized egg's hatch data (`IsoAnimal.createEgg`).
#[derive(Clone, Debug, PartialEq)]
pub struct EggHatch {
    pub mother_id: i32,
    pub time_to_hatch: i32,
    pub animal_hatch: Option<String>,
    pub base_hunger: f32,
    pub hung_change: f32,
    pub animal_hatch_breed: Option<String>,
    pub genome: crate::JHashMap<crate::AnimalGene>,
}

/// The world as the animal code sees it. Every method has the original's
/// meaning for the animal it is called for; the defaults describe an animal
/// that is nowhere (no square, zone, hutch or vehicle) on a dry day.
#[allow(unused_variables)]
pub trait AnimalWorld {
    /// `RainManager.isRaining()`.
    fn is_raining(&self) -> bool {
        false
    }
    /// `IsoPuddles.getInstance().getPuddlesSize()`.
    fn puddles_size(&self) -> f32 {
        0.0
    }

    /// `IsoWorld.instance.metaGrid.animalZoneHandler.getZone(uuid) != null`.
    fn animal_zone_exists(&self, msb: i64, lsb: i64) -> bool {
        false
    }

    // --- designation zones (`DesignationZoneAnimal`) ---
    /// `DesignationZoneAnimal.getZoneF(x, y, z)`.
    fn zone_at(&self, x: f32, y: f32, z: f32) -> Option<ObjectRef> {
        None
    }
    /// `setDZone`: `old.removeAnimal(this)`, `new.addAnimal(this)`.
    fn move_animal_to_zone(&mut self, old: Option<ObjectRef>, new: Option<ObjectRef>) {}
    /// `DesignationZoneAnimal.getAllDZones(list, zone, null)`: the zones
    /// connected to `zone`, in the original's order.
    fn connected_zones(&self, zone: Option<ObjectRef>) -> Vec<ObjectRef> {
        Vec::new()
    }
    /// `getFullZoneSize()`.
    fn zone_full_size(&self, zone: ObjectRef) -> i32 {
        0
    }
    /// `getNbOfDung()`.
    fn zone_dung_count(&self, zone: ObjectRef) -> i32 {
        0
    }
    /// `getNbOfFeather()`.
    fn zone_feather_count(&self, zone: ObjectRef) -> i32 {
        0
    }
    /// `zone.getRandomSquare()` (draws `Rand`).
    fn zone_random_square(
        &mut self,
        zone: ObjectRef,
        rand: &mut dyn GameRand,
    ) -> Option<ObjectRef> {
        None
    }
    /// `zone.troughs`.
    fn zone_troughs(&self, zone: ObjectRef) -> Vec<ObjectRef> {
        Vec::new()
    }
    /// `zone.foodOnGround` (`IsoWorldInventoryObject`s).
    fn zone_food_on_ground(&self, zone: ObjectRef) -> Vec<ObjectRef> {
        Vec::new()
    }
    /// `zone.foodOnGround.remove(worldItem)`.
    fn zone_remove_food_on_ground(&mut self, zone: ObjectRef, world_item: ObjectRef) {}
    /// `zone.nearWaterSquares.getFirst()`.
    fn zone_first_near_water_square(&self, zone: ObjectRef) -> Option<ObjectRef> {
        None
    }
    /// `zone.getAnimals()`.
    fn zone_animals(&self, zone: ObjectRef) -> Vec<ObjectRef> {
        Vec::new()
    }
    /// `zone.getHutchsConnected()`: each hutch and its `type`.
    fn zone_hutches(&self, zone: ObjectRef) -> Vec<(ObjectRef, String)> {
        Vec::new()
    }

    // --- squares and items ---
    /// `square.isFree(false)`.
    fn square_is_free(&self, square: ObjectRef) -> bool {
        false
    }
    /// `InventoryItemFactory.CreateItem(type)`.
    fn create_item(&mut self, full_type: &str) -> Option<ObjectRef> {
        None
    }
    /// `item.setWeight(item.getWeight() * factor)`, `setActualWeight`,
    /// `setCustomWeight(true)`.
    fn scale_item_weight(&mut self, item: ObjectRef, factor: f32) {}
    /// `square.AddWorldInventoryItem(item, x, y, z[, true])`.
    fn add_world_item(&mut self, square: ObjectRef, item: ObjectRef, x: f32, y: f32, z: f32) {}
    /// `IsoWorld.instance.getCell().addToProcessItems(...)` for a dropped
    /// item and its world object.
    fn add_to_process_items(&mut self, item: ObjectRef) {}
    /// `container.addItem(item)` (the container the animal is carried in).
    fn container_add_item(&mut self, container: ObjectRef, item: ObjectRef) {}
    /// Makes `item` a fertilized egg (`Food.setFertilized` and the hatch
    /// fields).
    fn set_egg_hatch(&mut self, item: ObjectRef, hatch: EggHatch) {}
    /// The item an `IsoWorldInventoryObject` holds, and whether that object
    /// still exists in the world.
    fn world_item(&self, world_object: ObjectRef) -> Option<(ObjectRef, bool)> {
        None
    }
    /// What an item is, for eating.
    fn feed_item(&self, item: ObjectRef) -> Option<FeedItem> {
        None
    }
    /// `Food.setHungChange`, `Food.setThirstChange`.
    fn food_set_changes(&mut self, item: ObjectRef, hung_change: f32, thirst_change: f32) {}
    /// `DrainableComboItem.Use()`.
    fn drainable_use(&mut self, item: ObjectRef) {}
    /// Empties the milk out of the item's fluid container by `amount`
    /// (`getFluidContainer().removeFluid`); returns whether it is empty.
    fn remove_fluid(&mut self, item: ObjectRef, amount: f32) -> bool {
        true
    }
    /// Whether the item's fluid container is empty.
    fn fluid_empty(&self, item: ObjectRef) -> bool {
        true
    }
    /// The eaten item is used up: removed from the world square (`onground`)
    /// and the zones' food lists, or from its container (with the
    /// replace-on-use item and the network messages).
    fn consume_item(&mut self, item: ObjectRef, on_ground: bool, drainable: bool) {}
    /// After an animal ate from a trough's container
    /// (`checkOverlayAfterAnimalEat`).
    fn trough_eaten_from(&mut self, trough: ObjectRef) {}
    /// The items in a trough's container, in order.
    fn trough_items(&self, trough: ObjectRef) -> Vec<ObjectRef> {
        Vec::new()
    }
    /// `trough.getWater()`.
    fn trough_water(&self, trough: ObjectRef) -> f32 {
        0.0
    }
    /// `trough.removeWater(amount)` and the global object change.
    fn trough_remove_water(&mut self, trough: ObjectRef, amount: f32) {}
    /// The edible-for-this-animal items of the vehicle's `TrailerAnimalFood`
    /// part, in order (`None` when the part or its container is missing).
    fn vehicle_food_items(&self, vehicle: ObjectRef) -> Option<Vec<ObjectRef>> {
        None
    }
    /// `addEgg` in a vehicle: the egg goes into `TrailerAnimalEggs`, then
    /// `TrailerAnimalFood`; returns `false` when neither has room (the egg
    /// is then dropped on the vehicle's square).
    fn vehicle_store_egg(&mut self, vehicle: ObjectRef, egg: Option<ObjectRef>) -> bool {
        false
    }
    /// The vehicle's square (`vehicle.getSquare()`).
    fn vehicle_square(&self, vehicle: ObjectRef) -> Option<ObjectRef> {
        None
    }
    /// `square.removeGrass()` (or the network event on a client).
    fn remove_grass(&mut self, square: ObjectRef) {}

    // --- hutches ---
    /// `getHutchDirt()`.
    fn hutch_dirt(&self, hutch: ObjectRef) -> f32 {
        0.0
    }
    /// `setHutchDirt(dirt)`.
    fn set_hutch_dirt(&mut self, hutch: ObjectRef, dirt: f32) {}
    /// `isDoorClosed()`.
    fn hutch_door_closed(&self, hutch: ObjectRef) -> bool {
        false
    }
    /// `haveRoomForNewEggs()`.
    fn hutch_has_room_for_eggs(&self, hutch: ObjectRef) -> bool {
        false
    }
    /// `addMetaEgg(animal)`.
    fn hutch_add_meta_egg(&mut self, hutch: ObjectRef) -> bool {
        false
    }
    /// `addAnimalInNestBox(animal)`.
    fn hutch_add_animal_in_nest_box(&mut self, hutch: ObjectRef) -> bool {
        false
    }
    /// `killAnimal(animal)`.
    fn hutch_kill_animal(&mut self, hutch: ObjectRef) {}
    /// The square in front of the hutch's entrance
    /// (`square + getEnterSpotX/Y`, the hutch's z) and its integer
    /// coordinates.
    fn hutch_enter_square(&self, hutch: ObjectRef) -> Option<(ObjectRef, i32, i32, i32)> {
        None
    }
    /// `square.getChunk().addBloodSplat(x, y, z, type)`.
    fn add_blood_splat(&mut self, square: ObjectRef, x: f32, y: f32, z: f32, kind: i32) {}
    /// A square's integer coordinates.
    fn square_xyz(&self, square: ObjectRef) -> (i32, i32, i32) {
        (0, 0, 0)
    }
    /// `getBehavior().callToHutch(null, true)`.
    fn call_to_hutch(&mut self) {}
    /// `getBehavior()`: `setDoingBehavior(false)` then `checkBehavior()`
    /// (`check`) or `resetBehaviorAction()`.
    fn behavior_after_eating(&mut self, check: bool) {}
    /// `mother.getBehavior().blockMovement = false`.
    fn unblock_mother_movement(&mut self, mother: ObjectRef) {}
    /// `getCurrentSquare().DistToProper(mother.getCurrentSquare())`, `None`
    /// when either square is missing.
    fn distance_to(&self, other: ObjectRef) -> Option<f32> {
        None
    }

    // --- other animals ---
    /// Another live animal and its character.
    fn animal_mut(
        &mut self,
        animal: ObjectRef,
    ) -> Option<(&mut crate::Animal, &mut dyn AnimalCharacter)> {
        None
    }
    /// Adds a new animal (a baby, or the grown form of the animal) to the
    /// world or to the vehicle the parent is in, as the original does after
    /// constructing it; returns its handle.
    fn add_new_animal(
        &mut self,
        animal: crate::NewAnimal,
        into_vehicle: Option<ObjectRef>,
    ) -> ObjectRef {
        0
    }
    /// The animal grew up (`AnimalData.grow`): the old animal is removed
    /// (`delete()`, or replaced in its container or trailer) and the grown
    /// one added.
    fn replace_grown_animal(&mut self, grown: crate::NewAnimal) -> Option<ObjectRef> {
        None
    }
    /// `checkForChickenpocalypse(replacing)` or `checkForWater()` of a new
    /// animal at the parent's square (the original then deletes the new
    /// animal).
    fn new_animal_blocked(&mut self) -> bool {
        false
    }

    // --- the live animal in the world (the character side's IsoAnimal) ---
    /// `getVehicle().getCurrentSpeedKmHour()`, `None` outside a vehicle.
    fn vehicle_speed_kmh(&self) -> Option<f32> {
        None
    }
    /// `getSquare() != null && !getSquare().haveRoof` (rain reaches the
    /// animal).
    fn under_open_sky(&self) -> bool {
        false
    }
    /// `getData().getAttachedPlayer().removeAttachedAnimal(this)` (the
    /// leash), from `removeFromWorld`.
    fn detach_from_player(&mut self) {}
    /// `IsoAnimal.addToWorld`'s world part (the cell's object list; the
    /// population's own bookkeeping).
    fn added_to_world(&mut self) {}
    /// `removeFromWorld`'s world part: the cell's remove list and object
    /// list, the hutch's animal list and `AnimalManagerMain.removeFromWorld`.
    fn removed_from_world(&mut self) {}
    /// `AnimalPopulationManager.addToRecentlyRemoved(this)` (end of
    /// `removeFromWorld`).
    fn add_to_recently_removed(&mut self) {}
    /// `AnimalInstanceManager.getInstance().update(this)` (the online-id
    /// map; `IsoPlayer.updateInternal1` for an animal).
    fn instance_manager_update(&mut self) {}
}

/// A world with nothing in it ([`AnimalWorld`]'s defaults).
pub struct NoWorld;

impl AnimalWorld for NoWorld {}

/// The shared inputs of the animal code.
#[derive(Clone, Copy)]
pub struct AnimalCtx<'a> {
    pub defs: &'a AnimalDefinitionTables,
    pub clock: &'a dyn AnimalClock,
    pub sandbox: &'a AnimalSandbox,
}

impl<'a> AnimalCtx<'a> {
    /// `AnimalDefinitions.getDef(type)`.
    pub fn def(&self, animal_type: &str) -> Option<&'a AnimalDefinitions> {
        self.defs.def(animal_type)
    }
}

/// Everything an animal update needs besides the animal and its character.
pub struct AnimalEnv<'a> {
    pub ctx: AnimalCtx<'a>,
    pub rand: &'a mut dyn GameRand,
    pub world: &'a mut dyn AnimalWorld,
    /// The character stat table new animals are created with.
    pub stat_defs: Rc<[CharacterStatDecl]>,
    /// `IsoGameCharacter.patienceMin`/`patienceMax` (field initialisers),
    /// for the draws of constructing new animals.
    pub patience_range: (i32, i32),
}
