//! `IsoGridSquare`'s object mutations and the recalculation they trigger:
//! `AddTileObject`, `AddSpecialObject` / `AddSpecialTileObject`,
//! `RemoveTileObject` (with `IsoObjectUtils.safelyRemoveTileObjectFromSquare`
//! for multi-square objects), `RemoveTileObjectErosionNoRecalc`,
//! `DeleteTileObject`, the single-player paths of
//! `transmitAddObjectToSquare` / `transmitRemoveItemFromSquare`, `addFloor`,
//! `addCorpse` / `removeCorpse`, `EnsureSurroundNotNull`,
//! `RecalcAllWithNeighbours` (the original's version, which recalculates
//! the neighbours' properties too), `IsoCell.checkHaveRoof`,
//! `restackSheetRope` and `fixPlacedItemRenderOffsets`.
//!
//! Everything the original calls outside the square goes through hooks, at
//! the original's call points and in its order:
//!
//! * [`IsoRegions`] (`setPreviousFlags` / `squareChanged`) is called
//!   directly;
//! * [`PathfindSquareHooks`]: `PathfindNative` / `PolygonalMap2` square and
//!   chunk updates and `MapCollisionData.squareChanged` (the pathfinding
//!   and population threads);
//! * [`RenderSquareHooks`]: render-chunk and vispoly invalidation,
//!   cutaways, `LosUtil.cachecleared`, `setRecalcLightTime`,
//!   `Core.dirtyGlobalLightsCount`, `GameTime.lightSourceUpdate`,
//!   `IsoGridOcclusionData.SquareChanged` and the per-`RecalcProperties`
//!   lighting/puddle flags;
//! * [`SquareMutationHooks`]: Lua events, the registries objects join and
//!   leave in `addToWorld` / `removeFromWorld` ([`WorldCall`]), the queries
//!   those make (power, fluids, piped fuel, entity sprite configs) and the
//!   other world systems (`FMODAmbientWalls`, `DesignationZoneAnimal`,
//!   corpses).
//!
//! `Rand` is drawn through [`GameRand`] where the original draws it
//! (`IsoObject.addToWorld`'s ambient emitters, and whatever the hooks draw).
//!
//! Single player only: the server and client branches (packets,
//! `GameServer.RemoveItemFromMap`, `transmitCompleteItemToClients`) are
//! not ported. `OnObjectAdded` is only fired by the client's
//! `AddItemToMapPacket`, so no add path here fires a Lua event.
//!
//! Not modelled: the square's cached navigation pointers
//! (`doGridNav`, `setBlockedGridPointers`; neighbours are cell lookups
//! here), `solidFloorCached` / `cachedIsFree`, `IsoObject.sy` and
//! `renderSquareOverride` (screen caches), and the object pools
//! (`CellLoader.isoObjectCache`).

use crate::chunk::{IsoCell, IsoGridSquare, MAX_LEVEL, MIN_LEVEL};
use crate::isoregion::IsoRegions;
use crate::java::GameRand;
use crate::meta_grid::IsoMetaGrid;
use crate::object_slot::{IsoObjectRef, ObjectSlot};
use crate::objects::{IsoObject, ObjectId, ObjectKind};
use crate::save::visual::IsoDeadBody;
use crate::sprite::{IsoSpriteManager, SpriteIdx};
use crate::square::{set_matrix_bit, IsoDirections};
use crate::SquareRef;
use pz_inventory::InventoryItem;

mod lifecycle;
mod world_item;
pub use lifecycle::{AmbientEmitterKind, WorldCall};
pub use world_item::{render_square, FloorDropEnv, WorldItemPlaced, WorldItemPlacementError, WORLD_ITEM_ADDED};

#[cfg(test)]
mod tests;

/// A square position `(x, y, z)`.
pub type SquarePos = (i32, i32, i32);

/// An object on a square: its square and its identity there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub pos: SquarePos,
    pub id: ObjectId,
}

/// `IsoChunk.invalidateRenderChunkLevel` flags the mutations pass.
pub mod dirty {
    /// `AddTileObject`, `AddSpecialObject`, `addFloor`.
    pub const OBJECT_ADDED: u64 = 64;
    /// `RemoveTileObject` (not a world item).
    pub const OBJECT_REMOVED: u64 = 128;
    /// `RemoveTileObject` of an `IsoWorldInventoryObject`.
    pub const WORLD_ITEM_REMOVED: u64 = 136;
    /// `addCorpse`.
    pub const CORPSE_ADDED: u64 = 66;
    /// `removeCorpse`.
    pub const CORPSE_REMOVED: u64 = 130;
}

/// What the pathfinding side is told about square changes.
pub trait PathfindSquareHooks {
    /// `PathfindNative.useNativeCode` (the debug option
    /// `pathfindUseNativeCode`, on by default): decides the order of
    /// [`PathfindSquareHooks::square_changed`] against the render and sound
    /// notifications `PolygonalMap2.squareChanged` makes.
    fn uses_native_code(&self) -> bool {
        true
    }

    /// `PathfindNative.squareChanged(sq)` (or `PolygonalMap2`'s own task
    /// queueing when not native) for the square at `sq`: an update task for
    /// each existing neighbour (`getAdjacentSquare(dir)` in `IsoDirections`
    /// order, see [`adjacent_squares`]) and then for the square, each built
    /// from `cell` as it is at the call (`getBits` reads the neighbours, the
    /// square above and its doors and windows). The native side skips
    /// chunks it has not been given (`loadedBits & 2`).
    fn square_changed(&mut self, _cell: &IsoCell, _sq: SquarePos) {}

    /// `MapCollisionData.instance.squareChanged(sq)`: the population
    /// thread's collision bits (solid, blocked N/W, water, room) of `sq`.
    fn map_collision_square_changed(&mut self, _sq: SquareRef) {}

    /// `PathfindNative.instance.addChunkToWorld(chunk)` (or
    /// `PolygonalMap2`'s): `IsoChunk.setSquare` on the game thread put a
    /// square on a level the chunk did not have. `ChunkUpdateTask` builds
    /// its buffer from chunk `(wx, wy)` of `cell` here.
    fn add_chunk_to_world(&mut self, _cell: &IsoCell, _wx: i32, _wy: i32) {}
}

/// What the render and lighting side is told about square changes.
pub trait RenderSquareHooks {
    /// `PerformanceSettings.fboRenderChunk`.
    fn fbo_render_chunk(&self) -> bool {
        false
    }

    /// `IsoChunk.invalidateRenderChunkLevel(level, flags)` of chunk
    /// `(wx, wy)` (it does nothing unless `fboRenderChunk`, and on a
    /// server).
    fn invalidate_render_chunk_level(&mut self, _wx: i32, _wy: i32, _level: i32, _flags: u64) {}

    /// `IsoChunk.invalidateVispolyChunkLevel(level)`.
    fn invalidate_vispoly_chunk_level(&mut self, _wx: i32, _wy: i32, _level: i32) {}

    /// `FBORenderCutaways.squareChanged(sq)`: every local player's
    /// `checkSquare` is dropped (`None` is `squareChanged(null)`).
    fn cutaways_square_changed(&mut self, _sq: Option<SquarePos>) {}

    /// `LosUtil.cachecleared[player] = true`.
    fn los_cache_cleared(&mut self, _player: i32) {}

    /// `IsoGridSquare.recalcLightTime = t` (the `fboRenderChunk` increment
    /// of `setRecalcLightTime` is made through
    /// [`RenderSquareHooks::dirty_global_lights`]).
    fn set_recalc_light_time(&mut self, _t: f32) {}

    /// `++Core.dirtyGlobalLightsCount`.
    fn dirty_global_lights(&mut self) {}

    /// `GameTime.instance.lightSourceUpdate = 100.0f`.
    fn light_source_update(&mut self) {}

    /// `IsoGridOcclusionData.SquareChanged()` (`++objectEpochCount`).
    fn grid_occlusion_square_changed(&mut self) {}

    /// The tail of `IsoGridSquare.RecalcProperties()` for `sq`, after its
    /// properties were rebuilt: `getPuddles().recalc = true`,
    /// `chunk.checkLightingLater_AllPlayers_OneLevel(z)`, and `clearWater()`
    /// when the square has valid water geometry but no `water` flag any
    /// more. Called for every `RecalcProperties`, including the many
    /// `RecalcAllWithNeighbours` makes; `chunk.checkPhysicsLater(z)`
    /// ([`crate::chunk::IsoChunk::check_physics_later`]) and the chunk's
    /// collision data (`chunk.collision.clear()`, both between the lighting
    /// and water parts upstream) are done right after it.
    fn properties_recalculated(&mut self, _sq: SquareRef) {}

    /// `IsoGridSquare.clearWater()` (`addFloor`): the square's water
    /// geometry goes back to its pool.
    fn clear_water(&mut self, _sq: SquareRef) {}

    /// `LightingJNI.buildingsChanged()`: every local player's
    /// `buildingsChangedCounter`, `lightSourceUpdate = 100`, all
    /// `LosUtil.cachecleared` and `++Core.dirtyGlobalLightsCount`
    /// (`WorldRegionToMetaGrid` after player buildings changed).
    fn buildings_changed(&mut self) {}

    /// For a chunk whose squares `WorldRegionToMetaGrid` updated:
    /// `invalidateRenderChunkLevels(2112)`, `getCutawayData().invalidateAll()`
    /// and `checkLightingLater_AllPlayers_AllLevels()`.
    fn chunk_buildings_changed(&mut self, _wx: i32, _wy: i32) {}
}

/// A Lua event fired by the mutations (`LuaEventManager.triggerEvent`).
#[derive(Debug)]
pub enum LuaEvent {
    /// `OnObjectAboutToBeRemoved(obj)`: `obj` is `sq.objects[index]`. The
    /// original throws if the handler removes it.
    ObjectAboutToBeRemoved { index: usize, object: IsoObjectRef },
    /// `OnTileRemoved(obj)`, with the original receiver even when detached.
    /// `None` is an absent argument in the locator-only entry point.
    TileRemoved { object: Option<IsoObjectRef> },
    /// `OnContainerUpdate(sq)` (`removeCorpse`).
    ContainerUpdate,
}

/// The world systems outside this crate the mutations call.
pub trait SquareMutationHooks {
    /// `LuaEventManager.triggerEvent(name, args)`.
    fn lua_event(&mut self, _sq: SquareRef, _event: LuaEvent) {}

    /// One registry or system call of `addToWorld` / `removeFromWorld` /
    /// `removeFromSquare` for `obj` (see [`WorldCall`]), in the
    /// original's order. Hooks that draw `Rand` (mannequin outfits) use
    /// `rand`.
    fn world_call(
        &mut self,
        _obj: ObjectRef,
        _object: IsoObjectRef,
        _rand: &mut dyn GameRand,
        _call: WorldCall,
    ) {
    }

    /// `ItemContainer.isPowered()` of `obj`'s container `container`
    /// (`getContainerByIndex` order).
    fn container_powered(
        &mut self,
        _obj: ObjectRef,
        _object: &IsoObject,
        _container: usize,
    ) -> bool {
        false
    }

    /// `IsoObject.getFluidAmount()`.
    fn fluid_amount(&mut self, _obj: ObjectRef, _object: &IsoObject) -> f32 {
        0.0
    }

    /// `IsoObject.getPipedFuelAmount()` (mod data, sandbox and power).
    fn piped_fuel_amount(&mut self, _obj: ObjectRef, _object: &IsoObject) -> i32 {
        0
    }

    /// `getSpriteConfig()` of an entity object: `None` when it has none or
    /// `isValidMultiSquare()` is false; otherwise
    /// `getAllMultiSquareObjects`, `None` inside when that fails.
    fn sprite_config_multi_square(
        &mut self,
        _obj: ObjectRef,
        _object: &IsoObject,
    ) -> Option<Option<Vec<ObjectRef>>> {
        None
    }

    /// `FMODAmbientWalls.getInstance().squareChanged(sq)`: the level's
    /// ambient wall data is marked dirty.
    fn ambient_walls_square_changed(&mut self, _sq: SquareRef) {}

    /// `DesignationZoneAnimal.addNewRoof(x, y, z)` (`addFloor`).
    fn designation_zone_add_new_roof(&mut self, _x: i32, _y: i32, _z: i32) {}

    /// `WorldRegionToMetaGrid.updateSquares` for a square whose new
    /// `roomID` names a room (with its runtime `IsoRoom`): the room's lights
    /// are created and added to the cell's `roomLights` when it has none, an
    /// activated generator inside makes the building toxic, and light
    /// switches join `room.lightSwitches`.
    fn room_square_updated(&mut self, _sq: SquareRef, _room: crate::defs::RoomIdx) {}

    /// `IsoRoom.clear(false)` of the runtime room of `room_id`, when one
    /// exists (`WorldRegionToMetaGrid.removeRoomDef`): its lights leave the
    /// cell and the lighting.
    fn iso_room_removed(&mut self, _room_id: i64) {}

    /// `IsoDeadBody.addToWorld()` (`addCorpse`): `IsoObject.addToWorld`'s
    /// registries, `CorpseCount` / `FliesSound.corpseAdded`,
    /// `ObjectIDManager.addObject`, the animal designation zone, the
    /// reanimation updater and the `deathTime` clamp to the world age.
    fn corpse_added_to_world(&mut self, _square: SquareRef, _index: usize) {}

    /// `IsoDeadBody.removeFromWorld()` (`removeCorpse`): the reverse of
    /// [`SquareMutationHooks::corpse_added_to_world`].
    fn corpse_removed_from_world(&mut self, _pos: SquarePos, _body: &mut IsoDeadBody) {}

    // ---- `AddWorldInventoryItem` ----

    /// `item.loadCorpseFromByteData(null)` of a corpse item
    /// (`isHumanCorpse()` / `isAnimalCorpse()`) put down at `pos`: its
    /// dead body (the saved one, else `createDefaultDeadBody`). The
    /// position is set and the body added with `addCorpse` afterwards.
    /// `None` leaves the item unplaced.
    fn corpse_from_item(&mut self, _pos: SquarePos, _item: &InventoryItem) -> Option<IsoDeadBody> {
        None
    }

    /// `new IsoGenerator(item, cell, sq)`: the generator object (the
    /// script's `WorldObjectSprite`, `setInfoFromItem`), which is then
    /// added with `AddSpecialObject`. `None` leaves the item unplaced.
    fn generator_from_item(
        &mut self,
        _pos: SquarePos,
        _item: &InventoryItem,
    ) -> Option<ObjectSlot> {
        None
    }

    /// An `AnimalInventoryItem` put down at `pos`: `new IsoAnimal` copied
    /// from the item's animal joins the world on the square (with its
    /// `put_down` sound and idle/stressed sound timers).
    fn animal_dropped(&mut self, _pos: SquarePos, _item: &InventoryItem) {}

    /// `item.getName()` for the world object's name: the display name
    /// (the fluid's UI name, the bloody / broken / worn prefixes, a food's
    /// freshness, a moveable's or clothing's own; translated). `None`
    /// takes the item's stored `name`.
    fn item_display_name(&mut self, _item: &InventoryItem) -> Option<String> {
        None
    }

    /// `IsoCell.addToProcessItemsRemove(item)`.
    fn process_items_remove(&mut self, _item: &InventoryItem) {}

    /// `DesignationZoneAnimal.addItemOnGround(obj, sq)`: food, dung and
    /// feathers counted by the animal zone at the square.
    fn designation_zone_item_on_ground(&mut self, _obj: ObjectRef, _object: IsoObjectRef) {}
}

/// Every hook a no-op: no other systems.
pub struct NoMutationHooks;
impl PathfindSquareHooks for NoMutationHooks {}
impl RenderSquareHooks for NoMutationHooks {}
impl SquareMutationHooks for NoMutationHooks {}

/// `IsoDoor`'s double-door offset tables (`DoubleDoorNorthClosedXOffset`
/// and the seven others), read from the installed game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoorTables {
    /// `[north][open]` x offsets of the four door parts.
    pub x_offset: [[[i32; 4]; 2]; 2],
    /// `[north][open]` y offsets of the four door parts.
    pub y_offset: [[[i32; 4]; 2]; 2],
}

impl DoorTables {
    /// The arrays the static initialiser of `zombie.iso.objects.IsoDoor`
    /// stores in the `DoubleDoor{North,West}{Open,Closed}{X,Y}Offset`
    /// fields.
    pub fn from_jar(jar: &pz_classfile::Jar) -> Result<Self, String> {
        use pz_classfile::Value;
        const DOOR: &str = "zombie/iso/objects/IsoDoor";
        let read = |field: &str| -> Result<[i32; 4], String> {
            match pz_classfile::static_value(jar, DOOR, field)
                .map_err(|e| format!("{DOOR}: {e}"))?
            {
                Some(Value::Array(a)) if a.len() == 4 => {
                    let mut out = [0; 4];
                    for (o, v) in out.iter_mut().zip(&a) {
                        match v {
                            Value::Int(i) => *o = *i as i32,
                            other => return Err(format!("{DOOR}.{field}: {other:?}")),
                        }
                    }
                    Ok(out)
                }
                other => Err(format!("{DOOR}.{field}: {other:?}")),
            }
        };
        let mut t = DoorTables {
            x_offset: [[[0; 4]; 2]; 2],
            y_offset: [[[0; 4]; 2]; 2],
        };
        for (n, side) in [(0, "West"), (1, "North")] {
            for (o, state) in [(0, "Closed"), (1, "Open")] {
                t.x_offset[n][o] = read(&format!("DoubleDoor{side}{state}XOffset"))?;
                t.y_offset[n][o] = read(&format!("DoubleDoor{side}{state}YOffset"))?;
            }
        }
        Ok(t)
    }
}

/// What the mutations work on: the cell and the systems they call.
pub struct MutationEnv<'a> {
    pub cell: &'a mut IsoCell,
    /// `IsoWorld.instance.metaGrid` (`isValidSquare`, rooms, associated
    /// buildings).
    pub meta: &'a IsoMetaGrid,
    /// `IsoSpriteManager.instance`: `addFloor` sets `solidfloor` on the
    /// floor sprite itself, and adds a sprite for an unknown name.
    pub sprites: &'a mut IsoSpriteManager,
    pub regions: &'a mut IsoRegions,
    pub rand: &'a mut dyn GameRand,
    pub doors: &'a DoorTables,
    pub world: &'a mut dyn SquareMutationHooks,
    pub pathfind: &'a mut dyn PathfindSquareHooks,
    pub render: &'a mut dyn RenderSquareHooks,
    /// `IsoPlayer.numPlayers`.
    pub num_players: i32,
}

fn chunk_of(pos: SquarePos) -> (i32, i32) {
    (pos.0.div_euclid(8), pos.1.div_euclid(8))
}

impl MutationEnv<'_> {
    fn sq(&self, pos: SquarePos) -> Option<SquareRef> {
        self.cell.get_grid_square(pos.0, pos.1, pos.2)
    }

    fn sq_mut(&mut self, pos: SquarePos) -> Option<SquareRef> {
        self.cell.get_grid_square_mut(pos.0, pos.1, pos.2)
    }

    fn spr(&self, o: &IsoObject) -> Option<&crate::sprite::IsoSprite> {
        o.sprite.map(|s| &self.sprites.sprites[s])
    }

    fn flag(&self, name: &str) -> Option<crate::sprite::IsoFlagType> {
        self.sprites.tables.flag_from_string(name)
    }

    // ---- recalculation ----

    /// `RecalcPropertiesIfNeeded()`. `propertiesDirty` is set again by
    /// every `RecalcProperties` of a square in a loaded chunk, so this
    /// always recalculates.
    pub fn recalc_properties(&mut self, pos: SquarePos) {
        let Some(sq_owner) = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2) else {
            return;
        };
        let mut sq = sq_owner.borrow_mut();
        sq.recalc_properties(&*self.sprites);
        drop(sq);
        self.render.properties_recalculated(sq_owner);
        // `this.chunk.checkPhysicsLater(z)`, then
        // `this.chunk.collision.clear()`; only the hook's water part comes
        // after them upstream, and neither reads the other.
        let (wx, wy) = chunk_of(pos);
        if let Some(ch) = self.cell.chunks.get_mut(&(wx, wy)) {
            ch.check_physics_later(pos.2);
            ch.collision.clear();
        }
    }

    /// `ReCalculateCollide` / `ReCalculatePathFind` /
    /// `ReCalculateVisionBlocked` of `a` towards `b`.
    fn recalc_matrices(&mut self, a: SquarePos, b: SquarePos) {
        let (collide, path, vision) = {
            let (Some(sa), Some(sb)) = (self.sq(a), self.sq(b)) else {
                return;
            };
            let m = &*self.sprites;
            let g = &*self.cell;
            let (sa, sb) = (sa.borrow(), sb.borrow());
            (
                sa.calculate_collide(Some(&sb), false, false, false, false, m, g),
                sa.calculate_collide(Some(&sb), false, true, false, false, m, g),
                sa.calculate_vision_blocked(Some(&sb), m, g),
            )
        };
        let (dx, dy, dz) = (1 + b.0 - a.0, 1 + b.1 - a.1, 1 + b.2 - a.2);
        if dx < 0 || dy < 0 || dz < 0 || dx > 2 || dy > 2 || dz > 2 {
            return;
        }
        let sq_owner = self.sq_mut(a).unwrap();
        let mut sq = sq_owner.borrow_mut();
        sq.collide_matrix = set_matrix_bit(sq.collide_matrix, dx, dy, dz, collide);
        sq.path_matrix = set_matrix_bit(sq.path_matrix, dx, dy, dz, path);
        sq.vision_matrix = set_matrix_bit(sq.vision_matrix, dx, dy, dz, vision);
    }

    /// `ReCalculateAll(bDoReverse, a, getter)`.
    fn recalculate_all(&mut self, this: SquarePos, a: SquarePos, reverse: bool) {
        if this == a {
            return;
        }
        for p in [this, a] {
            if let Some(sq_owner) = self.sq(p) {
                let sq = sq_owner.borrow();
                sq.solid_floor_cache.set(None);
            }
        }
        self.recalc_properties(this);
        if reverse {
            self.recalc_properties(a);
        }
        // Collide, path and vision, this way then (reversed) that way; the
        // three only depend on properties, so computing each direction's
        // three together gives the same bits.
        self.recalc_matrices(this, a);
        if reverse {
            self.recalc_matrices(a, this);
        }
    }

    /// `RecalcAllWithNeighbours(true)`: the square's properties, then every
    /// existing valid neighbour in the 3x3x3 block (`ReCalculateAll`, which
    /// recalculates both squares' properties), then the four corner pairs
    /// around it.
    pub fn recalc_all_with_neighbours(&mut self, pos: SquarePos) {
        let Some(sq_owner) = self.sq(pos) else {
            return;
        };
        let sq = sq_owner.borrow();
        sq.solid_floor_cache.set(None);
        drop(sq);
        self.recalc_properties(pos);
        for x in pos.0 - 1..=pos.0 + 1 {
            for y in pos.1 - 1..=pos.1 + 1 {
                for z in pos.2 - 1..=pos.2 + 1 {
                    if !(MIN_LEVEL..=MAX_LEVEL).contains(&z) || !self.meta.is_valid_square(x, y) {
                        continue;
                    }
                    if (x, y, z) == pos || self.sq((x, y, z)).is_none() {
                        continue;
                    }
                    self.recalculate_all(pos, (x, y, z), true);
                }
            }
        }
        let nav = |env: &Self, d: IsoDirections| {
            let (dx, dy) = d.delta();
            let p = (pos.0 + dx, pos.1 + dy, pos.2);
            env.sq(p).map(|_| p)
        };
        let n = nav(self, IsoDirections::N);
        let s = nav(self, IsoDirections::S);
        let w = nav(self, IsoDirections::W);
        let e = nav(self, IsoDirections::E);
        for (a, b) in [(n, w), (n, e), (s, w), (s, e)] {
            if let (Some(a), Some(b)) = (a, b) {
                self.recalculate_all(a, b, true);
            }
        }
    }

    /// `IsoCell.checkHaveRoof(x, y)`: from level 31 down to 0, a square
    /// has a roof when a rain-blocking square is above it; squares whose
    /// flag changes get `RecalcAllWithNeighbours(true)`.
    pub fn check_have_roof(&mut self, x: i32, y: i32) {
        let mut have_roof = false;
        for z in (0..=31).rev() {
            let Some(sq_owner) = self.sq_mut((x, y, z)) else {
                continue;
            };
            let mut sq = sq_owner.borrow_mut();
            let changed = have_roof != sq.have_roof;
            sq.have_roof = have_roof;
            drop(sq);
            if changed {
                self.recalc_all_with_neighbours((x, y, z));
            }
            if self
                .sq((x, y, z))
                .unwrap()
                .borrow()
                .has_rain_blocking_tile(self.sprites)
            {
                have_roof = true;
            }
        }
    }

    /// `IsoGridSquare.getNew` + `IsoCell.ConnectNewSquare(sq, false)`: an
    /// empty square put into its chunk (`IsoChunk.setSquare`, which sets
    /// `associatedBuilding` and widens the chunk's levels, telling the
    /// pathfinding a new level appeared). No room is assigned. Returns
    /// whether a square was created (`false` outside loaded chunks, where
    /// the chunk map drops it).
    pub fn connect_new_square(&mut self, pos: SquarePos) -> bool {
        let (wx, wy) = chunk_of(pos);
        let associated = self
            .meta
            .get_associated_building_at(pos.0, pos.1)
            .ok()
            .flatten();
        let Some(ch) = self.cell.chunks.get_mut(&(wx, wy)) else {
            return false;
        };
        if !(MIN_LEVEL..=MAX_LEVEL).contains(&pos.2) {
            return false;
        }
        let new_level = !ch.is_valid_level(pos.2);
        let mut sq = IsoGridSquare::new(pos.0, pos.1, pos.2);
        sq.associated_building = associated;
        ch.set_square(pos.0 - wx * 8, pos.1 - wy * 8, pos.2, Some((sq).into()));
        if new_level {
            self.pathfind.add_chunk_to_world(self.cell, wx, wy);
        }
        true
    }

    /// `IsoCell.createNewGridSquare(x, y, z, recalcAll)`: the square there,
    /// or a new empty one (`ConnectNewSquare`) in a loaded chunk, with
    /// `RecalcAllWithNeighbours(true)` when `recalc_all`. `None` for an
    /// invalid square or one outside the loaded chunks.
    pub fn create_new_grid_square(
        &mut self,
        pos: SquarePos,
        recalc_all: bool,
    ) -> Option<SquarePos> {
        if !(MIN_LEVEL..=MAX_LEVEL).contains(&pos.2) || !self.meta.is_valid_square(pos.0, pos.1) {
            return None;
        }
        if self.sq(pos).is_some() {
            return Some(pos);
        }
        if !self.cell.chunks.contains_key(&chunk_of(pos)) || !self.connect_new_square(pos) {
            return None;
        }
        if recalc_all {
            self.recalc_all_with_neighbours(pos);
        }
        Some(pos)
    }

    /// `IsoGridSquare.EnsureSurroundNotNull()`: an empty square at each of
    /// the eight neighbours that is valid, in a loaded chunk and missing.
    pub fn ensure_surround_not_null(&mut self, pos: SquarePos) {
        for x in -1..=1 {
            for y in -1..=1 {
                let p = (pos.0 + x, pos.1 + y, pos.2);
                if x == 0 && y == 0
                    || !(MIN_LEVEL..=MAX_LEVEL).contains(&p.2)
                    || !self.meta.is_valid_square(p.0, p.1)
                    || !self.cell.chunks.contains_key(&chunk_of(p))
                {
                    continue;
                }
                if self.sq(p).is_some() {
                    continue;
                }
                self.connect_new_square(p);
            }
        }
    }

    // ---- shared tails ----

    /// The lighting part the add / remove / floor paths share: line of
    /// sight caches, `setRecalcLightTime(-1)`, the global light count and
    /// `lightSourceUpdate`.
    fn invalidate_lighting(&mut self) {
        for pn in 0..self.num_players {
            self.render.los_cache_cleared(pn);
        }
        self.set_recalc_light_time(-1.0);
        if self.render.fbo_render_chunk() {
            self.render.dirty_global_lights();
        }
        self.render.light_source_update();
    }

    /// `IsoGridSquare.setRecalcLightTime(t)`.
    pub fn set_recalc_light_time(&mut self, t: f32) {
        self.render.set_recalc_light_time(t);
        if self.render.fbo_render_chunk() && t < 0.0 {
            self.render.dirty_global_lights();
        }
    }

    /// `IsoGridSquare.invalidateRenderChunkLevel(flags)` (and
    /// `IsoObject.invalidateRenderChunkLevel`, whose render square is the
    /// object's square here).
    pub(crate) fn invalidate_render(&mut self, pos: SquarePos, flags: u64) {
        let (wx, wy) = chunk_of(pos);
        if self.cell.chunks.contains_key(&(wx, wy)) {
            self.render
                .invalidate_render_chunk_level(wx, wy, pos.2, flags);
        }
    }

    /// `PolygonalMap2.instance.squareChanged(sq)`.
    pub fn polygonal_map2_square_changed(&mut self, pos: SquarePos) {
        let native = self.pathfind.uses_native_code();
        let Some(sq_owner) = self.cell.get_grid_square(pos.0, pos.1, pos.2) else {
            return;
        };
        let (wx, wy) = chunk_of(pos);
        if !native {
            self.pathfind.square_changed(self.cell, pos);
        }
        self.render.invalidate_vispoly_chunk_level(wx, wy, pos.2);
        self.render.cutaways_square_changed(Some(pos));
        self.world.ambient_walls_square_changed(sq_owner);
        if native {
            self.pathfind.square_changed(self.cell, pos);
        }
    }

    pub(crate) fn map_collision_square_changed(&mut self, pos: SquarePos) {
        if let Some(sq_owner) = self.cell.get_grid_square(pos.0, pos.1, pos.2) {
            self.pathfind.map_collision_square_changed(sq_owner);
        }
    }

    fn regions_set_previous_flags(&mut self, pos: SquarePos) {
        let sq = self.cell.get_grid_square(pos.0, pos.1, pos.2);
        self.regions
            .set_previous_flags(sq.as_ref().map(|s| s.borrow()).as_deref(), self.sprites);
    }

    fn regions_square_changed(&mut self, pos: SquarePos, is_removal: bool) {
        self.regions
            .square_changed(self.cell, self.sprites, pos.0, pos.1, pos.2, is_removal);
    }

    /// `getPlayerBuiltFloor()`: the floor, when the square is in no
    /// building and its `roofHideBuilding` is absent or entirely
    /// empty-outside.
    pub fn player_built_floor(&self, pos: SquarePos) -> Option<ObjectId> {
        let sq_owner = self.sq(pos)?;
        let sq = sq_owner.borrow();
        let in_building = sq.room_def(self.meta).is_some_and(|r| r.building.is_some());
        let hide = sq
            .roof_hide_building
            .is_some_and(|b| !self.meta.building_defs[b.0 as usize].is_entirely_empty_outside());
        if in_building || hide {
            return None;
        }
        let f = self.sprites.tables.flag.solidfloor;
        sq.objects
            .iter()
            .map(ObjectSlot::borrow)
            .find(|o| self.spr(&o).is_some_and(|s| s.properties.has(f)))
            .map(|o| o.id)
    }

    /// `fixPlacedItemRenderOffsets()`: world items (not extended
    /// placement) snap their height to the nearest surface height of the
    /// square's objects.
    pub fn fix_placed_item_render_offsets(&mut self, pos: SquarePos) {
        let Some(sq_owner) = self.sq(pos) else {
            return;
        };
        let sq = sq_owner.borrow();
        let mut offsets: Vec<i32> = Vec::new();
        for o in sq.objects.iter().map(ObjectSlot::borrow) {
            let (surface, item_height) = match self.spr(&o) {
                Some(s) => {
                    let i = s.properties.surface_info(&self.sprites.alias);
                    (i.surface as f32, i.item_height)
                }
                None => (0.0, 0),
            };
            let v = round_to_int(surface + o.render_y_offset + item_height as f32);
            if v as f32 <= 0.0 || offsets.contains(&v) {
                continue;
            }
            offsets.push(v);
        }
        if offsets.is_empty() {
            offsets.push(0);
        }
        drop(sq);
        let sq = sq_owner.borrow();
        for mut o in sq.objects.iter().map(ObjectSlot::borrow_mut) {
            let ObjectKind::WorldInventory(w) = &mut o.kind else {
                continue;
            };
            let mut w = w.borrow_mut();
            if w.extended_placement {
                continue;
            }
            let render = round_to_int(w.zoff * 96.0);
            let mut new = 0;
            for j in 0..offsets.len() {
                if render <= offsets[j] {
                    new = offsets[j];
                    break;
                }
                new = offsets[j];
                if j < offsets.len() - 1 && render < offsets[j + 1] {
                    break;
                }
            }
            w.zoff = new as f32 / 96.0;
        }
    }

    /// `restackSheetRope()` (single player): the first sheet rope below
    /// the top object moves to the top.
    pub fn restack_sheet_rope(&mut self, pos: SquarePos) {
        let names = ["climbSheetW", "climbSheetN", "climbSheetE", "climbSheetS"];
        let flags: Vec<_> = names.iter().filter_map(|n| self.flag(n)).collect();
        let Some(sq_owner) = self.sq(pos) else {
            return;
        };
        let sq = sq_owner.borrow();
        if !flags.iter().any(|&f| sq.has(f)) {
            return;
        }
        let n = sq.objects.len();
        let found = (0..n.saturating_sub(1)).find(|&i| {
            self.spr(&sq.objects[i].borrow())
                .is_some_and(|s| flags.iter().any(|&f| s.properties.has(f)))
        });
        drop(sq);
        if let Some(i) = found {
            let sq_owner = self.sq_mut(pos).unwrap();
            let mut sq = sq_owner.borrow_mut();
            let o = sq.objects.remove(i);
            sq.objects.push((o).into());
        }
    }

    // ---- adding ----

    fn insert_object(&mut self, pos: SquarePos, obj: ObjectSlot, index: i32) -> Option<ObjectId> {
        let sprites = &*self.sprites;
        let sq_owner = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2)?;
        let mut sq = sq_owner.borrow_mut();
        let index = sq.place_wall_and_door_check(sprites, &obj.borrow(), index);
        let id = sq.new_object_id();
        obj.borrow_mut().id = id;
        if index != -1 && index >= 0 && index <= sq.objects.len() as i32 {
            sq.objects.insert(index as usize, (obj).into());
        } else {
            sq.objects.push((obj).into());
        }
        Some(id)
    }

    /// `AddTileObject(obj, index)` (index -1 appends): the object is
    /// placed (`placeWallAndDoorCheck`), joins the world (`addToWorld`) and
    /// the square and its neighbours are recalculated. Returns the
    /// object's identity on the square (`None` without a square).
    pub fn add_tile_object(
        &mut self,
        pos: SquarePos,
        obj: ObjectSlot,
        index: i32,
    ) -> Option<ObjectId> {
        self.add_object(pos, obj, index, false)
    }

    /// `AddSpecialObject(obj, index)`: [`MutationEnv::add_tile_object`]
    /// that also lists the object in `specialObjects` and restacks sheet
    /// ropes.
    pub fn add_special_object(
        &mut self,
        pos: SquarePos,
        obj: ObjectSlot,
        index: i32,
    ) -> Option<ObjectId> {
        self.add_object(pos, obj, index, true)
    }

    /// `AddSpecialTileObject(obj)`: `AddSpecialObject(obj)`.
    pub fn add_special_tile_object(&mut self, pos: SquarePos, obj: ObjectSlot) -> Option<ObjectId> {
        self.add_special_object(pos, obj, -1)
    }

    /// `transmitAddObjectToSquare(obj, index)` in single player: an object
    /// not on the square yet is added with `AddTileObject`.
    pub fn transmit_add_object_to_square(
        &mut self,
        pos: SquarePos,
        obj: ObjectSlot,
        index: i32,
    ) -> Option<ObjectId> {
        self.add_tile_object(pos, obj, index)
    }

    fn add_object(
        &mut self,
        pos: SquarePos,
        obj: ObjectSlot,
        index: i32,
        special: bool,
    ) -> Option<ObjectId> {
        self.cell
            .retire_object(ObjectSlot::retain(&obj.ref_handle()));
        self.sq(pos)?;
        self.regions_set_previous_flags(pos);
        let world_item = matches!(obj.borrow().kind, ObjectKind::WorldInventory(_));
        let id = self.insert_object(pos, obj, index)?;
        let sq_owner = self.sq_mut(pos).unwrap();
        let mut sq = sq_owner.borrow_mut();
        if special {
            sq.special_objects.push(id);
        }
        sq.burnt_out = false;
        drop(sq);
        lifecycle::add_to_world(self, ObjectRef { pos, id });
        if special {
            self.restack_sheet_rope(pos);
        }
        self.recalc_all_with_neighbours(pos);
        if !world_item {
            self.fix_placed_item_render_offsets(pos);
            self.invalidate_lighting();
            if self.player_built_floor(pos) == Some(id) {
                self.render.grid_occlusion_square_changed();
            }
        }
        self.map_collision_square_changed(pos);
        self.polygonal_map2_square_changed(pos);
        self.regions_square_changed(pos, false);
        self.invalidate_render(pos, dirty::OBJECT_ADDED);
        Some(id)
    }

    /// `addFloor(sprite)`: the floors, vegetation, tainted water and grass
    /// overlays on the square are removed (rugs stay; the new floor goes
    /// under them), the sprite gets `solidfloor`, missing neighbours are
    /// created here and on every level below down to 1, and the roof,
    /// lighting and regions are updated. The floor does not get
    /// `addToWorld`.
    pub fn add_floor(&mut self, pos: SquarePos, sprite: &str) -> Option<ObjectId> {
        self.sq(pos)?;
        self.regions_set_previous_flags(pos);
        // `new IsoObject(cell, sq, sprite)`: `IsoSpriteManager.getSprite`.
        let s = match self.sprites.named_map.get(sprite) {
            Some(&s) => s,
            None => {
                let s = self.sprites.add_sprite_named(sprite);
                self.sprites.sprites[s].name = Some(sprite.to_string());
                s
            }
        };
        let mut obj = IsoObject::new(0, ObjectKind::Object, Some(s));
        obj.tile = Some(sprite.to_string());
        let mut has_rug = false;
        let f = self.sprites.tables.flag;
        let tainted = self.flag("taintedWater");
        let mut nn = 0;
        while nn < self.sq(pos).unwrap().borrow().objects.len() {
            let object = self.sq(pos).unwrap().borrow().objects[nn].ref_handle();
            let o = object.borrow();
            let Some(ss) = self.spr(&o) else {
                nn += 1;
                continue;
            };
            let p = &ss.properties;
            let name = ss.name.as_deref();
            let floorish = p.has(f.solidfloor)
                || p.has(f.noStart)
                || p.has(f.vegitation) && ss.tile_type != self.sprites.tables.ty.tree
                || tainted.is_some_and(|t| p.has(t))
                || name.is_some_and(|n| n.starts_with("blends_grassoverlays"));
            if !floorish {
                nn += 1;
                continue;
            }
            if name.is_some_and(|n| n.starts_with("floors_rugs")) {
                has_rug = true;
                nn += 1;
                continue;
            }
            let id = o.id;
            drop(o);
            let before = self.sq(pos).unwrap().borrow().objects.len();
            self.transmit_remove_item_from_square(pos, id, true);
            // The original steps back one and looks at the same index
            // again; an object the removal left in place would be looked
            // at forever, so it is passed over here.
            if self.sq(pos).unwrap().borrow().objects.len() >= before {
                nn += 1;
            }
        }
        self.sprites.sprites[s].properties.set_flag(f.solidfloor);
        let sq_owner = self.sq_mut(pos).unwrap();
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        obj.id = id;
        if has_rug {
            sq.objects.insert(0, (ObjectSlot::new(obj)).into());
        } else {
            sq.objects.push((ObjectSlot::new(obj)).into());
        }
        drop(sq);
        self.ensure_surround_not_null(pos);
        self.recalc_properties(pos);
        self.world
            .designation_zone_add_new_roof(pos.0, pos.1, pos.2);
        self.check_have_roof(pos.0, pos.1);
        self.invalidate_lighting();
        self.recalc_all_with_neighbours(pos);
        let mut z1 = pos.2 - 1;
        while z1 > 0 {
            let below = (pos.0, pos.1, z1);
            if self.sq(below).is_none() {
                self.connect_new_square(below);
            }
            if self.sq(below).is_some() {
                self.ensure_surround_not_null(below);
                self.recalc_all_with_neighbours(below);
            }
            z1 -= 1;
        }
        self.polygonal_map2_square_changed(pos);
        self.render.grid_occlusion_square_changed();
        self.regions_square_changed(pos, false);
        if let Some(sq_owner) = self.cell.get_grid_square(pos.0, pos.1, pos.2) {
            let sq = sq_owner.borrow();
            drop(sq);
            self.render.clear_water(sq_owner);
        }
        self.invalidate_render(pos, dirty::OBJECT_ADDED);
        Some(id)
    }

    /// `addCorpse(body, false)` in single player.
    pub fn add_corpse(&mut self, pos: SquarePos, body: IsoDeadBody) {
        let burnt = self.sprites.tables.flag.burntOut;
        let Some(sq_owner) = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2) else {
            return;
        };
        let mut sq = sq_owner.borrow_mut();
        sq.static_moving.push(body);
        let i = sq.static_moving.len() - 1;
        drop(sq);
        self.world.corpse_added_to_world(sq_owner.clone(), i);
        {
            let mut sq = sq_owner.borrow_mut();
            sq.burnt_out = false;
            sq.properties.unset_flag(burnt);
        }
        self.invalidate_render(pos, dirty::CORPSE_ADDED);
    }

    /// `removeCorpse(body, false)` in single player: the corpse at
    /// `staticMovingObjects[index]` leaves the world and the square.
    pub fn remove_corpse(&mut self, pos: SquarePos, index: usize) -> Option<IsoDeadBody> {
        let sq_owner = self.sq_mut(pos)?;
        let sq = sq_owner.borrow();
        if index >= sq.static_moving.len() {
            return None;
        }
        drop(sq);
        self.invalidate_render(pos, dirty::CORPSE_REMOVED);
        let sq_owner = self.sq_mut(pos).unwrap();
        let mut sq = sq_owner.borrow_mut();
        let mut body = sq.static_moving.remove(index);
        drop(sq);
        self.world.corpse_removed_from_world(pos, &mut body);
        let sq_owner = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2).unwrap();
        self.world.lua_event(sq_owner, LuaEvent::ContainerUpdate);
        Some(body)
    }

    // ---- removing ----

    /// `RemoveTileObject(obj)`: objects of a loaded chunk are removed
    /// safely (all parts of a multi-square object). Returns the object's
    /// index (in `objects`, or in `specialObjects` when only there; -1 when
    /// on neither) and the removed object.
    pub fn remove_tile_object(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
    ) -> (i32, Option<ObjectSlot>) {
        self.remove_tile_object_with(pos, id, true)
    }

    /// `RemoveTileObject(obj, safelyRemove)`.
    pub fn remove_tile_object_with(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
        safely_remove: bool,
    ) -> (i32, Option<ObjectSlot>) {
        if safely_remove {
            return self.safely_remove_tile_object_from_square(ObjectRef { pos, id });
        }
        self.remove_tile_object_unsafe(pos, id)
    }

    /// `transmitRemoveItemFromSquare(obj, safelyRemove)` in single player:
    /// -1 unless the object is in `objects`, else `RemoveTileObject`.
    pub fn transmit_remove_item_from_square(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
        safely_remove: bool,
    ) -> (i32, Option<ObjectSlot>) {
        if self
            .sq(pos)
            .is_none_or(|sq| sq.borrow().object_index(id).is_none())
        {
            return (-1, None);
        }
        self.remove_tile_object_with(pos, id, safely_remove)
    }

    /// `RemoveTileObject(obj, false)`.
    fn remove_tile_object_unsafe(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
    ) -> (i32, Option<ObjectSlot>) {
        let receiver = self.object_handle(ObjectRef { pos, id });
        self.remove_tile_object_receiver(pos, id, receiver)
    }

    pub(crate) fn remove_tile_object_receiver(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
        receiver: Option<IsoObjectRef>,
    ) -> (i32, Option<ObjectSlot>) {
        if let Some(object) = &receiver {
            self.cell.retire_object(ObjectSlot::retain(object));
        }
        if self.sq(pos).is_none() {
            return (-1, None);
        }
        self.regions_set_previous_flags(pos);
        let sq_owner = self.sq(pos).unwrap();
        let (actual_index, mut index, table_adjustments) = {
            let sq = sq_owner.borrow();
            let actual_index = receiver
                .as_ref()
                .and_then(|r| sq.objects.iter().position(|o| o.ptr_eq_ref(r)));
            let index = actual_index.map(|i| i as i32).unwrap_or_else(|| {
                sq.special_objects
                    .iter()
                    .position(|&s| s == id)
                    .map_or(-1, |i| i as i32)
            });
            let mut adjustments = Vec::new();
            if let Some(i) = actual_index {
                let m = &*self.sprites;
                let info = |o: &IsoObject| {
                    o.sprite
                        .map(|s| m.sprites[s].properties.surface_info(&m.alias))
                        .unwrap_or_default()
                };
                let this = info(&sq.objects[i].borrow());
                if this.is_table {
                    for slot in sq.objects.iter().skip(i + 1) {
                        let o = slot.borrow();
                        let other = info(&o);
                        let countertop = o
                            .sprite
                            .is_some_and(|s| m.has_prop(&m.sprites[s], "Countertop"));
                        if other.is_table_top || other.is_table && !countertop {
                            adjustments.push((slot.ref_handle(), this.surface as f32));
                        }
                    }
                }
            }
            (actual_index, index, adjustments)
        };
        let mut removed = None;
        if let Some(i) = actual_index {
            for (object, offset) in table_adjustments {
                let mut object = object.borrow_mut();
                object.render_y_offset -= offset;
                object.sx = 0.0;
            }
            if self.player_built_floor(pos) == Some(id) {
                self.render.grid_occlusion_square_changed();
            }
            let receiver = receiver.as_ref().unwrap().clone();
            let sq_owner = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2).unwrap();
            self.world.lua_event(
                sq_owner,
                LuaEvent::ObjectAboutToBeRemoved {
                    index: i,
                    object: receiver.clone(),
                },
            );
            let sq_owner = self.sq(pos).unwrap();
            let sq = sq_owner.borrow();
            let i = sq
                .objects
                .iter()
                .position(|o| o.ptr_eq_ref(&receiver))
                .expect("OnObjectAboutToBeRemoved not allowed to remove the object");
            index = i as i32;
            let world_item = matches!(sq.objects[i].borrow().kind, ObjectKind::WorldInventory(_));
            drop(sq);
            self.invalidate_render(
                pos,
                if world_item {
                    dirty::WORLD_ITEM_REMOVED
                } else {
                    dirty::OBJECT_REMOVED
                },
            );
            let r = ObjectRef { pos, id };
            lifecycle::remove_from_world_handle(self, r, &receiver);
            removed = lifecycle::remove_from_square_handle(self, r, &receiver);
            if !world_item {
                self.recalc_all_with_neighbours(pos);
                self.check_have_roof(pos.0, pos.1);
                self.invalidate_lighting();
                self.fix_placed_item_render_offsets(pos);
            }
        }
        self.map_collision_square_changed(pos);
        let sq_owner = self.cell.get_grid_square_mut(pos.0, pos.1, pos.2).unwrap();
        self.world
            .lua_event(sq_owner, LuaEvent::TileRemoved { object: receiver });
        self.polygonal_map2_square_changed(pos);
        self.regions_square_changed(pos, true);
        (index, removed)
    }

    /// `RemoveTileObjectErosionNoRecalc(obj)`: the object leaves the world
    /// and the square, and only the square's properties are recalculated.
    pub fn remove_tile_object_erosion_no_recalc(
        &mut self,
        pos: SquarePos,
        id: ObjectId,
    ) -> (i32, Option<ObjectSlot>) {
        let Some(sq_owner) = self.sq(pos) else {
            return (-1, None);
        };
        let sq = sq_owner.borrow();
        let index = sq.object_index(id).map_or(-1, |i| i as i32);
        let on_square = index != -1 || sq.special_objects.contains(&id);
        drop(sq);
        if !on_square {
            return (index, None);
        }
        let r = ObjectRef { pos, id };
        let receiver = self.object_handle(r);
        let removed = if let Some(receiver) = receiver {
            self.cell.retire_object(ObjectSlot::retain(&receiver));
            lifecycle::remove_from_world_handle(self, r, &receiver);
            lifecycle::remove_from_square_handle(self, r, &receiver)
        } else {
            None
        };
        self.recalc_properties(pos);
        (index, removed)
    }

    /// `DeleteTileObject(obj)`: the object is taken out of `objects` with
    /// nothing else (no `removeFromWorld`, no recalculation).
    pub fn delete_tile_object(&mut self, pos: SquarePos, id: ObjectId) -> Option<ObjectSlot> {
        let sq_owner = self.sq_mut(pos)?;
        let mut sq = sq_owner.borrow_mut();
        let i = sq.object_index(id)?;
        Some(sq.remove_object_slot(i))
    }

    // ---- multi-square objects (`IsoObjectUtils`) ----

    fn object(&self, r: ObjectRef) -> Option<IsoObjectRef> {
        self.object_handle(r)
    }
    fn object_handle(&self, r: ObjectRef) -> Option<IsoObjectRef> {
        let sq = self.sq(r.pos)?;
        let sq = sq.borrow();
        sq.objects
            .iter()
            .find(|o| o.borrow().id == r.id)
            .map(ObjectSlot::ref_handle)
    }

    fn door_open_north(o: &IsoObject) -> Option<(bool, bool, bool)> {
        match &o.kind {
            ObjectKind::Door(d) => Some((d.open, d.north, true)),
            ObjectKind::Thumpable(t) => Some((t.open, t.north, false)),
            _ => None,
        }
    }

    fn int_prop(&self, o: &IsoObject, name: &str) -> Option<i32> {
        let s = self.spr(&o)?;
        let v = self.sprites.prop(s, name)?;
        crate::java::parse_int(v)
    }

    /// `IsoDoor.getDoubleDoorIndex(obj)`.
    pub fn double_door_index(&self, r: ObjectRef) -> i32 {
        let Some(o) = self.object(r) else {
            return -1;
        };
        let Some(dd) = self.int_prop(&o.borrow(), "DoubleDoor") else {
            return -1;
        };
        if !(1..=8).contains(&dd) {
            return -1;
        }
        let Some((open, _, _)) = Self::door_open_north(&o.borrow()) else {
            return -1;
        };
        if open {
            return if dd >= 5 { dd - 4 } else { -1 };
        }
        dd
    }

    /// `IsoDoor.getGarageDoorIndex(obj)`.
    pub fn garage_door_index(&self, r: ObjectRef) -> i32 {
        let Some(o) = self.object(r) else {
            return -1;
        };
        let Some(gd) = self.int_prop(&o.borrow(), "GarageDoor") else {
            return -1;
        };
        if !(1..=6).contains(&gd) {
            return -1;
        }
        let Some((open, _, _)) = Self::door_open_north(&o.borrow()) else {
            return -1;
        };
        if open {
            return if gd >= 4 { gd - 3 } else { -1 };
        }
        gd
    }

    /// The special object of the same class (door or thumpable) and
    /// facing as `like` on `pos` that `pred` accepts.
    fn find_door_part(
        &self,
        like: &IsoObject,
        pos: SquarePos,
        pred: &dyn Fn(ObjectRef) -> bool,
    ) -> Option<ObjectRef> {
        let (_, north, is_door) = Self::door_open_north(like)?;
        let sq_owner = self.sq(pos)?;
        let sq = sq_owner.borrow();
        for &sid in &sq.special_objects {
            let Some(o) = sq
                .objects
                .iter()
                .map(ObjectSlot::borrow)
                .find(|o| o.id == sid)
            else {
                continue;
            };
            let Some((_, n2, d2)) = Self::door_open_north(&o) else {
                continue;
            };
            if d2 != is_door || n2 != north {
                continue;
            }
            let r = ObjectRef { pos, id: sid };
            if pred(r) {
                return Some(r);
            }
        }
        None
    }

    /// `IsoDoor.getDoubleDoorObject(obj, index)`.
    pub fn double_door_object(&self, r: ObjectRef, index: i32) -> Option<ObjectRef> {
        let own = self.double_door_index(r);
        if own == -1 || index <= 0 {
            return None;
        }
        let o = self.object(r)?;
        let (open, north, _) = Self::door_open_north(&o.borrow())?;
        let (n, op) = (north as usize, open as usize);
        let xo = self.doors.x_offset[n][op];
        let yo = self.doors.y_offset[n][op];
        let first_x = r.pos.0 - xo[(own - 1) as usize];
        let first_y = r.pos.1 - yo[(own - 1) as usize];
        let p = (
            first_x + xo[(index - 1) as usize],
            first_y + yo[(index - 1) as usize],
            r.pos.2,
        );
        let result = self.find_door_part(&o.borrow(), p, &|c| self.double_door_index(c) == index);
        result
    }

    /// `IsoDoor.getDoubleDoorPartnerIndex(index)`.
    pub fn double_door_partner_index(mut dd: i32) -> i32 {
        if dd >= 5 {
            dd -= 4;
        }
        match dd {
            1 => 2,
            2 => 1,
            3 => 4,
            4 => 3,
            _ => -1,
        }
    }

    /// `IsoDoor.getGarageDoorPrev(obj)`.
    fn garage_door_prev(&self, r: ObjectRef) -> Option<ObjectRef> {
        let own = self.garage_door_index(r);
        if own == -1 || own == 1 {
            return None;
        }
        let o = self.object(r)?;
        let (_, north, _) = Self::door_open_north(&o.borrow())?;
        let p = (r.pos.0 - north as i32, r.pos.1 + !north as i32, r.pos.2);
        let result = self.find_door_part(&o.borrow(), p, &|c| self.garage_door_index(c) <= own);
        result
    }

    /// `IsoDoor.getGarageDoorNext(obj)`.
    fn garage_door_next(&self, r: ObjectRef) -> Option<ObjectRef> {
        let own = self.garage_door_index(r);
        if own == -1 || own == 3 {
            return None;
        }
        let o = self.object(r)?;
        let (_, north, _) = Self::door_open_north(&o.borrow())?;
        let p = (r.pos.0 + north as i32, r.pos.1 - !north as i32, r.pos.2);
        let result = self.find_door_part(&o.borrow(), p, &|c| self.garage_door_index(c) >= own);
        result
    }

    /// `IsoDoor.getGarageDoorFirst(obj)`.
    fn garage_door_first(&self, r: ObjectRef) -> Option<ObjectRef> {
        let own = self.garage_door_index(r);
        if own == -1 {
            return None;
        }
        if own == 1 {
            return Some(r);
        }
        let mut prev = self.garage_door_prev(r);
        while let Some(p) = prev {
            if self.garage_door_index(p) == 1 {
                return Some(p);
            }
            prev = self.garage_door_prev(p);
        }
        Some(r)
    }

    fn sprite_grid(&self, r: ObjectRef) -> Option<(SpriteIdx, usize)> {
        let o = self.object(r)?;
        let s = o.borrow().sprite?;
        Some((s, self.sprites.sprites[s].sprite_grid?))
    }

    /// `IsoObjectUtils.isObjectMultiSquare(obj)`.
    pub fn is_object_multi_square(&mut self, r: ObjectRef) -> bool {
        if self.object(r).is_none() {
            return false;
        }
        if self.double_door_index(r) != -1 || self.garage_door_index(r) != -1 {
            return true;
        }
        let o = self.object(r).unwrap().borrow().clone();
        if self.world.sprite_config_multi_square(r, &o).is_some() {
            return true;
        }
        self.sprite_grid(r).is_some()
    }

    /// `IsoObjectUtils.getAllMultiTileObjects(obj, list)`: `None` where
    /// the original returns false.
    pub fn get_all_multi_tile_objects(&mut self, r: ObjectRef) -> Option<Vec<ObjectRef>> {
        let dd = self.double_door_index(r);
        if dd != -1 {
            let mut out = Vec::new();
            out.extend(self.double_door_object(r, dd));
            out.extend(self.double_door_object(r, Self::double_door_partner_index(dd)));
            return (!out.is_empty()).then_some(out);
        }
        if self.garage_door_index(r) != -1 {
            let mut out = Vec::new();
            let mut o = self.garage_door_first(r);
            while let Some(p) = o {
                out.push(p);
                o = self.garage_door_next(p);
            }
            return (!out.is_empty()).then_some(out);
        }
        let o = self.object(r)?.borrow().clone();
        if let Some(all) = self.world.sprite_config_multi_square(r, &o) {
            return all;
        }
        let (s, g) = self.sprite_grid(r)?;
        let grid = &self.sprites.grids[g];
        let (ox, oy, oz) = grid.sprite_pos(s)?;
        let mut out = Vec::new();
        for z in 0..grid.levels {
            for x in 0..grid.width {
                for y in 0..grid.height {
                    let p = (r.pos.0 + x - ox, r.pos.1 + y - oy, r.pos.2 + z - oz);
                    let test = grid.get_sprite(x, y, z);
                    let found = self.sq(p).and_then(|sq| {
                        sq.borrow()
                            .objects
                            .iter()
                            .map(ObjectSlot::borrow)
                            .find(|o| o.sprite.is_some() && o.sprite == test)
                            .map(|o| o.id)
                    });
                    out.push(ObjectRef { pos: p, id: found? });
                }
            }
        }
        Some(out)
    }

    /// `IsoObjectUtils.safelyRemoveTileObjectFromSquare(obj)`.
    pub fn safely_remove_tile_object_from_square(
        &mut self,
        r: ObjectRef,
    ) -> (i32, Option<ObjectSlot>) {
        if self.is_object_multi_square(r) {
            let Some(all) = self.get_all_multi_tile_objects(r) else {
                return (-1, None);
            };
            // The original ArrayList stores the actual Java receivers before
            // any callbacks run; a later same-ID replacement is unrelated.
            let target = self.object_handle(r);
            let pinned: Vec<_> = all
                .into_iter()
                .filter_map(|part| self.object_handle(part).map(|object| (part, object)))
                .collect();
            for (_, object) in &pinned {
                self.cell.retire_object(ObjectSlot::retain(object));
            }
            let mut result = (-1, None);
            for (part, object) in pinned {
                let is_target = target
                    .as_ref()
                    .is_some_and(|t| std::rc::Rc::ptr_eq(t, &object));
                let removed = self.remove_tile_object_receiver(part.pos, part.id, Some(object));
                if is_target {
                    result = removed;
                }
            }
            return result;
        }
        if self.sq(r.pos).is_some() && self.object(r).is_some() {
            return self.remove_tile_object_unsafe(r.pos, r.id);
        }
        (-1, None)
    }
}

/// `getAdjacentSquare(dir)` for every `IsoDirections` value.
pub fn adjacent_squares(cell: &IsoCell, pos: SquarePos) -> [Option<SquareRef>; 8] {
    IsoDirections::ALL.map(|d| {
        let (dx, dy) = d.delta();
        cell.get_grid_square(pos.0 + dx, pos.1 + dy, pos.2)
    })
}

/// `PZMath.roundToInt(f)`: `(int)(f + 0.5f)`.
fn round_to_int(f: f32) -> i32 {
    (f + 0.5) as i32
}
