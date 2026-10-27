//! `addToWorld`, `removeFromWorld` and `removeFromSquare` of `IsoObject`
//! and the subclasses that override them, dispatched on [`ObjectKind`]. The
//! data they change on the object is changed here; every registry they
//! join or leave is a [`WorldCall`] to [`super::SquareMutationHooks`].

use super::{MutationEnv, ObjectRef};
use crate::objects::{IsoObject, ObjectKind};
use crate::{IsoObjectRef, ObjectSlot};
use std::cell::RefMut;

#[cfg(test)]
mod native_tests;

/// Genuine supported native floor lifecycle, separate from the data-profile
/// WorldCall recorder. Unknown registry/process/property branches stop at the
/// phase that needs them and preserve prior entity/event effects.
pub fn native_floor_add_to_world(
    object: &IsoObjectRef,
    _supplied_square: &crate::SquareRef,
    tables: &crate::sprite::SpriteTables,
    retired: &mut pz_entity::RetiredComponents,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    let unsupported = |operation| RuntimeHostError::Unsupported {
        operation,
        kind: None,
    };
    let item = crate::floor_object::runtime::item_ref(object)?;
    if let Some(item) = item {
        if pz_inventory::world_update::should_update_in_world_ref(
            &item,
            &PlainFloorContext { tables },
        )? {
            return Err(unsupported("actual cell processWorldItems registry"));
        }
        if matches!(
            item.try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("floor addToWorld inventory class"))?
                .kind,
            pz_inventory::ItemKind::Container(_)
        ) {
            return Err(unsupported("actual floor inventory addItemsToProcessItems"));
        }
    }
    // GameEntityManager.RegisterEntity returns for no components. Its
    // nonempty manager branch must not be replaced by a successful trace.
    {
        let body = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor RegisterEntity receiver"))?;
        if !body.components.is_empty() {
            return Err(unsupported("actual floor GameEntityManager.RegisterEntity"));
        }
    }
    object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor addedToWorld flag"))?
        .added_to_world_or_equipped = true;
    pz_entity::runtime_event::send_entity_event(
        &mut PlainFloorEventHost(object),
        pz_entity::runtime_event::EntityEventType::AddedToWorld,
        retired,
    )?;

    // Fresh static sprites have actual empty PropertyContainer fields.
    // Nonempty property evaluation/container producers remain a distinct
    // capability; arena/name snapshots cannot authenticate these branches.
    let sprite = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor sprite properties"))?
        .sprite
        .native_slot()
        .cloned()
        .ok_or_else(|| unsupported("actual unmanaged floor sprite properties"))?;
    {
        let sprite = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor createContainers sprite"))?;
        if !sprite.data.properties.is_empty() || sprite.data.properties.flags() != (0, 0) {
            return Err(unsupported(
                "native nonempty floor sprite property evaluation",
            ));
        }
    }
    if object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor containers"))?
        .container_count()
        != 0
    {
        return Err(unsupported("actual floor container processItems registry"));
    }
    // getPipedFuelAmount first consults actual object ModData, even when
    // the sprite has no FuelAmount property. Its live table path is pending.
    if object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor piped fuel ModData"))?
        .mod_data
        .is_some()
    {
        return Err(unsupported("actual floor piped fuel ModData query"));
    }
    // With no containers and no property flags the generator, fridge and
    // waterPiped branches are authenticated false. No RNG is consumed.
    {
        let sprite = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor tent sprite name"))?;
        if sprite
            .data
            .name
            .as_deref()
            .is_some_and(|name| name.starts_with("camping_01"))
            && matches!(sprite.data.tile_sheet_index, 0 | 3)
        {
            return Err(unsupported("actual floor tent ambient emitter"));
        }
    }
    // This entry accepts the actual WorldInventory subclass only, which is
    // neither Door/Window nor Tree. Empty properties lack AmbientSound/RGB.
    {
        let sprite = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor wind sprite"))?;
        if sprite.data.move_with_wind {
            return Err(unsupported("actual floor wind render effect"));
        }
    }
    try_flag_object_for_hot_save(object)
}

struct PlainFloorContext<'a> {
    tables: &'a crate::sprite::SpriteTables,
}
impl pz_inventory::world_update::WorldItemUpdateContext for PlainFloorContext<'_> {
    fn is_server(&self) -> pz_entity::runtime_load::HostResult<bool> {
        Ok(false)
    }
    fn is_client(&self) -> pz_entity::runtime_load::HostResult<bool> {
        Ok(false)
    }
    fn world_object_index(
        &self,
        owner: &pz_entity::EntityOwner,
    ) -> pz_entity::runtime_load::HostResult<i32> {
        use pz_entity::runtime_load::RuntimeHostError;
        let object = owner
            .try_handle::<IsoObject>()
            .ok_or(RuntimeHostError::InvalidHostState(
                "floor predicate world-item allocation",
            ))?;
        let square = {
            let body = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("floor predicate object.square"))?;
            // Native returns -1 for null square or a non-WorldInventory
            // receiver without consulting that square's list/body.
            let Some(square) = body.square_slot() else {
                return Ok(-1);
            };
            if !matches!(body.kind, ObjectKind::WorldInventory(_)) {
                return Ok(-1);
            }
            std::rc::Rc::clone(square)
        };
        let list = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor predicate square.worldObjects"))?
            .world_objects
            .clone();
        let list = list
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor predicate worldObjects"))?;
        // ArrayList.indexOf uses this actual object identity and returns the
        // first match. Other/null entries neither cast nor borrow their bodies.
        let index = list.entry_slots().position(|entry| {
            matches!(entry, crate::world_objects::WorldObjectEntry::Object(candidate)
                if std::rc::Rc::ptr_eq(candidate, &object))
        });
        match index {
            Some(index) => i32::try_from(index).map_err(|_| {
                RuntimeHostError::InvalidHostState("floor worldObjects Java index range")
            }),
            None => Ok(-1),
        }
    }
    fn world_square_outside(
        &self,
        owner: &pz_entity::EntityOwner,
    ) -> pz_entity::runtime_load::HostResult<Option<bool>> {
        use pz_entity::runtime_load::RuntimeHostError;
        let object = owner
            .try_handle::<IsoObject>()
            .ok_or(RuntimeHostError::InvalidHostState(
                "floor predicate world-item allocation",
            ))?;
        let square = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor predicate object.square"))?
            .square_slot()
            .cloned();
        let Some(square) = square else {
            return Ok(None);
        };
        let square = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor predicate square.properties"))?;
        Ok(Some(square.properties.has(self.tables.flag.exterior)))
    }
    fn rotten_food_removal_days(&self) -> pz_entity::runtime_load::HostResult<i32> {
        Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "actual floor rotten removal sandbox query",
            kind: None,
        })
    }
}

struct PlainFloorEventHost<'a>(&'a IsoObjectRef);
impl pz_entity::runtime_event::EntityEventHost for PlainFloorEventHost<'_> {
    fn entity_owner(&self) -> pz_entity::runtime_load::HostResult<pz_entity::EntityOwner> {
        Ok(pz_entity::EntityOwner::from_ref(self.0))
    }
    fn has_component_container(&self) -> pz_entity::runtime_load::HostResult<bool> {
        // The supported fresh receiver has no component container. An
        // allocated empty representation makes the same no-callback branch.
        Ok(!self
            .0
            .try_borrow()
            .map_err(|_| {
                pz_entity::runtime_load::RuntimeHostError::Borrowed(
                    "floor event component container",
                )
            })?
            .components
            .is_empty())
    }
    fn component_count(&self) -> pz_entity::runtime_load::HostResult<usize> {
        Ok(self
            .0
            .try_borrow()
            .map_err(|_| {
                pz_entity::runtime_load::RuntimeHostError::Borrowed("floor event components")
            })?
            .components
            .len())
    }
    fn component_event(
        &mut self,
        _: usize,
        _: &pz_entity::runtime_event::EntityEventRef,
        _: &mut pz_entity::RetiredComponents,
    ) -> pz_entity::runtime_load::HostResult<()> {
        Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "actual floor component onEntityEvent",
            kind: None,
        })
    }
    // Current native client profile has Java desiredAssertionStatus disabled.
    fn assertions_enabled(&self) -> bool {
        false
    }
    fn debug_enabled(&self) -> pz_entity::runtime_load::HostResult<bool> {
        Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
            operation: "native entity-event Core.debug assertion profile",
            kind: None,
        })
    }
}

/// Reads the current actual object.square slot at the native phase. The
/// retained chunk-state association survives coordinate/cell replacement.
pub fn try_flag_object_for_hot_save(
    object: &IsoObjectRef,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    let square = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("flagForHotSave object.square"))?
        .square_slot()
        .cloned();
    let Some(square) = square else {
        return Ok(());
    };
    let state = square
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("flagForHotSave square.chunk"))?
        .chunk_hot_save
        .clone();
    if let Some(state) = state {
        state
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("flagForHotSave actual chunk"))?
            .flag_for_hot_save();
    }
    Ok(())
}

/// `ObjectAmbientEmitters` logic `IsoObject.addToWorld` attaches
/// (`chunk.addObjectAmbientEmitter(obj, logic)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientEmitterKind {
    /// `FridgeHumLogic` (a powered fridge or freezer).
    FridgeHum,
    /// `WaterDripLogic` (a piped water source with water, 1 in 15).
    WaterDrip,
    /// `TentAmbianceLogic` (`camping_01` sprites 0 and 3).
    TentAmbiance,
    /// `TreeAmbianceLogic` (an `IsoTree`, 1 in 40).
    TreeAmbiance,
    /// `AmbientSoundLogic` (sprite property `AmbientSound`).
    AmbientSound,
}

/// A call `addToWorld` / `removeFromWorld` / `removeFromSquare` makes into
/// a system outside the square, in the order the original makes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldCall {
    // `GameEntity` / `IsoObject`.
    /// `GameEntity.addToWorld`: `GameEntityManager.RegisterEntity(obj)`,
    /// `addedToWorldOrEquipped = true`, `sendEntityEvent(AddedToWorld)`.
    RegisterEntity,
    /// `GameEntity.removeFromWorld(toMeta)`:
    /// `GameEntityManager.UnregisterEntity(obj, toMeta)` (`toMeta` is the
    /// object's `removeFromWorldToMeta`).
    UnregisterEntity,
    /// `container.addItemsToProcessItems()` / `removeItemsFromProcessItems()`
    /// for the object's container `container` (`getContainerByIndex`).
    ProcessItems { container: usize, add: bool },
    /// `chunk.addObjectPoweredByGenerator(obj)` / `remove...`
    /// (`couldBePoweredByGenerator()`).
    PoweredByGenerator { add: bool },
    /// `addObjectAmbientEmitter(new <kind>().init(obj))`.
    AmbientEmitter(AmbientEmitterKind),
    /// `chunk.removeObjectAmbientEmitter(obj)`.
    RemoveAmbientEmitters,
    /// `cell.addToProcessIsoObject(obj)`.
    ProcessIsoObjectAdd,
    /// `cell.addToProcessIsoObjectRemove(obj)`.
    ProcessIsoObjectRemove,
    /// `cell.addToStaticUpdaterObjectList(obj)` /
    /// `removeFromStaticUpdaterObjectList(obj)`.
    StaticUpdater { add: bool },
    /// `checkMoveWithWind()` (wind render effects for `moveWithWind`
    /// sprites).
    CheckMoveWithWind,
    /// `addLightSourceToWorld()` / `removeLightSourceFromWorld()` (the
    /// object's own `lightSource`).
    LightSource { add: bool },
    /// `flagForHotSave()`: the chunk needs a hot save.
    FlagForHotSave,
    /// `emitter.stopAll(); emitter = null` of the object's sound emitter
    /// (`removeFromWorld`; generators, traps and butcher hooks also return
    /// it to `IsoWorld`).
    StopEmitter,
    /// `clearOnOverlay()`.
    ClearOnOverlay,
    /// `FBORenderObjectHighlight` / `FBORenderObjectOutline
    /// .unregisterObject(obj)` (only with `fboRenderChunk`).
    UnregisterHighlight,
    // Subclasses.
    /// `IsoObjectAnimations.addDancingDoor` / `removeDancingDoor`.
    DancingDoor { add: bool },
    /// `cell.addToWindowList` / `removeFromWindowList`.
    WindowList { add: bool },
    /// `IsoFireManager.Add` / `Remove` (a non-permanent fire).
    FireManager { add: bool },
    /// `IsoFireManager.stopSound(fire)`.
    FireStopSound,
    /// `cell.removeLamppost(lightSource)` of a fire, fireplace or
    /// thumpable's own light source.
    RemoveObjectLamppost,
    /// `cell.removeHeatSource(heatSource)` (barbecue, fire, fireplace).
    RemoveHeatSource,
    /// `IsoGenerator.AllGenerators.add` / `remove`.
    Generators { add: bool },
    /// `HutchManager.add` / `remove`, and on removal
    /// `removeFromUpdateLists()` for every animal inside.
    HutchManager { add: bool },
    /// `ZomboidRadio.RegisterDevice` / `UnRegisterDevice`.
    RadioDevice { add: bool },
    /// `IsoWaveSignal.removeFromWorld`: `lightSource = null` and
    /// `deviceData.cleanSoundsAndEmitter()`.
    RadioCleanSounds,
    /// `cell.getLamppostPositions().add(lights[light])` /
    /// `removeLamppost(lights[light])` of a light switch (on removal the
    /// light was set inactive first).
    SwitchLamppost { light: usize, add: bool },
    /// `IsoLightSwitch.checkAmbientSound()`.
    SwitchCheckAmbientSound,
    /// `room.lightSwitches.remove(switch)` (a room light switch).
    RoomLightSwitchRemove,
    /// `IsoMannequin.addToWorld`'s `initOutfit`, `validateSkinTexture`,
    /// `validatePose`, `syncModel` and the `FBORenderCell` mannequin list
    /// (skipped for a chunk in a soft reset).
    MannequinInit,
    /// `FBORenderCell.instance.mannequinList.remove(obj)`.
    MannequinListRemove,
    /// `IsoStove.addToWorld`'s `setActivated(activated)`.
    StoveSetActivated,
    /// `IsoFeedingTrough.addToWorld`'s `checkContainer()` and
    /// `checkOverlayAfterAnimalEat()` (of the master trough for a slave).
    FeedingTroughCheck,
    /// `DesignationZoneAnimal.getZoneF(x, y, z).troughs.remove(trough)`.
    FeedingTroughZoneRemove,
    /// `IsoCarBatteryCharger.stopChargingSound()`.
    StopChargingSound,
    /// `IsoButcherHook.removeFromWorld`: the animal on it remembers the
    /// hook position.
    ButcherHookAnimal,
    /// `IsoWorldInventoryObject.addToWorld`: `processWorldItems.add` when
    /// the item updates in the world and is not listed yet.
    ProcessWorldItems { add: bool },
    /// `addItemsToProcessItems()` / `removeItemsFromProcessItems()` of the
    /// inventory of a world item that is a container.
    WorldItemContainer { add: bool },
    /// `IsoWorldInventoryObject.removeFromWorld`: `removeProcess = true`,
    /// `addToProcessItemsRemove(item)`, `ItemSoundManager.removeItem(item)`,
    /// `item.atlasTexture = null`.
    WorldItemRemoved,
    /// `IsoMovingObject.removeFromWorld`: the cell's object, add and
    /// remove lists and `MovingObjectUpdateScheduler` (pushables in the
    /// object list).
    MovingObjectRemove,
}

/// A Java method receiver remains the same allocation across host calls,
/// even when callbacks remove or replace its square membership.
struct Receiver {
    location: ObjectRef,
    object: IsoObjectRef,
}
impl std::ops::Deref for Receiver {
    type Target = ObjectRef;
    fn deref(&self) -> &ObjectRef {
        &self.location
    }
}
fn receiver(env: &MutationEnv<'_>, r: ObjectRef) -> Option<Receiver> {
    Some(Receiver {
        location: r,
        object: env.object_handle(r)?,
    })
}
fn call(env: &mut MutationEnv<'_>, r: &Receiver, c: WorldCall) {
    env.world
        .world_call(r.location, r.object.clone(), &mut *env.rand, c);
}
fn call_checked(
    env: &mut MutationEnv<'_>,
    r: &Receiver,
    c: WorldCall,
) -> pz_entity::runtime_load::HostResult<()> {
    env.world
        .try_world_call(r.location, r.object.clone(), &mut *env.rand, c)
}
fn with_obj<T>(_env: &MutationEnv<'_>, r: &Receiver, f: impl FnOnce(&IsoObject) -> T) -> Option<T> {
    Some(f(&r.object.borrow()))
}
fn obj_mut<'r>(_env: &mut MutationEnv<'_>, r: &'r Receiver) -> Option<RefMut<'r, IsoObject>> {
    Some(r.object.borrow_mut())
}

fn kind_of(env: &MutationEnv<'_>, r: &Receiver) -> Option<ObjectKind> {
    with_obj(env, r, |o| o.kind.clone())
}

fn container_types(o: &IsoObject) -> Vec<Option<String>> {
    o.containers().map(|c| c.kind.clone()).collect()
}

/// `getContainerByEitherType("fridge", "freezer")`'s index.
fn fridge_index(o: &IsoObject) -> Option<usize> {
    container_types(o)
        .iter()
        .position(|t| matches!(t.as_deref(), Some("fridge" | "freezer")))
}

/// `couldBePoweredByGenerator()`.
fn could_be_powered_by_generator(env: &mut MutationEnv<'_>, r: &Receiver) -> bool {
    // Synchronous property query: do not archive-copy live components or
    // nested item graphs merely to read power/container information.
    let o = r.object.borrow();
    if env.world.piped_fuel_amount(r.location, &o) > 0 {
        return true;
    }
    fridge_index(&o).is_some()
}

/// `IsoObject.addToWorld()`.
fn base_add(env: &mut MutationEnv<'_>, r: &Receiver) -> pz_entity::runtime_load::HostResult<()> {
    call(env, r, WorldCall::RegisterEntity);
    // GameEntity.addToWorld writes this after RegisterEntity even when the
    // manager's empty-component branch performs no registration.
    r.object.borrow_mut().added_to_world_or_equipped = true;
    {
        let sprites = &*env.sprites;
        let mut o = r.object.borrow_mut();
        // Malformed properties preserve the partially built container.
        let _ = crate::cell_loader::create_containers_from_sprite_properties(sprites, &mut o);
    }
    let n = with_obj(env, r, |o| o.container_count()).unwrap_or(0);
    for i in 0..n {
        call(
            env,
            r,
            WorldCall::ProcessItems {
                container: i,
                add: true,
            },
        );
    }
    if could_be_powered_by_generator(env, r) {
        call(env, r, WorldCall::PoweredByGenerator { add: true });
    }
    // These checks need only scalar sprite/class data. Cloning a whole live
    // floor object here would archive-copy its retained item/component graph.
    let Some((sprite, is_tree)) = with_obj(env, r, |o| {
        (o.sprite.retained(), matches!(o.kind, ObjectKind::Tree(_)))
    }) else {
        return Ok(());
    };
    let spr = match &sprite {
        // Explicit old archive-ID data profile; no actual sprite is inferred.
        crate::objects::ObjectSprite::ArchiveUnmanaged(_) => None,
        _ => sprite.try_read(env.sprites)?,
    };
    let water_piped = env.sprites.tables.flag_from_string("waterPiped");
    let piped = spr
        .as_ref()
        .zip(water_piped)
        .is_some_and(|(s, f)| s.properties.has(f));
    let tent = spr.as_ref().is_some_and(|s| {
        s.name
            .as_deref()
            .is_some_and(|n| n.starts_with("camping_01"))
            && (s.tile_sheet_index == 0 || s.tile_sheet_index == 3)
    });
    let ambient = spr
        .as_ref()
        .is_some_and(|s| env.sprites.has_prop(s, "AmbientSound"));
    drop(spr);
    let fridge_powered = {
        let object = r.object.borrow();
        fridge_index(&object).is_some_and(|i| env.world.container_powered(r.location, &object, i))
    };
    let emitter = if fridge_powered {
        Some((AmbientEmitterKind::FridgeHum, true))
    } else if piped
        && {
            let object = r.object.borrow();
            env.world.fluid_amount(r.location, &object) > 0.0
        }
        && env.rand.next(15) == 0
    {
        Some((AmbientEmitterKind::WaterDrip, false))
    } else if tent {
        Some((AmbientEmitterKind::TentAmbiance, false))
    } else if is_tree && env.rand.next(40) == 0 {
        Some((AmbientEmitterKind::TreeAmbiance, false))
    } else {
        None
    };
    if let Some((kind, process)) = emitter {
        call(env, r, WorldCall::AmbientEmitter(kind));
        if process {
            call(env, r, WorldCall::ProcessIsoObjectAdd);
        }
    }
    if ambient {
        call(
            env,
            r,
            WorldCall::AmbientEmitter(AmbientEmitterKind::AmbientSound),
        );
    }
    call(env, r, WorldCall::CheckMoveWithWind);
    call(env, r, WorldCall::LightSource { add: true });
    call(env, r, WorldCall::FlagForHotSave);
    Ok(())
}

/// Native allocations never enter tile-only property adapters or legacy
/// trace hooks. A successful true means the actual native phase completed.
fn try_native_add_to_world_dispatch(
    object: &IsoObjectRef,
    tables: &crate::sprite::SpriteTables,
    retired: &mut pz_entity::RetiredComponents,
) -> pz_entity::runtime_load::HostResult<bool> {
    use pz_entity::runtime_load::RuntimeHostError;
    let native_class = {
        let body = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("addToWorld sprite authority"))?;
        body.sprite
            .native_slot()
            .map(|_| matches!(body.kind, ObjectKind::WorldInventory(_)))
    };
    let Some(world_item) = native_class else {
        return Ok(false);
    };
    if !world_item {
        return Err(RuntimeHostError::Unsupported {
            operation: "actual native sprite subclass addToWorld",
            kind: None,
        });
    }
    let square = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("native addToWorld actual square"))?
        .square_slot()
        .cloned()
        .ok_or(RuntimeHostError::Unsupported {
            operation: "native floor addToWorld null-square capability",
            kind: None,
        })?;
    retired.push_owner(pz_entity::EntityOwner::from_ref(object));
    retired.push_owner(pz_entity::EntityOwner::from_ref(&square));
    native_floor_add_to_world(object, &square, tables, retired)?;
    Ok(true)
}

/// `obj.addToWorld()` for the object's class.
fn add_to_world_on(
    env: &mut MutationEnv<'_>,
    r: &Receiver,
) -> pz_entity::runtime_load::HostResult<()> {
    let mut retired = pz_entity::RetiredComponents::new();
    let native = try_native_add_to_world_dispatch(&r.object, &env.sprites.tables, &mut retired);
    env.cell.retire_components(retired);
    if native? {
        return Ok(());
    }
    // Class dispatch must not clone the floor payload and its actual item
    // graph merely to select this branch.
    if with_obj(env, r, |o| matches!(o.kind, ObjectKind::WorldInventory(_))).unwrap_or(false) {
        call(env, r, WorldCall::ProcessWorldItems { add: true });
        call(env, r, WorldCall::WorldItemContainer { add: true });
        base_add(env, r)?;
        return Ok(());
    }
    let Some(kind) = kind_of(env, r) else {
        return Ok(());
    };
    use ObjectKind as K;
    match kind {
        K::Barbecue(_) | K::CarBatteryCharger(_) | K::Thumpable(_) => {
            base_add(env, r)?;
            call(env, r, WorldCall::ProcessIsoObjectAdd);
        }
        K::ClothingDryer(_)
        | K::ClothingWasher(_)
        | K::CombinationWasherDryer(_)
        | K::StackedWasherDryer(_)
        | K::Compost(_)
        | K::Trap(_) => call(env, r, WorldCall::ProcessIsoObjectAdd),
        K::Door(_) => {
            base_add(env, r)?;
            call(env, r, WorldCall::DancingDoor { add: true });
        }
        K::FeedingTrough(_) => {
            base_add(env, r)?;
            call(env, r, WorldCall::ProcessIsoObjectAdd);
            call(env, r, WorldCall::FeedingTroughCheck);
        }
        K::Fire(f) => {
            if f.perm {
                call(env, r, WorldCall::StaticUpdater { add: true });
            } else {
                call(env, r, WorldCall::FireManager { add: true });
            }
        }
        K::Fireplace(_) => {
            call(env, r, WorldCall::ProcessIsoObjectAdd);
            let capacity = {
                let object = r.object.try_borrow().map_err(|_| {
                    pz_entity::runtime_load::RuntimeHostError::Borrowed("fireplace sprite")
                })?;
                let sprite = object.sprite.try_read(env.sprites)?;
                sprite
                    .as_ref()
                    .is_some_and(|sprite| env.sprites.has_prop(sprite, "ContainerCapacity"))
            };
            if capacity {
                call(
                    env,
                    r,
                    WorldCall::ProcessItems {
                        container: 0,
                        add: true,
                    },
                );
            }
        }
        K::Generator(_) => {
            call(env, r, WorldCall::ProcessIsoObjectAdd);
            call(env, r, WorldCall::Generators { add: true });
        }
        K::Hutch(h) => {
            base_add(env, r)?;
            if h.master.is_some() {
                call(env, r, WorldCall::HutchManager { add: true });
            }
        }
        K::Jukebox => {
            base_add(env, r)?;
            call(env, r, WorldCall::StaticUpdater { add: true });
        }
        K::LightSwitch(_) => {
            let (light_room, lights, can_be_modified) = {
                let Some(mut o) = obj_mut(env, r) else {
                    return Ok(());
                };
                let ObjectKind::LightSwitch(s) = &mut o.kind else {
                    return Ok(());
                };
                if !s.activated {
                    s.last_minute_stamp = -1;
                }
                (s.light_room, s.lights.len(), s.can_be_modified)
            };
            if !light_room {
                for i in 0..lights {
                    call(
                        env,
                        r,
                        WorldCall::SwitchLamppost {
                            light: i,
                            add: true,
                        },
                    );
                }
            }
            if can_be_modified && !light_room {
                call(env, r, WorldCall::StaticUpdater { add: true });
            }
            call(env, r, WorldCall::SwitchCheckAmbientSound);
            if could_be_powered_by_generator(env, r) {
                call(env, r, WorldCall::PoweredByGenerator { add: true });
            }
        }
        K::Mannequin(_) => {
            base_add(env, r)?;
            call(env, r, WorldCall::MannequinInit);
        }
        K::Stove(_) => {
            base_add(env, r)?;
            if with_obj(env, r, |o| o.container.is_none()).unwrap_or(true) {
                return Ok(());
            }
            call(env, r, WorldCall::ProcessIsoObjectAdd);
            call(
                env,
                r,
                WorldCall::ProcessItems {
                    container: 0,
                    add: true,
                },
            );
            call(env, r, WorldCall::StoveSetActivated);
            call(env, r, WorldCall::LightSource { add: true });
        }
        K::Radio(_) | K::Television(_) => {
            call(env, r, WorldCall::RadioDevice { add: true });
            call(env, r, WorldCall::StaticUpdater { add: true });
            base_add(env, r)?;
        }
        K::Window(_) => {
            base_add(env, r)?;
            call(env, r, WorldCall::WindowList { add: true });
        }
        K::WorldInventory(_) => {
            call(env, r, WorldCall::ProcessWorldItems { add: true });
            call(env, r, WorldCall::WorldItemContainer { add: true });
            base_add(env, r)?;
        }
        _ => base_add(env, r)?,
    }
    Ok(())
}

/// Checked live base lifecycle. Unregister failure preserves the added flag.
fn base_remove(env: &mut MutationEnv<'_>, r: &Receiver) -> pz_entity::runtime_load::HostResult<()> {
    remove_world_item_base(env, r)
}

/// Legacy fire-manager data projection only. Its infallible trace capability
/// cannot stand in for a checked Native registry host.
/// `IsoObject.removeFromWorld()`.
fn base_remove_data_projection(env: &mut MutationEnv<'_>, r: &Receiver) {
    call(env, r, WorldCall::UnregisterEntity);
    r.object.borrow_mut().added_to_world_or_equipped = false;
    call(env, r, WorldCall::ProcessIsoObjectRemove);
    call(env, r, WorldCall::StaticUpdater { add: false });
    let n = with_obj(env, r, |o| o.container_count()).unwrap_or(0);
    for i in 0..n {
        call(
            env,
            r,
            WorldCall::ProcessItems {
                container: i,
                add: false,
            },
        );
    }
    call(env, r, WorldCall::StopEmitter);
    call(env, r, WorldCall::RemoveAmbientEmitters);
    call(env, r, WorldCall::LightSource { add: false });
    call(env, r, WorldCall::ClearOnOverlay);
    if env.render.fbo_render_chunk() {
        call(env, r, WorldCall::UnregisterHighlight);
    }
    if could_be_powered_by_generator(env, r) {
        call(env, r, WorldCall::PoweredByGenerator { add: false });
    }
    call(env, r, WorldCall::FlagForHotSave);
}

/// `obj.removeFromWorld()` for the object's class.
fn remove_from_world_on(
    env: &mut MutationEnv<'_>,
    r: &Receiver,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    let kind = r
        .object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld class"))?
        .kind
        .clone();
    use ObjectKind as K;
    match kind {
        K::ButcherHook => {
            call_checked(env, r, WorldCall::ButcherHookAnimal)?;
            call_checked(env, r, WorldCall::StopEmitter)?;
            base_remove(env, r)?;
        }
        K::Barbecue(_) => {
            call_checked(env, r, WorldCall::RemoveHeatSource)?;
            base_remove(env, r)?;
        }
        K::CarBatteryCharger(_) => {
            call_checked(env, r, WorldCall::StopChargingSound)?;
            base_remove(env, r)?;
        }
        K::Door(_) => {
            base_remove(env, r)?;
            call_checked(env, r, WorldCall::DancingDoor { add: false })?;
        }
        K::FeedingTrough(_) => {
            call_checked(env, r, WorldCall::FeedingTroughZoneRemove)?;
            base_remove(env, r)?;
        }
        K::Fire(f) => {
            if !f.perm {
                call_checked(env, r, WorldCall::FireManager { add: false })?;
            }
            call_checked(env, r, WorldCall::FireStopSound)?;
            call_checked(env, r, WorldCall::RemoveObjectLamppost)?;
            call_checked(env, r, WorldCall::RemoveHeatSource)?;
            base_remove(env, r)?;
        }
        K::Fireplace(_) => {
            call_checked(env, r, WorldCall::RemoveObjectLamppost)?;
            call_checked(env, r, WorldCall::RemoveHeatSource)?;
            base_remove(env, r)?;
        }
        K::Generator(_) => {
            call_checked(env, r, WorldCall::Generators { add: false })?;
            call_checked(env, r, WorldCall::StopEmitter)?;
            base_remove(env, r)?;
        }
        K::Hutch(_) => {
            base_remove(env, r)?;
            call_checked(env, r, WorldCall::HutchManager { add: false })?;
        }
        K::LightSwitch(_) => {
            let (light_room, lights) = {
                let body = r
                    .object
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld light switch"))?;
                match &body.kind {
                    ObjectKind::LightSwitch(s) => (s.light_room, s.lights.len()),
                    _ => {
                        return Err(RuntimeHostError::InvalidHostState(
                            "light switch class changed",
                        ))
                    }
                }
            };
            if !light_room && lights > 0 {
                for i in 0..lights {
                    {
                        let mut o = r.object.try_borrow_mut().map_err(|_| {
                            RuntimeHostError::Borrowed("removeFromWorld light switch")
                        })?;
                        let ObjectKind::LightSwitch(s) = &mut o.kind else {
                            return Err(RuntimeHostError::InvalidHostState(
                                "light switch class changed",
                            ));
                        };
                        s.lights[i].active = false;
                    }
                    call_checked(
                        env,
                        r,
                        WorldCall::SwitchLamppost {
                            light: i,
                            add: false,
                        },
                    )?;
                }
                {
                    let mut o = r
                        .object
                        .try_borrow_mut()
                        .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld light switch"))?;
                    let ObjectKind::LightSwitch(s) = &mut o.kind else {
                        return Err(RuntimeHostError::InvalidHostState(
                            "light switch class changed",
                        ));
                    };
                    s.lights.clear();
                }
            }
            if light_room {
                call_checked(env, r, WorldCall::RoomLightSwitchRemove)?;
            }
            call_checked(env, r, WorldCall::ClearOnOverlay)?;
            base_remove(env, r)?;
        }
        K::Mannequin(_) => {
            base_remove(env, r)?;
            call_checked(env, r, WorldCall::MannequinListRemove)?;
        }
        K::Thumpable(_) => {
            call_checked(env, r, WorldCall::RemoveObjectLamppost)?;
            base_remove(env, r)?;
        }
        K::Trap(_) => {
            call_checked(env, r, WorldCall::StopEmitter)?;
            base_remove(env, r)?;
        }
        K::Radio(_) | K::Television(_) => {
            call_checked(env, r, WorldCall::RadioDevice { add: false })?;
            call_checked(env, r, WorldCall::LightSource { add: false })?;
            call_checked(env, r, WorldCall::RadioCleanSounds)?;
            base_remove(env, r)?;
        }
        K::Window(_) => {
            base_remove(env, r)?;
            call_checked(env, r, WorldCall::WindowList { add: false })?;
        }
        K::WorldInventory(_) => {
            call_checked(env, r, WorldCall::ProcessWorldItems { add: false })?;
            call_checked(env, r, WorldCall::WorldItemRemoved)?;
            call_checked(env, r, WorldCall::WorldItemContainer { add: false })?;
            base_remove(env, r)?;
        }
        K::Pushable(_) => {
            call_checked(env, r, WorldCall::MovingObjectRemove)?;
            base_remove(env, r)?;
        }
        _ => base_remove(env, r)?,
    }
    Ok(())
}

pub(super) fn add_to_world(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
) -> pz_entity::runtime_load::HostResult<()> {
    if let Some(r) = receiver(env, r) {
        add_to_world_on(env, &r)?;
    }
    Ok(())
}
pub(super) fn add_to_world_handle(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
    object: &IsoObjectRef,
) -> pz_entity::runtime_load::HostResult<()> {
    add_to_world_on(
        env,
        &Receiver {
            location: r,
            object: object.clone(),
        },
    )
}
pub(super) fn remove_from_world_handle(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
    object: &IsoObjectRef,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    // Reading the class must not clone the floor payload and its item graph.
    let world_item = matches!(
        object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld receiver"))?
            .kind,
        ObjectKind::WorldInventory(_)
    );
    if world_item {
        let receiver = Receiver {
            location: r,
            object: object.clone(),
        };
        call_checked(env, &receiver, WorldCall::ProcessWorldItems { add: false })?;
        call_checked(env, &receiver, WorldCall::WorldItemRemoved)?;
        call_checked(env, &receiver, WorldCall::WorldItemContainer { add: false })?;
        return remove_world_item_base(env, &receiver);
    }
    remove_from_world_on(
        env,
        &Receiver {
            location: r,
            object: object.clone(),
        },
    )
}

fn remove_world_item_base(
    env: &mut MutationEnv<'_>,
    r: &Receiver,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    call_checked(env, r, WorldCall::UnregisterEntity)?;
    // GameEntity.removeFromWorld: UnregisterEntity PC13, then flag PC18.
    // Do not clear before a failed unregister or execute a later system phase.
    r.object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld added flag"))?
        .added_to_world_or_equipped = false;
    call_checked(env, r, WorldCall::ProcessIsoObjectRemove)?;
    call_checked(env, r, WorldCall::StaticUpdater { add: false })?;
    let count = r
        .object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld containers"))?
        .container_count();
    for container in 0..count {
        call_checked(
            env,
            r,
            WorldCall::ProcessItems {
                container,
                add: false,
            },
        )?;
    }
    call_checked(env, r, WorldCall::StopEmitter)?;
    call_checked(env, r, WorldCall::RemoveAmbientEmitters)?;
    call_checked(env, r, WorldCall::LightSource { add: false })?;
    call_checked(env, r, WorldCall::ClearOnOverlay)?;
    if env.render.fbo_render_chunk() {
        call_checked(env, r, WorldCall::UnregisterHighlight)?;
    }
    let powered = {
        let body = r
            .object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("removeFromWorld power query"))?;
        env.world.piped_fuel_amount(r.location, &body) > 0 || fridge_index(&body).is_some()
    };
    if powered {
        call_checked(env, r, WorldCall::PoweredByGenerator { add: false })?;
    }
    call_checked(env, r, WorldCall::FlagForHotSave)?;
    Ok(())
}

/// Remove only this actual receiver; a same-ID replacement is a different object.
pub(super) fn remove_from_square_handle(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
    object: &IsoObjectRef,
) -> pz_entity::runtime_load::HostResult<Option<ObjectSlot>> {
    use pz_entity::runtime_load::RuntimeHostError;
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_owner(pz_entity::EntityOwner::from_ref(object));
    let result = (|| {
        let (world_item, fluid) = {
            let body = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("removeFromSquare receiver"))?;
            let world_item = matches!(body.kind, ObjectKind::WorldInventory(_));
            let fluid = if world_item {
                body.components
                    .get_ref(pz_entity::ComponentType::FluidContainer)
                    .map_err(RuntimeHostError::Storage)?
            } else {
                None
            };
            (world_item, fluid)
        };
        if let Some(fluid) = fluid {
            retired.push_component(fluid);
            let item = crate::floor_object::runtime::item_ref(object)?.ok_or(
                RuntimeHostError::InvalidHostState(
                    "world fluid transfer requires a live floor item",
                ),
            )?;
            retired.push_owner(pz_entity::EntityOwner::from_ref(&item));
            // Native transfer runs before either membership is removed. The
            // item.worldItem field is independent and is not cleared here.
            crate::floor_object::runtime::fluid_to_item(object, &item, &mut retired)?;
        }
        let Some(sq_owner) = env.cell.get_grid_square_mut(r.pos.0, r.pos.1, r.pos.2) else {
            return Ok(None);
        };
        let mut sq = sq_owner
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("removeFromSquare square"))?;
        if world_item {
            let removed = sq
                .world_objects
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("removeFromSquare world list"))?
                .remove_first_object(object);
            if let Some(crate::world_objects::WorldObjectEntry::Object(owner)) = removed {
                retired.push_owner(pz_entity::EntityOwner::from_ref(&owner));
                sq.retire_object(ObjectSlot::retain(&owner));
            }
        }
        let removed = sq
            .objects
            .iter()
            .position(|o| o.ptr_eq_ref(object))
            .map(|i| sq.remove_object_slot(i));
        if let Some(i) = sq.special_objects.iter().position(|&s| s == r.id) {
            sq.special_objects.remove(i);
        }
        Ok(removed)
    })();
    // Both success and every partial-prefix error hand off physical pins
    // before returning through any enclosing world/sprite guards.
    env.cell.retire_components(retired);
    result
}
impl MutationEnv<'_> {
    /// Invoke `addToWorld` on the selected actual receiver. Host callbacks
    /// must be staged outside enclosing world/sprite guards by the caller.
    pub fn object_add_to_world(
        &mut self,
        r: ObjectRef,
        object: &IsoObjectRef,
    ) -> pz_entity::runtime_load::HostResult<()> {
        add_to_world_handle(self, r, object)
    }
    pub(crate) fn object_remove_from_world(
        &mut self,
        r: ObjectRef,
    ) -> pz_entity::runtime_load::HostResult<()> {
        let Some(object) = self.try_object_handle(r)? else {
            return Ok(());
        };
        self.object_remove_from_world_detached(r, &object)
    }
    /// A detached Java receiver is reachable by reference, never reinserted
    /// into square lists merely to execute its method.
    pub(crate) fn object_remove_from_world_detached(
        &mut self,
        r: ObjectRef,
        obj: &IsoObjectRef,
    ) -> pz_entity::runtime_load::HostResult<()> {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(pz_entity::EntityOwner::from_ref(obj));
        let result = remove_from_world_handle(self, r, obj);
        self.cell.retire_components(retired);
        result
    }
    pub(crate) fn world_call_on(&mut self, r: ObjectRef, obj: &IsoObjectRef, c: WorldCall) {
        self.world.world_call(r, obj.clone(), &mut *self.rand, c);
    }
    pub(crate) fn base_remove_from_world_detached_data_projection(
        &mut self,
        r: ObjectRef,
        obj: &IsoObjectRef,
    ) {
        base_remove_data_projection(
            self,
            &Receiver {
                location: r,
                object: obj.clone(),
            },
        );
    }
}
