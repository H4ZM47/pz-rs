//! `IsoFireManager`: the list of burning fires (`FireStack`), starting
//! fires and smoke (`StartFire`, `StartSmoke`), the per-frame `Update`,
//! spreading (`IsoFire.Spread`), explosions (`explode`) and removal
//! (`extinctFire`, `RemoveAllOn`, the oldest-fire eviction in `Add`).
//!
//! The engine calls [`IsoFireManager::update`] once per frame (where
//! `IngameState` calls `IsoFireManager.Update()`) with a [`FireWorld`] over
//! the loaded cell, the frame's timing ([`FireFrame`]) and the game's
//! `Rand`. Sandbox and weather values are runtime input
//! ([`FireSettings`]); sound, light, heat and fire-only Lua events go out
//! through [`FireHooks`], and everything the square mutations call
//! (`RemoveTileObject`, `removeFromWorld`, render invalidation, collision
//! and pathfinding updates) through the [`MutationEnv`] parts in
//! [`FireMutation`].
//!
//! Multiplayer (packets, `ServerOptions.NoFire`, safehouses) is not ported:
//! this is the single-player game, where `GameServer.server` and
//! `GameClient.client` are false.

use crate::chunk::IsoCell;
use crate::fire::sound::{FireSoundHooks, FireSoundInfo, FireSounds};
use crate::fire::{
    FireCallbacks, FireEvent, FireOscillators, FireSquare, IsoFire, MAX_FIRE_OBJECTS,
};
use crate::isoregion::IsoRegions;
use crate::java::GameRand;
use crate::meta_grid::IsoMetaGrid;
use crate::object_slot::{IsoObjectRef, ObjectSlot};
use crate::objects::{IsoObject, ObjectId, ObjectKind, ObjectName};
use crate::sprite::{IsoFlagType, IsoSpriteManager, SpriteTables};
use crate::square_mutation::{
    dirty, DoorTables, MutationEnv, ObjectRef, PathfindSquareHooks, RenderSquareHooks,
    SquareMutationHooks,
};
use crate::{IsoGridSquare, SquareRef};
use std::cell::{Ref, RefMut};

/// The `IsoFlagType`s fire reads and sets that the sprite loader does not
/// name, resolved from the game's tables.
#[allow(non_snake_case)]
#[derive(Clone, Copy, Debug)]
pub struct FireFlags {
    pub burning: IsoFlagType,
    pub smoke: IsoFlagType,
    pub unflamable: IsoFlagType,
    pub DoorWallW: IsoFlagType,
    pub DoorWallN: IsoFlagType,
    pub bed: IsoFlagType,
    pub waterPiped: IsoFlagType,
    pub attachedS: Option<IsoFlagType>,
    pub attachedE: Option<IsoFlagType>,
    pub WallN: Option<IsoFlagType>,
    pub WallW: Option<IsoFlagType>,
    pub WallNW: Option<IsoFlagType>,
    /// `climbSheetTop{N,S,W,E}` and `climbSheet{N,S,W,E}` (sheet ropes).
    pub climbSheetTop: [Option<IsoFlagType>; 4],
    pub climbSheet: [Option<IsoFlagType>; 4],
}

impl FireFlags {
    pub fn resolve(t: &SpriteTables) -> Result<Self, String> {
        let req = |n: &str| {
            t.flag_from_string(n)
                .ok_or_else(|| format!("IsoFlagType table has no {n}"))
        };
        Ok(FireFlags {
            burning: req("burning")?,
            smoke: req("smoke")?,
            unflamable: req("unflamable")?,
            DoorWallW: req("DoorWallW")?,
            DoorWallN: req("DoorWallN")?,
            bed: req("bed")?,
            waterPiped: req("waterPiped")?,
            attachedS: t.flag_from_string("attachedS"),
            attachedE: t.flag_from_string("attachedE"),
            WallN: t.flag_from_string("WallN"),
            WallW: t.flag_from_string("WallW"),
            WallNW: t.flag_from_string("WallNW"),
            climbSheetTop: ["N", "S", "W", "E"]
                .map(|d| t.flag_from_string(&format!("climbSheetTop{d}"))),
            climbSheet: ["N", "S", "W", "E"].map(|d| t.flag_from_string(&format!("climbSheet{d}"))),
        })
    }
}

/// Sandbox and world state fire reads (runtime input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FireSettings {
    /// `SandboxOptions.instance.fireSpread`.
    pub fire_spread: bool,
    /// `RainManager.isRaining()`: a spreading fire stops spreading.
    pub raining: bool,
    /// `TutorialManager.instance.active`.
    pub tutorial: bool,
}

impl Default for FireSettings {
    fn default() -> Self {
        FireSettings {
            fire_spread: true,
            raining: false,
            tutorial: false,
        }
    }
}

/// One frame's timing (`GameTime`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireFrame {
    /// `getMultipliedSecondsSinceLastUpdate()`.
    pub multiplied_seconds: f32,
    /// `getThirtyFPSMultiplier()`.
    pub thirty_fps: f32,
    /// `getCalender().getTimeInMillis()`.
    pub now_millis: i64,
}

/// Where the fire's effects leave the world model. Every method defaults
/// to doing nothing.
#[allow(unused_variables)]
pub trait FireHooks: FireSoundHooks {
    /// `WorldSoundManager.addSound(fire, x, y, z, radius, volume)` (a new
    /// fire, one time in five: zombies hear it).
    fn world_sound(&mut self, x: i32, y: i32, z: i32, radius: i32, volume: i32) {}
    /// `SoundManager.PlayWorldSound(name, square, 0, radius, maxGain,
    /// false)` (`"BurnedObjectExploded"`).
    fn play_world_sound(&mut self, name: &str, x: i32, y: i32, z: i32, radius: f32, gain: f32) {}
    /// After a fire's `update()` (or the creation of a new one): its
    /// `lightSource` may have been added (`addLamppost`), recoloured
    /// (stage 5) or replaced (`setLightRadius`), and a permanent fire's
    /// `heatSource` added or resized ([`IsoFire::heat_source`]). The
    /// renderer's `LightLink::sync_fire` belongs here.
    fn fire_updated(&mut self, r: FireRef, fire: &IsoFire) {}
    /// `IsoFire.removeFromWorld()`: the light (`removeLamppost`) and heat
    /// source (`removeHeatSource`) go (`LightLink::remove`).
    fn fire_removed(&mut self, r: FireRef) {}
    /// `FMODParameterUtils.getClosestListenerDistanceSquared(x, y, z)`
    /// (the fire sound slots go to the nearest fires).
    fn listener_distance_sq(&self, x: f32, y: f32, z: i32) -> f32 {
        0.0
    }
    /// `getAlpha(IsoCamera.frameState.playerIndex)` of the fire (the
    /// renderer's stage-5 fade).
    fn fire_alpha(&self, x: i32, y: i32, z: i32) -> f32 {
        1.0
    }
    /// `LuaEventManager.triggerEvent(name, square or fire)`: `OnNewFire`
    /// and `OnGridBurnt` (the mutations' own events go through
    /// [`SquareMutationHooks::lua_event`]).
    fn lua_event(&mut self, name: &str, x: i32, y: i32, z: i32) {}
    /// `IsoGenerator.setSurroundingElectricity()` after a burning generator
    /// was switched off.
    fn generator_switched_off(&mut self, x: i32, y: i32, z: i32) {}
    /// `IsoHutch.removeHutchObject(hutch)`'s `releaseAllAnimals()` and
    /// `dropAllEggs()` of the hutch object `r`, before it leaves the world.
    fn hutch_release_contents(&mut self, r: ObjectRef) {}
    /// The name of `WorldDictionary` object-name id `id`
    /// (`ObjectName::Dict`).
    fn dict_object_name(&self, id: i8) -> Option<String> {
        None
    }
}

/// [`FireHooks`] that do nothing.
pub struct NoFireHooks;

impl FireSoundHooks for NoFireHooks {}
impl FireHooks for NoFireHooks {}

/// The [`MutationEnv`] parts besides the cell, meta grid and sprites.
pub struct FireMutation<'a> {
    pub regions: &'a mut IsoRegions,
    pub doors: &'a DoorTables,
    pub world: &'a mut dyn SquareMutationHooks,
    pub pathfind: &'a mut dyn PathfindSquareHooks,
    pub render: &'a mut dyn RenderSquareHooks,
    /// `IsoPlayer.numPlayers`.
    pub num_players: i32,
}

/// The world a fire burns in.
pub struct FireWorld<'a> {
    pub cell: &'a mut IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a mut IsoSpriteManager,
    pub mutation: FireMutation<'a>,
    pub flags: FireFlags,
    pub settings: FireSettings,
    pub hooks: &'a mut dyn FireHooks,
}

impl FireWorld<'_> {
    /// The square mutations over this world, drawing from `rand`.
    pub fn env<'s>(&'s mut self, rand: &'s mut dyn GameRand) -> MutationEnv<'s> {
        let m = &mut self.mutation;
        MutationEnv {
            cell: &mut *self.cell,
            meta: self.meta,
            sprites: &mut *self.sprites,
            regions: &mut *m.regions,
            rand,
            doors: m.doors,
            world: &mut *m.world,
            pathfind: &mut *m.pathfind,
            render: &mut *m.render,
            num_players: m.num_players,
        }
    }

    /// `IsoFire.isCampfire()` of the fire at `r`: an object after the
    /// first on its square, not a world item, named `Campfire`.
    pub fn is_campfire(&self, r: FireRef) -> bool {
        let Some(sq_owner) = self.sq(r.x, r.y, r.z) else {
            return false;
        };
        let sq = sq_owner.borrow();
        sq.objects.iter().map(ObjectSlot::borrow).skip(1).any(|o| {
            !matches!(o.kind, ObjectKind::WorldInventory(_))
                && self
                    .object_name(&o)
                    .is_some_and(|n| n.eq_ignore_ascii_case("Campfire"))
        })
    }

    /// `getObjectName`-free `IsoObject.getName()`.
    pub fn object_name(&self, o: &IsoObject) -> Option<String> {
        match &o.name {
            None => None,
            Some(ObjectName::Grass) => Some("Grass".into()),
            Some(ObjectName::Str(s)) => Some(s.clone()),
            Some(ObjectName::Dict(i)) => self.hooks.dict_object_name(*i),
        }
    }

    /// `IsoSpriteManager.getSprite(name)`: the named sprite, added (with
    /// no `name`, as `AddSprite` leaves it) when unknown.
    pub fn sprite_named(&mut self, name: &str) -> usize {
        match self.sprites.named_map.get(name) {
            Some(&s) => s,
            None => self.sprites.add_sprite_named(name),
        }
    }

    pub(crate) fn sq(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.cell.get_grid_square(x, y, z)
    }

    pub(crate) fn sq_mut(&mut self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.cell.get_grid_square_mut(x, y, z)
    }

    /// `getFloor().getSprite().firerequirement`.
    pub fn floor_fire_requirement(&self, sq: &IsoGridSquare) -> Option<i32> {
        let i = sq.floor_index(self.sprites)?;
        let s = sq.objects[i].borrow().sprite?;
        Some(self.sprites.sprites[s].firerequirement)
    }

    /// `IsoFire.CanAddFire(square, canBurnAnywhere, smoke)` in single
    /// player.
    pub fn can_add_fire(&self, at: Option<(i32, i32, i32)>, can_burn_anywhere: bool) -> bool {
        let Some(sq) = at.and_then(|(x, y, z)| self.sq(x, y, z)) else {
            return false;
        };
        let f = &self.sprites.tables.flag;
        if sq.borrow().objects.is_empty() || sq.borrow().has(f.water) {
            return false;
        }
        if !can_burn_anywhere && sq.borrow().has(f.burntOut) {
            return false;
        }
        if sq.borrow().has(self.flags.burning) || sq.borrow().has(self.flags.smoke) {
            return false;
        }
        if !can_burn_anywhere && sq.borrow().has(self.flags.unflamable) {
            return false;
        }
        true
    }

    /// `IsoFire.getSquaresEnergyRequirement(square)`.
    pub fn squares_energy_requirement(&self, sq: &IsoGridSquare) -> i32 {
        let f = &self.sprites.tables.flag;
        let mut v = 30;
        if sq.has(f.vegitation) {
            v = 15;
        }
        if !sq.has(f.exterior) {
            v = 40;
        }
        if let Some(req) = self.floor_fire_requirement(sq) {
            v = req;
        }
        if self.settings.tutorial {
            v / 4
        } else {
            v
        }
    }
}

/// A fire in `FireStack`: its square and object identity there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FireRef {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: ObjectId,
}

/// A character's identity for the burning-character lists.
pub type CharacterId = u64;

/// `IsoFireManager`'s state.
#[derive(Clone, Debug, Default)]
pub struct IsoFireManager {
    /// `FireStack`, in order.
    pub fire_stack: Vec<FireRef>,
    /// `CharactersOnFire_Stack` (`AddBurningCharacter`).
    pub burning_characters: Vec<CharacterId>,
    /// `charactersOnFire` (`setOnFire`), whose `SpreadFireMP` only
    /// spreads on a server.
    pub characters_on_fire: Vec<CharacterId>,
    pub oscillators: FireOscillators,
    /// `fireRecalc` (counts down from `fireRecalcDelay`, 25).
    pub fire_recalc: i32,
    /// `fireSounds`.
    pub sounds: FireSounds,
}

/// `IsoFireManager.fireRecalcDelay`.
pub const FIRE_RECALC_DELAY: i32 = 25;

impl IsoFireManager {
    pub fn new() -> Self {
        IsoFireManager {
            fire_recalc: FIRE_RECALC_DELAY,
            ..Default::default()
        }
    }

    /// `Reset()`.
    pub fn reset(&mut self, hooks: &mut dyn FireHooks) {
        self.fire_stack.clear();
        self.burning_characters.clear();
        self.sounds.reset(hooks);
    }

    fn object(cell: &IsoCell, r: FireRef) -> Option<IsoObjectRef> {
        let square_owner = cell.get_grid_square(r.x, r.y, r.z)?;
        let square = square_owner.borrow();
        Some(square.objects[square.object_index(r.id)?].ref_handle())
    }

    /// The actual whole-object owner, when the attached object is a fire.
    pub fn fire(&self, cell: &IsoCell, r: FireRef) -> Option<IsoObjectRef> {
        let object = Self::object(cell, r)?;
        let is_fire = matches!(&object.borrow().kind, ObjectKind::Fire(_));
        is_fire.then_some(object)
    }
    pub fn fire_mut(cell: &mut IsoCell, r: FireRef) -> Option<IsoObjectRef> {
        Self::default().fire(cell, r)
    }

    /// `AddBurningCharacter(c)`.
    pub fn add_burning_character(&mut self, c: CharacterId) {
        if !self.burning_characters.contains(&c) {
            self.burning_characters.push(c);
        }
    }

    /// `RemoveBurningCharacter(c)`.
    pub fn remove_burning_character(&mut self, c: CharacterId) {
        if let Some(i) = self.burning_characters.iter().position(|&x| x == c) {
            self.burning_characters.remove(i);
        }
    }

    /// `addCharacterOnFire` / `deleteCharacterOnFire` (only a server's
    /// `setOnFire` calls them).
    pub fn set_character_on_fire(&mut self, c: CharacterId, on: bool) {
        let at = self.characters_on_fire.iter().position(|&x| x == c);
        match (on, at) {
            (true, None) => self.characters_on_fire.push(c),
            (false, Some(i)) => {
                self.characters_on_fire.remove(i);
            }
            _ => {}
        }
    }

    /// `IsoFire.removeFromWorld()`'s own part: out of `FireStack`, its
    /// sound slot, light and heat source gone (`IsoObject.removeFromWorld`
    /// follows).
    fn fire_remove_from_world(&mut self, w: &mut FireWorld<'_>, r: FireRef, fire: &mut IsoFire) {
        if !fire.perm {
            self.remove(r);
        }
        self.sounds.remove_fire(r);
        fire.light = None;
        fire.heat_source = None;
        w.hooks.fire_removed(r);
    }

    /// `Remove(fire)`.
    pub fn remove(&mut self, r: FireRef) {
        if let Some(i) = self.fire_stack.iter().position(|&f| f == r) {
            self.fire_stack.remove(i);
        }
    }

    /// `Add(fire)`: at `MAX_FIRE_OBJECTS` fires the oldest one (largest
    /// `age` above 0) is taken off its square first.
    pub fn add(&mut self, w: &mut FireWorld<'_>, rand: &mut dyn GameRand, r: FireRef) {
        if self.fire_stack.contains(&r) {
            return;
        }
        if self.fire_stack.len() >= MAX_FIRE_OBJECTS {
            let mut oldest = None;
            let mut oldest_age = 0;
            for &f in &self.fire_stack {
                let age = self.fire(w.cell, f).map_or(0, |f| fire_body(&f).age);
                if age > oldest_age {
                    oldest_age = age;
                    oldest = Some(f);
                }
            }
            if let Some(o) = oldest {
                self.evict(w, rand, o);
            }
        }
        self.fire_stack.push(r);
    }

    /// `Add`'s removal of the oldest fire: the square's flags,
    /// `RemoveAttachedAnims`, `removeFromWorld`, `removeFromSquare`.
    fn evict(&mut self, w: &mut FireWorld<'_>, rand: &mut dyn GameRand, o: FireRef) {
        let flags = w.flags;
        let Some(sq_owner) = w.sq_mut(o.x, o.y, o.z) else {
            // No square: removeFromWorld is not reached.
            return;
        };
        let mut sq = sq_owner.borrow_mut();
        sq.properties.unset_flag(flags.burning);
        sq.properties.unset_flag(flags.smoke);
        drop(sq);
        let object = Self::object(w.cell, o);
        let fire = self.fire(w.cell, o).map(|f| fire_body(&f).clone());
        let (Some(object), Some(mut fire)) = (object, fire) else {
            // Preserve missing-fire cleanup in the existing locator registry.
            // A full retained FireStack owner is a separate migration.
            self.remove(o);
            self.sounds.remove_fire(o);
            w.hooks.fire_removed(o);
            return;
        };
        fire.anims.clear();
        self.fire_remove_from_world(w, o, &mut fire);
        Self::sync(&object, &fire);
        let or = ObjectRef {
            pos: (o.x, o.y, o.z),
            id: o.id,
        };
        w.env(rand).base_remove_from_world_detached(or, &object);
        if let Some(sq) = w.sq_mut(o.x, o.y, o.z) {
            let index = {
                sq.borrow()
                    .objects
                    .iter()
                    .position(|o| o.ptr_eq_ref(&object))
            };
            if let Some(i) = index {
                sq.borrow_mut().remove_object_slot(i);
            }
            sq.borrow_mut().special_objects.retain(|&id| id != o.id);
        }
    }

    /// Puts `fire` on its square as a new object (a fire loaded with the
    /// square), returning its reference; [`IsoFireManager::add_loaded`]
    /// registers it.
    pub fn place(w: &mut FireWorld<'_>, x: i32, y: i32, z: i32, fire: IsoFire) -> Option<FireRef> {
        let sq_owner = w.sq_mut(x, y, z)?;
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        sq.objects
            .push((IsoObject::new(id, ObjectKind::Fire(fire), None)).into());
        Some(FireRef { x, y, z, id })
    }

    /// `StartFire(cell, square, igniteOnAny, energy, life)`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_fire(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        x: i32,
        y: i32,
        z: i32,
        ignite_on_any: bool,
        mut energy: i32,
        life: i32,
    ) -> Option<FireRef> {
        if energy <= 0 {
            return None;
        }
        let sq_owner = w.sq(x, y, z)?;
        let sq = sq_owner.borrow();
        if let Some(req) = w.floor_fire_requirement(&sq) {
            energy -= req;
        }
        drop(sq);
        if energy < 5 {
            energy = 5;
        }
        if !w.can_add_fire(Some((x, y, z)), ignite_on_any) {
            return None;
        }
        let sq_owner = w.sq(x, y, z)?;
        let sq = sq_owner.borrow();
        let fs = FireSquare {
            vegitation: sq.has(w.sprites.tables.flag.vegitation),
            floor_fire_requirement: w.floor_fire_requirement(&sq),
            tutorial: w.settings.tutorial,
        };
        drop(sq);
        let fire = IsoFire::new_fire(fs, energy, life, rand);
        let burning = w.flags.burning;
        w.sq_mut(x, y, z)?.borrow_mut().properties.set_flag(burning);
        w.hooks.lua_event("OnNewFire", x, y, z);
        // Add before the object joins the square (an eviction cannot pick
        // the new fire: it is not in FireStack yet).
        let id = w.sq_mut(x, y, z)?.borrow_mut().new_object_id();
        let r = FireRef { x, y, z, id };
        self.add(w, rand, r);
        let sq_owner = w.sq_mut(x, y, z)?;
        let mut sq = sq_owner.borrow_mut();
        sq.objects
            .push((IsoObject::new(id, ObjectKind::Fire(fire), None)).into());
        drop(sq);
        if rand.next(5) == 0 {
            w.hooks.world_sound(x, y, z, 20, 20);
        }
        w.env(rand)
            .invalidate_render((x, y, z), dirty::OBJECT_ADDED);
        Some(r)
    }

    /// `StartFire(cell, square, igniteOnAny, energy)`.
    pub fn start_fire4(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        at: (i32, i32, i32),
        ignite_on_any: bool,
        energy: i32,
    ) -> Option<FireRef> {
        self.start_fire(w, rand, at.0, at.1, at.2, ignite_on_any, energy, 0)
    }

    /// `StartSmoke(cell, square, igniteOnAny, energy, life)`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_smoke(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        x: i32,
        y: i32,
        z: i32,
        ignite_on_any: bool,
        energy: i32,
        life: i32,
    ) -> Option<FireRef> {
        // CanAddSmoke: CanAddFire with smoke set (the same checks in single
        // player).
        if !w.can_add_fire(Some((x, y, z)), ignite_on_any) {
            return None;
        }
        let smoke = w.flags.smoke;
        w.sq_mut(x, y, z)?.borrow_mut().properties.set_flag(smoke);
        let fire = IsoFire::new_smoke(energy, life, true, rand);
        let id = w.sq_mut(x, y, z)?.borrow_mut().new_object_id();
        let r = FireRef { x, y, z, id };
        self.add(w, rand, r);
        w.sq_mut(x, y, z)?
            .borrow_mut()
            .objects
            .push((IsoObject::new(id, ObjectKind::Fire(fire), None)).into());
        w.env(rand)
            .invalidate_render((x, y, z), dirty::OBJECT_ADDED);
        Some(r)
    }

    /// Registers a fire loaded with its square (`IsoFire.addToWorld` for a
    /// non-permanent fire) and sets the square flag `IsoFire.load` sets.
    pub fn add_loaded(&mut self, w: &mut FireWorld<'_>, rand: &mut dyn GameRand, r: FireRef) {
        let Some(f) = self.fire(w.cell, r) else {
            return;
        };
        let (perm, burning) = {
            let f = fire_body(&f);
            (f.perm, f.burning())
        };
        drop(f);
        let flag = if burning {
            w.flags.burning
        } else {
            w.flags.smoke
        };
        if let Some(sq_owner) = w.sq_mut(r.x, r.y, r.z) {
            let mut sq = sq_owner.borrow_mut();
            sq.properties.set_flag(flag);
        }
        if !perm {
            self.add(w, rand, r);
        }
    }

    /// `explode(cell, square, power)`.
    pub fn explode(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        (gx, gy, gz): (i32, i32, i32),
        power: i32,
    ) -> Result<(), String> {
        if w.sq(gx, gy, gz).is_none() {
            return Ok(());
        }
        self.fire_recalc = 1;
        for x in -2..=2 {
            for y in -2..=2 {
                for z in 0..=1 {
                    let at = (gx + x, gy + y, gz + z);
                    if w.sq(at.0, at.1, at.2).is_none()
                        || rand.next(100) >= power
                        || !w.can_add_fire(Some(at), true)
                    {
                        continue;
                    }
                    let energy = rand.next_range(100, 250 + power);
                    self.start_fire(w, rand, at.0, at.1, at.2, true, energy, 0);
                    super::burn::burn_walls(self, w, rand, at, true, true)?;
                }
            }
        }
        Ok(())
    }

    /// `RemoveAllOn(square)`: puts out every fire there (last first).
    pub fn remove_all_on(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        (x, y, z): (i32, i32, i32),
    ) {
        for n in (0..self.fire_stack.len()).rev() {
            let Some(&r) = self.fire_stack.get(n) else {
                continue;
            };
            if (r.x, r.y, r.z) != (x, y, z) {
                continue;
            }
            let Some(mut f) = self.fire(w.cell, r).map(|f| fire_body(&f).clone()) else {
                // Not on its square any more: extinctFire only drops it.
                self.remove(r);
                continue;
            };
            if let Some(object) = Self::object(w.cell, r) {
                self.extinct(w, rand, r, &object, &mut f);
            }
        }
    }

    /// `IsoGridSquare.stopFire()`.
    pub fn stop_fire(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        at: (i32, i32, i32),
    ) {
        self.remove_all_on(w, rand, at);
        let (burnt, burning) = (w.sprites.tables.flag.burntOut, w.flags.burning);
        if let Some(sq_owner) = w.sq_mut(at.0, at.1, at.2) {
            let mut sq = sq_owner.borrow_mut();
            sq.properties.set_flag(burnt);
            sq.properties.unset_flag(burning);
            sq.burnt_out = true;
        }
    }

    /// `extinctFire()`: the square's flags, `RemoveAttachedAnims`, out of
    /// the objects, `RemoveTileObject` (which finds nothing left to remove
    /// but still tells collision, Lua, pathfinding and regions),
    /// `setLife(0)`, `removeFromWorld()`.
    fn extinct(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        r: FireRef,
        object: &IsoObjectRef,
        fire: &mut IsoFire,
    ) {
        let flags = w.flags;
        if let Some(sq_owner) = w.sq_mut(r.x, r.y, r.z) {
            let mut sq = sq_owner.borrow_mut();
            sq.properties.unset_flag(flags.burning);
            sq.properties.unset_flag(flags.smoke);
        }
        fire.anims.clear();
        let mut obj = None;
        if let Some(sq) = w.sq_mut(r.x, r.y, r.z) {
            let index = {
                sq.borrow()
                    .objects
                    .iter()
                    .position(|o| o.ptr_eq_ref(object))
            };
            if let Some(i) = index {
                obj = Some(sq.borrow_mut().remove_object_slot(i));
            }
        }
        // A fire is never multi-square, so either branch of
        // RemoveTileObject(obj) ends in RemoveTileObject(obj, false).
        w.env(rand)
            .remove_tile_object_receiver((r.x, r.y, r.z), r.id, Some(object.clone()));
        fire.life = 0;
        self.fire_remove_from_world(w, r, fire);
        let _removed = obj;
        object.borrow_mut().kind = ObjectKind::Fire(fire.clone());
        let or = ObjectRef {
            pos: (r.x, r.y, r.z),
            id: r.id,
        };
        w.env(rand).base_remove_from_world_detached(or, &object);
    }

    /// `IsoFire.Spread()`.
    fn spread(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        r: FireRef,
        object: &IsoObjectRef,
        fire: &mut IsoFire,
    ) {
        if !w.settings.fire_spread || fire.life_stage >= 4 {
            return;
        }
        let mut num = rand.next(3) + 1;
        if rand.next(50) == 0 {
            num += 15;
        }
        if w.settings.tutorial {
            num += 15;
        }
        let (x, y, z) = (r.x, r.y, r.z);
        for _ in 0..num {
            let at = match rand.next(13) {
                0 => (x, y - 1, z),
                1 => (x + 1, y - 1, z),
                2 => (x + 1, y, z),
                3 => (x + 1, y + 1, z),
                4 => (x, y + 1, z),
                5 => (x - 1, y + 1, z),
                6 => (x - 1, y, z),
                7 => (x - 1, y - 1, z),
                8 => (x - 1, y - 1, z - 1),
                9 => (x - 1, y, z - 1),
                10 => (x, y - 1, z - 1),
                11 => (x, y, z - 1),
                _ => (x, y, z + 1),
            };
            if !w.can_add_fire(Some(at), false) {
                continue;
            }
            let Some(sq_owner) = w.sq(at.0, at.1, at.2) else {
                continue;
            };
            let sq = sq_owner.borrow();
            let req = w.squares_energy_requirement(&sq);
            if fire.energy < req {
                continue;
            }
            fire.energy -= req;
            if w.settings.raining {
                return;
            }
            let exterior = sq.has(w.sprites.tables.flag.exterior);
            drop(sq);
            let energy = if exterior { fire.energy } else { req * 2 };
            // The new fire may evict this one: keep its age current.
            Self::sync(object, fire);
            self.start_fire(w, rand, at.0, at.1, at.2, false, energy, 0);
        }
    }

    /// Commit the existing stepped simulation body to its pinned receiver,
    /// never to a replacement occupying its old local ID.
    fn sync(object: &IsoObjectRef, fire: &IsoFire) {
        if let ObjectKind::Fire(f) = &mut object.borrow_mut().kind {
            f.clone_from(fire);
        }
    }

    /// `IsoFire.update()` of one fire in `FireStack`. On `Err` the fire
    /// keeps the state it had when the original threw (it was written
    /// back before the burn).
    fn update_fire(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        frame: FireFrame,
        r: FireRef,
    ) -> Result<(), String> {
        let Some(object) = Self::object(w.cell, r) else {
            return Ok(());
        };
        let Some(mut fire) = self.fire(w.cell, r).map(|f| fire_body(&f).clone()) else {
            return Ok(());
        };
        self.sounds.add_fire(r);
        let flag = if fire.life_stage < 4 {
            w.flags.burning
        } else {
            w.flags.smoke
        };
        if let Some(sq_owner) = w.sq_mut(r.x, r.y, r.z) {
            let mut sq = sq_owner.borrow_mut();
            sq.properties.set_flag(flag);
        }
        if !fire.smoke && fire.life_stage < 5 {
            super::burn::burn_tick(w, rand, (r.x, r.y, r.z), frame.multiplied_seconds);
        }
        let dt = frame.multiplied_seconds * 60.0;
        let alpha = w.hooks.fire_alpha(r.x, r.y, r.z);
        let mut step = Step {
            mgr: self,
            w,
            r,
            object,
            gone: false,
        };
        let res = fire.update_with(
            dt,
            frame.thirty_fps,
            frame.now_millis,
            alpha,
            rand,
            &mut step,
        );
        if res.is_err() {
            let f = match &step.object.borrow().kind {
                ObjectKind::Fire(f) => Some(f.clone()),
                _ => None,
            };
            if let Some(f) = f {
                step.w.hooks.fire_updated(r, &f);
            }
            return res;
        }
        if !step.gone
            && step.w.sq(r.x, r.y, r.z).is_some_and(|sq| {
                sq.borrow()
                    .objects
                    .iter()
                    .any(|o| o.ptr_eq_ref(&step.object))
            })
        {
            Self::sync(&step.object, &fire);
            step.w.hooks.fire_updated(r, &fire);
        }
        Ok(())
    }

    /// `IsoFireManager.Update()`: every fire in `FireStack` (as it was when
    /// the frame began, skipping fires removed meanwhile) updates once.
    ///
    /// `Err` is an exception a fire's update threw (its `toString()`
    /// text): `Update` stops there, without the later fires, `fireRecalc`
    /// or the sounds, and nothing on the way to `IngameState.update`
    /// catches it, so the engine skips the rest of that frame's
    /// `IngameState.update` and logs it with `ExceptionLogger`.
    pub fn update(
        &mut self,
        w: &mut FireWorld<'_>,
        rand: &mut dyn GameRand,
        frame: FireFrame,
    ) -> Result<(), String> {
        self.oscillators.update();
        let stack = self.fire_stack.clone();
        for r in stack {
            if self.fire(w.cell, r).is_none() || !self.fire_stack.contains(&r) {
                continue;
            }
            self.update_fire(w, rand, frame, r)?;
        }
        self.fire_recalc -= 1;
        if self.fire_recalc < 0 {
            self.fire_recalc = FIRE_RECALC_DELAY;
        }
        self.update_sounds(w);
        Ok(())
    }

    /// `fireSounds.update()`.
    pub fn update_sounds(&mut self, w: &mut FireWorld<'_>) {
        let mut sounds = std::mem::take(&mut self.sounds);
        let info = |r: FireRef| {
            let f = self.fire(w.cell, r);
            FireSoundInfo {
                on_square: f.is_some(),
                smoke: f.as_ref().is_some_and(|f| fire_body(f).smoke),
                life_stage: f.map_or(0, |f| fire_body(&f).life_stage),
                campfire: w.is_campfire(r),
                listener_distance_sq: w.hooks.listener_distance_sq(
                    r.x as f32 + 0.5,
                    r.y as f32 + 0.5,
                    r.z,
                ),
            }
        };
        let infos: Vec<(FireRef, FireSoundInfo)> =
            sounds.fires.iter().map(|&r| (r, info(r))).collect();
        sounds.update(&mut *w.hooks, |r| {
            infos
                .iter()
                .find(|(f, _)| *f == r)
                .map(|(_, i)| *i)
                .unwrap()
        });
        self.sounds = sounds;
    }
}

/// The callbacks of one fire's update.
struct Step<'m, 'w, 'a> {
    mgr: &'m mut IsoFireManager,
    w: &'w mut FireWorld<'a>,
    r: FireRef,
    object: IsoObjectRef,
    /// `extinctFire()` ran: the fire is off its square.
    gone: bool,
}

impl FireCallbacks for Step<'_, '_, '_> {
    fn on_event(
        &mut self,
        fire: &mut IsoFire,
        event: FireEvent,
        rand: &mut dyn GameRand,
    ) -> Result<(), String> {
        let r = self.r;
        match event {
            FireEvent::Burn => {
                IsoFireManager::sync(&self.object, fire);
                super::burn::burn(self.mgr, self.w, rand, (r.x, r.y, r.z))?;
            }
            FireEvent::Spread => self.mgr.spread(self.w, rand, r, &self.object, fire),
            FireEvent::Extinct => {
                self.mgr.extinct(self.w, rand, r, &self.object, fire);
                self.gone = true;
            }
            // The new light is read after the update (FireHooks::fire_updated).
            FireEvent::LightChanged => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod owner_tests;

pub fn fire_body(object: &IsoObjectRef) -> Ref<'_, IsoFire> {
    Ref::map(object.borrow(), |o| match &o.kind {
        ObjectKind::Fire(f) => f,
        _ => panic!("object is not a fire"),
    })
}
pub fn fire_body_mut(object: &IsoObjectRef) -> RefMut<'_, IsoFire> {
    RefMut::map(object.borrow_mut(), |o| match &mut o.kind {
        ObjectKind::Fire(f) => f,
        _ => panic!("object is not a fire"),
    })
}
