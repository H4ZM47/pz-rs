//! `addToWorld`, `removeFromWorld` and `removeFromSquare` of `IsoObject`
//! and the subclasses that override them, dispatched on [`ObjectKind`]. The
//! data they change on the object is changed here; every registry they
//! join or leave is a [`WorldCall`] to [`super::SquareMutationHooks`].

use super::{MutationEnv, ObjectRef};
use crate::objects::{IsoObject, ObjectKind};
use crate::{IsoObjectRef, ObjectSlot};
use std::cell::RefMut;

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
    /// `GameEntityFactory.TransferComponent(obj, item, FluidContainer)`
    /// (`IsoWorldInventoryObject.removeFromSquare` of an object with a
    /// fluid container).
    WorldItemTransferFluid,
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
    let Some(o) = with_obj(env, r, |o| o.clone()) else {
        return false;
    };
    if env.world.piped_fuel_amount(r.location, &o) > 0 {
        return true;
    }
    fridge_index(&o).is_some()
}

/// `IsoObject.addToWorld()`.
fn base_add(env: &mut MutationEnv<'_>, r: &Receiver) {
    call(env, r, WorldCall::RegisterEntity);
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
    let Some(o) = with_obj(env, r, |o| o.clone()) else {
        return;
    };
    let spr = o.sprite.map(|s| &env.sprites.sprites[s]);
    let water_piped = env.sprites.tables.flag_from_string("waterPiped");
    let piped = spr
        .zip(water_piped)
        .is_some_and(|(s, f)| s.properties.has(f));
    let tent = spr.is_some_and(|s| {
        s.name
            .as_deref()
            .is_some_and(|n| n.starts_with("camping_01"))
            && (s.tile_sheet_index == 0 || s.tile_sheet_index == 3)
    });
    let ambient = spr.is_some_and(|s| env.sprites.has_prop(s, "AmbientSound"));
    let fridge_powered =
        fridge_index(&o).is_some_and(|i| env.world.container_powered(r.location, &o, i));
    let emitter = if fridge_powered {
        Some((AmbientEmitterKind::FridgeHum, true))
    } else if piped && env.world.fluid_amount(r.location, &o) > 0.0 && env.rand.next(15) == 0 {
        Some((AmbientEmitterKind::WaterDrip, false))
    } else if tent {
        Some((AmbientEmitterKind::TentAmbiance, false))
    } else if matches!(o.kind, ObjectKind::Tree(_)) && env.rand.next(40) == 0 {
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
}

/// `obj.addToWorld()` for the object's class.
fn add_to_world_on(env: &mut MutationEnv<'_>, r: &Receiver) {
    let Some(kind) = kind_of(env, r) else {
        return;
    };
    use ObjectKind as K;
    match kind {
        K::Barbecue(_) | K::CarBatteryCharger(_) | K::Thumpable(_) => {
            base_add(env, r);
            call(env, r, WorldCall::ProcessIsoObjectAdd);
        }
        K::ClothingDryer(_)
        | K::ClothingWasher(_)
        | K::CombinationWasherDryer(_)
        | K::StackedWasherDryer(_)
        | K::Compost(_)
        | K::Trap(_) => call(env, r, WorldCall::ProcessIsoObjectAdd),
        K::Door(_) => {
            base_add(env, r);
            call(env, r, WorldCall::DancingDoor { add: true });
        }
        K::FeedingTrough(_) => {
            base_add(env, r);
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
            let capacity = with_obj(env, r, |o| {
                o.sprite.is_some_and(|s| {
                    env.sprites
                        .has_prop(&env.sprites.sprites[s], "ContainerCapacity")
                })
            })
            .unwrap_or(false);
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
            base_add(env, r);
            if h.master.is_some() {
                call(env, r, WorldCall::HutchManager { add: true });
            }
        }
        K::Jukebox => {
            base_add(env, r);
            call(env, r, WorldCall::StaticUpdater { add: true });
        }
        K::LightSwitch(_) => {
            let (light_room, lights, can_be_modified) = {
                let Some(mut o) = obj_mut(env, r) else {
                    return;
                };
                let ObjectKind::LightSwitch(s) = &mut o.kind else {
                    return;
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
            base_add(env, r);
            call(env, r, WorldCall::MannequinInit);
        }
        K::Stove(_) => {
            base_add(env, r);
            if with_obj(env, r, |o| o.container.is_none()).unwrap_or(true) {
                return;
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
            base_add(env, r);
        }
        K::Window(_) => {
            base_add(env, r);
            call(env, r, WorldCall::WindowList { add: true });
        }
        K::WorldInventory(_) => {
            call(env, r, WorldCall::ProcessWorldItems { add: true });
            call(env, r, WorldCall::WorldItemContainer { add: true });
            base_add(env, r);
        }
        _ => base_add(env, r),
    }
}

/// `IsoObject.removeFromWorld()`.
fn base_remove(env: &mut MutationEnv<'_>, r: &Receiver) {
    call(env, r, WorldCall::UnregisterEntity);
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
fn remove_from_world_on(env: &mut MutationEnv<'_>, r: &Receiver) {
    let Some(kind) = kind_of(env, r) else {
        return;
    };
    use ObjectKind as K;
    match kind {
        K::ButcherHook => {
            call(env, r, WorldCall::ButcherHookAnimal);
            call(env, r, WorldCall::StopEmitter);
            base_remove(env, r);
        }
        K::Barbecue(_) => {
            call(env, r, WorldCall::RemoveHeatSource);
            base_remove(env, r);
        }
        K::CarBatteryCharger(_) => {
            call(env, r, WorldCall::StopChargingSound);
            base_remove(env, r);
        }
        K::Door(_) => {
            base_remove(env, r);
            call(env, r, WorldCall::DancingDoor { add: false });
        }
        K::FeedingTrough(_) => {
            call(env, r, WorldCall::FeedingTroughZoneRemove);
            base_remove(env, r);
        }
        K::Fire(f) => {
            if !f.perm {
                call(env, r, WorldCall::FireManager { add: false });
            }
            call(env, r, WorldCall::FireStopSound);
            call(env, r, WorldCall::RemoveObjectLamppost);
            call(env, r, WorldCall::RemoveHeatSource);
            base_remove(env, r);
        }
        K::Fireplace(_) => {
            call(env, r, WorldCall::RemoveObjectLamppost);
            call(env, r, WorldCall::RemoveHeatSource);
            base_remove(env, r);
        }
        K::Generator(_) => {
            call(env, r, WorldCall::Generators { add: false });
            call(env, r, WorldCall::StopEmitter);
            base_remove(env, r);
        }
        K::Hutch(_) => {
            base_remove(env, r);
            call(env, r, WorldCall::HutchManager { add: false });
        }
        K::LightSwitch(_) => {
            let (light_room, lights) = with_obj(env, r, |o| match &o.kind {
                ObjectKind::LightSwitch(s) => (s.light_room, s.lights.len()),
                _ => (true, 0),
            })
            .unwrap_or((true, 0));
            if !light_room && lights > 0 {
                for i in 0..lights {
                    if let Some(mut o) = obj_mut(env, r) {
                        let ObjectKind::LightSwitch(s) = &mut o.kind else {
                            unreachable!()
                        };
                        s.lights[i].active = false;
                    }
                    call(
                        env,
                        r,
                        WorldCall::SwitchLamppost {
                            light: i,
                            add: false,
                        },
                    );
                }
                if let Some(mut o) = obj_mut(env, r) {
                    let ObjectKind::LightSwitch(s) = &mut o.kind else {
                        unreachable!()
                    };
                    s.lights.clear();
                }
            }
            if light_room {
                call(env, r, WorldCall::RoomLightSwitchRemove);
            }
            call(env, r, WorldCall::ClearOnOverlay);
            base_remove(env, r);
        }
        K::Mannequin(_) => {
            base_remove(env, r);
            call(env, r, WorldCall::MannequinListRemove);
        }
        K::Thumpable(_) => {
            call(env, r, WorldCall::RemoveObjectLamppost);
            base_remove(env, r);
        }
        K::Trap(_) => {
            call(env, r, WorldCall::StopEmitter);
            base_remove(env, r);
        }
        K::Radio(_) | K::Television(_) => {
            call(env, r, WorldCall::RadioDevice { add: false });
            call(env, r, WorldCall::LightSource { add: false });
            call(env, r, WorldCall::RadioCleanSounds);
            base_remove(env, r);
        }
        K::Window(_) => {
            base_remove(env, r);
            call(env, r, WorldCall::WindowList { add: false });
        }
        K::WorldInventory(_) => {
            call(env, r, WorldCall::ProcessWorldItems { add: false });
            call(env, r, WorldCall::WorldItemRemoved);
            call(env, r, WorldCall::WorldItemContainer { add: false });
            base_remove(env, r);
        }
        K::Pushable(_) => {
            call(env, r, WorldCall::MovingObjectRemove);
            base_remove(env, r);
        }
        _ => base_remove(env, r),
    }
}

pub(super) fn add_to_world(env: &mut MutationEnv<'_>, r: ObjectRef) {
    if let Some(r) = receiver(env, r) {
        add_to_world_on(env, &r);
    }
}
pub(super) fn add_to_world_handle(env: &mut MutationEnv<'_>, r: ObjectRef, object: &IsoObjectRef) {
    add_to_world_on(
        env,
        &Receiver {
            location: r,
            object: object.clone(),
        },
    );
}
pub(super) fn remove_from_world(env: &mut MutationEnv<'_>, r: ObjectRef) {
    if let Some(r) = receiver(env, r) {
        remove_from_world_on(env, &r);
    }
}
pub(super) fn remove_from_world_handle(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
    object: &IsoObjectRef,
) {
    remove_from_world_on(
        env,
        &Receiver {
            location: r,
            object: object.clone(),
        },
    );
}

/// Remove only this actual receiver; a same-ID replacement is a different object.
pub(super) fn remove_from_square_handle(
    env: &mut MutationEnv<'_>,
    r: ObjectRef,
    object: &IsoObjectRef,
) -> Option<ObjectSlot> {
    let world_item = {
        let o = object.borrow();
        (
            matches!(o.kind, ObjectKind::WorldInventory(_)),
            o.has_components,
        )
    };
    if world_item == (true, true) {
        call(
            env,
            &Receiver {
                location: r,
                object: object.clone(),
            },
            WorldCall::WorldItemTransferFluid,
        );
    }
    let sq_owner = env.cell.get_grid_square_mut(r.pos.0, r.pos.1, r.pos.2)?;
    let mut sq = sq_owner.borrow_mut();
    if world_item.0 {
        let removed = sq.world_objects.borrow_mut().remove_first_object(object);
        if let Some(crate::world_objects::WorldObjectEntry::Object(owner)) = removed {
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
    removed
}
impl MutationEnv<'_> {
    /// Invoke `addToWorld` on the selected actual receiver. Host callbacks
    /// must be staged outside enclosing world/sprite guards by the caller.
    pub fn object_add_to_world(&mut self, r: ObjectRef, object: &IsoObjectRef) {
        add_to_world_handle(self, r, object);
    }
    pub(crate) fn object_remove_from_world(&mut self, r: ObjectRef) {
        remove_from_world(self, r);
    }
    /// A detached Java receiver is reachable by reference, never reinserted
    /// into square lists merely to execute its method.
    pub(crate) fn object_remove_from_world_detached(&mut self, r: ObjectRef, obj: &IsoObjectRef) {
        remove_from_world_handle(self, r, obj);
    }
    pub(crate) fn world_call_on(&mut self, r: ObjectRef, obj: &IsoObjectRef, c: WorldCall) {
        self.world.world_call(r, obj.clone(), &mut *self.rand, c);
    }
    pub(crate) fn base_remove_from_world_detached(&mut self, r: ObjectRef, obj: &IsoObjectRef) {
        base_remove(
            self,
            &Receiver {
                location: r,
                object: obj.clone(),
            },
        );
    }
}
