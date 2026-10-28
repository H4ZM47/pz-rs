//! `IsoObject.factoryFromFileInput`, `IsoObject.load`/`save` and the
//! `load`/`save` overrides of the object classes chunk files hold.

use super::animal::IsoAnimal;
use super::io::{err, has, BitHeaderWrite, Reader, SaveResult, Writer};
use super::items::ItemBlob;
use super::lua_table::{present, LuaKey, LuaTable, LuaValue};
use super::visual::{load_container, save_container, HumanVisual};
use super::SaveCtx;
use crate::objects::*;
use crate::sprite::runtime::ObjectSprite;
use crate::sprite::SpriteIdx;

#[cfg(test)]
mod live_floor_tests;

/// `IsoSprite.CreateSprite` gives a sprite this id.
pub const UNREGISTERED_SPRITE_ID: i32 = 20_000_000;

/// The object `IsoObject.factoryFromFileInput` creates for `name`, in its
/// state before `load`. `Err` for classes chunk files cannot hold here
/// (characters, vehicles, animals) or that are not ported.
fn instantiate(name: Option<&str>, blood_body_parts: usize) -> SaveResult<ObjectKind> {
    Ok(match name {
        None | Some("IsoObject") => ObjectKind::Object,
        Some("Jukebox") => ObjectKind::Jukebox,
        Some("Curtain") => ObjectKind::Curtain(IsoCurtain::default()),
        Some("Radio") => ObjectKind::Radio(DeviceData::default()),
        Some("Television") => ObjectKind::Television(DeviceData::default()),
        Some("Barbecue") => ObjectKind::Barbecue(IsoBarbecue::default()),
        Some("ClothingDryer") => ObjectKind::ClothingDryer(DryerLogic::default()),
        Some("ClothingWasher") => ObjectKind::ClothingWasher(WasherLogic::default()),
        Some("Fireplace") => ObjectKind::Fireplace(IsoFireplace::default()),
        Some("Stove") => ObjectKind::Stove(IsoStove::default()),
        Some("Door") => ObjectKind::Door(IsoDoor::default()),
        Some("Thumpable") => ObjectKind::Thumpable(Box::default()),
        Some("IsoBrokenGlass") => ObjectKind::BrokenGlass,
        Some("Mannequin") => ObjectKind::Mannequin(Box::new(IsoMannequin::new(blood_body_parts))),
        Some("Window") => ObjectKind::Window(IsoWindow::default()),
        Some("Tree") => ObjectKind::Tree(IsoTree {
            size: 1,
            log_yield: 1,
            damage: 500,
        }),
        Some("LightSwitch") => ObjectKind::LightSwitch(IsoLightSwitch::default()),
        Some("CombinationWasherDryer") => {
            ObjectKind::CombinationWasherDryer(WasherDryer::default())
        }
        Some("StackedWasherDryer") => ObjectKind::StackedWasherDryer(WasherDryer::default()),
        Some("ButcherHook") => ObjectKind::ButcherHook,
        Some("IsoWindowFrame") => ObjectKind::WindowFrame { north: false },
        Some("IsoAnimalTrack") => ObjectKind::AnimalTrack(Box::default()),
        // The remaining classes read every saved field in load; the kind is
        // built there.
        Some(
            n @ ("WorldInventoryItem"
            | "IsoCompost"
            | "FeedingTrough"
            | "Barricade"
            | "IsoHutch"
            | "IsoGenerator"
            | "IsoTrap"
            | "IsoCarBatteryCharger"
            | "Fire"
            | "Pushable"
            | "WheelieBin"),
        ) => return err(format!("deferred:{n}")),
        Some(n) => return err(format!("object class {n} is not supported in chunk files")),
    })
}

/// Reads one `IsoObject.factoryFromFileInput` + `load`. `Ok(None)` when the
/// serialise flag is clear (nothing follows).
pub fn load_object(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    id: ObjectId,
) -> SaveResult<Option<IsoObject>> {
    if !r.bool()? {
        return Ok(None);
    }
    let class_id = r.i8()?;
    // An unknown id makes a plain IsoObject (outside debug builds).
    let name = ctx.tables.class_name(class_id);
    let mut obj = IsoObject::new(id, ObjectKind::Object, None);
    match instantiate(name, ctx.tables.blood_body_parts) {
        Ok(kind) => {
            obj.kind = kind;
            obj.components = archive_constructor_components(ctx, name.unwrap_or("IsoObject"))?;
            obj.has_components = !obj.components.is_empty();
            load_base(r, ctx, wv, &mut obj)?;
            load_tail(r, ctx, wv, &mut obj)?;
        }
        Err(e) if e.0.starts_with("deferred:") => {
            if name == Some("IsoGenerator") {
                crate::world_state::generator_constructed()?;
            }
            obj.components = archive_constructor_components(ctx, name.unwrap_or("IsoObject"))?;
            if name == Some("FeedingTrough") {
                feeding_trough_constructor(&mut obj)?;
            }
            obj.has_components = !obj.components.is_empty();
            load_special(r, ctx, wv, &mut obj, &e.0["deferred:".len()..])?;
        }
        Err(e) => return Err(e),
    }
    Ok(Some(obj))
}

/// Actual floor receiver retained with every completed prefix on failure.
/// Other classes still use the explicit existing by-value archive decoder.
#[derive(Debug)]
#[must_use]
pub struct WorldObjectLoadFailure {
    pub error: pz_entity::runtime_load::RuntimeHostError,
    pub object: Option<crate::IsoObjectRef>,
    pub retired: pz_entity::RetiredComponents,
    pub position: usize,
    pub phase: &'static str,
}
impl WorldObjectLoadFailure {
    pub fn into_error_and_retire(mut self) -> super::io::SaveError {
        if let Some(object) = self.object.take() {
            self.retired
                .push_owner(pz_entity::EntityOwner::from_ref(&object));
        }
        pz_inventory::component_host::retire_components(self.retired);
        super::io::SaveError(format!(
            "world object load {}: {:?}",
            self.phase, self.error
        ))
    }
}

/// Production floor loading allocates the full receiver BEFORE any load field
/// writes. The same reference is assigned to item.worldItem before tint and
/// later reads. Context-free archive fixtures retain their explicit value path.
pub fn load_object_slot_data_projection(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    id: ObjectId,
) -> Result<Option<crate::ObjectSlot>, WorldObjectLoadFailure> {
    load_object_slot_profile(r, ctx, wv, id, None)
}

pub fn load_object_slot_with_visual_host(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    id: ObjectId,
    host: &mut dyn crate::square_mutation::NativeFloorHost,
) -> Result<Option<crate::ObjectSlot>, WorldObjectLoadFailure> {
    load_object_slot_profile(r, ctx, wv, id, Some(host))
}

fn load_object_slot_profile(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    id: ObjectId,
    visual_host: Option<&mut dyn crate::square_mutation::NativeFloorHost>,
) -> Result<Option<crate::ObjectSlot>, WorldObjectLoadFailure> {
    use pz_entity::runtime_load::RuntimeHostError;
    let original = r.pos;
    let native = |error: super::io::SaveError| {
        RuntimeHostError::NativeException(pz_entity::EntityError(error.0))
    };
    let header = (|| -> Result<_, RuntimeHostError> {
        if !r.bool().map_err(native)? {
            return Ok(None);
        }
        Ok(Some(r.i8().map_err(native)?))
    })();
    let class_id = match header {
        Ok(None) => return Ok(None),
        Ok(Some(class)) => class,
        Err(error) => {
            return Err(WorldObjectLoadFailure {
                error,
                object: None,
                retired: Default::default(),
                position: r.pos,
                phase: "factory header",
            })
        }
    };
    if ctx.tables.class_name(class_id) != Some("WorldInventoryItem")
        || visual_host.is_none() && ctx.items.components().entity_context().is_none()
    {
        r.pos = original;
        let value = load_object(r, ctx, wv, id).map_err(|error| WorldObjectLoadFailure {
            error: native(error),
            object: None,
            retired: Default::default(),
            position: r.pos,
            phase: "explicit archive decoder",
        })?;
        return match value {
            None => Ok(None),
            Some(value) => crate::ObjectSlot::try_new(value)
                .map(Some)
                .map_err(|failure| WorldObjectLoadFailure {
                    error: RuntimeHostError::InvalidHostState(
                        "archive world allocation owner binding failed",
                    ),
                    object: Some(failure.object),
                    retired: Default::default(),
                    position: r.pos,
                    phase: "archive publication",
                }),
        };
    }
    let slot = crate::ObjectSlot::try_new(IsoObject::new(
        id,
        ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
            IsoWorldInventoryObject {
                xoff: 0.0,
                yoff: 0.0,
                zoff: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                item: crate::floor_item::FloorItem::null(),
                drop_time: -1.0,
                ignore_remove_sandbox: false,
                extended_placement: false,
            },
        )),
        None,
    ))
    .map_err(|failure| WorldObjectLoadFailure {
        error: RuntimeHostError::InvalidHostState("native floor allocation failed"),
        object: Some(failure.object),
        retired: Default::default(),
        position: r.pos,
        phase: "native constructor",
    })?;
    let object = slot.ref_handle();
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_owner(pz_entity::EntityOwner::from_ref(&object));
    let mut phase = "offsets";
    let result = load_floor_into_actual(r, ctx, wv, &object, &mut retired, &mut phase, visual_host);
    match result {
        Ok(()) => {
            pz_inventory::component_host::retire_components(retired);
            Ok(Some(slot))
        }
        Err(error) => Err(WorldObjectLoadFailure {
            error,
            object: Some(object),
            retired,
            position: r.pos,
            phase,
        }),
    }
}

fn load_floor_into_actual(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    object: &crate::IsoObjectRef,
    retired: &mut pz_entity::RetiredComponents,
    phase: &mut &'static str,
    mut visual_host: Option<&mut dyn crate::square_mutation::NativeFloorHost>,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::{RuntimeHostError, RuntimeLoadOptions};
    let native = |error: super::io::SaveError| {
        RuntimeHostError::NativeException(pz_entity::EntityError(error.0))
    };
    let floor = crate::floor_object::runtime::floor_ref(object)?;
    // Each field is published immediately after its own successful getFloat.
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor xoff"))?
        .xoff = r.f32().map_err(native)?;
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor yoff"))?
        .yoff = r.f32().map_err(native)?;
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor zoff"))?
        .zoff = r.f32().map_err(native)?;
    r.f32().map_err(native)?;
    r.f32().map_err(native)?;
    *phase = "CreateSprite";
    let sprite = if let Some(host) = visual_host.as_deref_mut() {
        ObjectSprite::Native(crate::sprite::runtime::create_runtime_sprite(
            &host.sprite_manager()?,
            &host.sprite_tables()?,
            retired,
        )?)
    } else {
        ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID)
    };
    let old = std::mem::replace(
        &mut object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor CreateSprite publication"))?
            .sprite,
        sprite,
    );
    retired.push_payload(Box::new(old));
    crate::object_slot::observe_object(object, "zombie.iso.objects.IsoWorldInventoryObject");
    *phase = "loadItem";
    let loaded = match pz_inventory::saved::load_item_record(r, wv, ctx.items, None) {
        Ok(loaded) => loaded,
        Err(failure) => {
            let error = failure.error;
            retired.append(failure.retired);
            if let Some(item) = failure.receiver {
                retired.push_owner(pz_entity::EntityOwner::from_ref(&item));
            }
            return Err(error);
        }
    };
    retired.append(loaded.retired);
    // Item loadItem's caught native body failure yields the genuine null
    // field. It is not replaced with an opaque Raw archive carrier.
    let item = loaded.item;
    {
        let old = floor
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("publish loaded floor item"))?
            .item
            .replace_item(item.as_ref());
        retired.push_payload(Box::new(old));
    }
    crate::object_slot::observe_object(object, "zombie.iso.objects.IsoWorldInventoryObject");
    let Some(item) = item else {
        *phase = "null item trailing fields";
        r.f64().map_err(native)?;
        super::io::read_header(r, 1).map_err(native)?;
        return Ok(());
    };
    retired.push_owner(pz_entity::EntityOwner::from_ref(&item));
    *phase = "item world backlink";
    crate::floor_object::runtime::publish_item_backlink(object, &item, retired)?;
    *phase = "sprite tint";
    let tint = {
        let item = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor item tint"))?;
        [item.col[0], item.col[1], item.col[2]]
    };
    if visual_host.is_some() {
        // load writes R/G/B on the stable sprite ColorInfo and preserves A.
        let sprite = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded sprite tint pointer"))?
            .sprite
            .native_slot()
            .cloned()
            .ok_or(RuntimeHostError::InvalidHostState(
                "loaded native sprite absent",
            ))?;
        let tint_slot = sprite
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded sprite tint pointer"))?
            .tint_slot()
            .cloned()
            .ok_or(RuntimeHostError::InvalidHostState(
                "loaded native tint absent",
            ))?;
        for (channel, value) in tint.into_iter().enumerate() {
            tint_slot
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("loaded sprite tint channel"))?[channel] =
                value;
        }
    } else {
        let mut object = object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("floor sprite tint projection"))?;
        object.tint_r = tint[0];
        object.tint_g = tint[1];
        object.tint_b = tint[2];
    }
    *phase = "dropTime";
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor dropTime"))?
        .drop_time = r.f64().map_err(native)?;
    *phase = "flags";
    let bits = super::io::read_header(r, 1).map_err(native)?;
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor ignoreRemoveSandbox"))?
        .ignore_remove_sandbox = has(bits, 1);
    if has(bits, 2) {
        *phase = "loadEntity";
        let context =
            ctx.items
                .components()
                .entity_context()
                .ok_or(RuntimeHostError::Unsupported {
                    operation: "actual floor entity context",
                    kind: None,
                })?;
        let mut host = crate::floor_object::load_host::FloorLoadHost { object, context };
        let result = {
            let mut input = pz_entity::io::Reader::borrow_cursor(r.buf, &mut r.pos)
                .map_err(RuntimeHostError::NativeException)?;
            pz_entity::runtime_load::load_native_with(
                &mut input,
                wv,
                &mut host,
                RuntimeLoadOptions {
                    collect_loaded: false,
                },
            )
        };
        match result {
            Ok(mut report) => {
                for component in report.loaded.drain(..) {
                    report.retired.push_component(component);
                }
                retired.append(report.retired);
            }
            Err(mut failure) => {
                for component in failure.report.loaded.drain(..) {
                    failure.report.retired.push_component(component);
                }
                retired.append(failure.report.retired);
                return Err(failure.error);
            }
        }
        let component = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("loaded floor fluid lookup"))?
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .map_err(RuntimeHostError::Storage)?;
        if let Some(component) = component {
            *phase = "setNonSavedFieldsFromItemScript";
            set_floor_fluid_script_fields(&item, &component)?;
        }
    }
    *phase = "extendedPlacement";
    floor
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor extendedPlacement"))?
        .extended_placement = has(bits, 4);
    object
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("floor visual continuation"))?
        .pending_floor_visual_tail = Some(FloorVisualTail::LoadedTextureAndSprite);
    let actual_texture_loaded = if let Some(host) = visual_host.as_deref_mut() {
        *phase = "native loaded texture and scale";
        let nonnull = crate::square_mutation::load_floor_sprite_visual(object, host, retired)?;
        object
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("loaded native visual completion"))?
            .pending_floor_visual_tail = None;
        if !nonnull {
            return Ok(());
        }
        Some(true)
    } else {
        None
    };
    // The pre-existing renderer uses item/model inputs instead of an actual
    // Java Sprite object. Do not fabricate a successful LoadSingleTexture to
    // decide the later conditional duplicate-Fluid removal.
    let both_fluid = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor duplicate fluid check"))?
        .components
        .runtime()
        .presence(pz_entity::ComponentType::FluidContainer)
        != pz_entity::EntryPresence::Absent
        && item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor item duplicate fluid check"))?
            .components
            .runtime()
            .presence(pz_entity::ComponentType::FluidContainer)
            != pz_entity::EntryPresence::Absent;
    if both_fluid {
        *phase = "native texture-dependent duplicate Fluid removal";
        match actual_texture_loaded
            .map(pz_entity::Lookup::Found)
            .unwrap_or_else(|| ctx.hooks.floor_load_texture_status(object))
        {
            pz_entity::Lookup::Found(false) => (),
            pz_entity::Lookup::Found(true) => {
                if let Some(component) =
                    pz_inventory::component_host::detach_component_for_transfer(
                        &item,
                        pz_entity::ComponentType::FluidContainer,
                        retired,
                    )?
                {
                    crate::floor_object::runtime::release_detached(&component, retired)?;
                }
            }
            _ => {
                return Err(RuntimeHostError::Unsupported {
                    operation:
                        "actual floor LoadSingleTexture outcome before duplicate Fluid removal",
                    kind: Some(pz_entity::ComponentType::FluidContainer),
                })
            }
        }
    }
    Ok(())
}

fn set_floor_fluid_script_fields(
    item: &pz_inventory::shared::ItemRef,
    component: &pz_entity::ComponentRef,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    let script = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor item script"))?
        .template
        .script
        .clone()
        .ok_or(RuntimeHostError::Unsupported {
            operation: "actual floor item script lookup",
            kind: Some(pz_entity::ComponentType::FluidContainer),
        })?;
    let script = script
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("floor item script definition"))?;
    for entry in script.entity.component_scripts() {
        if entry.component_type() == pz_entity::ComponentType::FluidContainer {
            let fluid = entry.as_any().downcast_ref::<pz_scripts::objects::entity::components::fluids::FluidContainerScript>()
                .ok_or(RuntimeHostError::InvalidHostState("floor fluid script class mismatch"))?;
            let mut body = component
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("floor non-saved fluid fields"))?;
            let body = body.fluid_mut().ok_or(RuntimeHostError::InvalidHostState(
                "floor Fluid actual class mismatch",
            ))?;
            body.custom_drink_sound = Some(fluid.custom_drink_sound().into());
            body.fills_with_clean_water = fluid.is_filled_with_clean_water();
            break;
        }
    }
    Ok(())
}

/// `new IsoFeedingTrough(cell)`: `createFluidContainer()` while `maxWater`
/// is still 0 (the definition is read after `load`).
fn archive_constructor_components(ctx: &SaveCtx<'_>, name: &str) -> SaveResult<ItemComponents> {
    ItemComponents::from_encoded_blobs(
        ctx.hooks.factory_components(name),
        pz_inventory::component::BLOB_WORLD_VERSION,
    )
    .map_err(|rejected| {
        super::io::SaveError(format!("object component archive: {}", rejected.error))
    })
}

fn feeding_trough_constructor(obj: &mut IsoObject) -> SaveResult<()> {
    let fc = pz_entity::factory::feeding_trough_fluid_container(0.0);
    let blob = pz_inventory::component::encode_component(pz_entity::Component::FluidContainer(fc));
    // GameEntityFactory.AddComponent(this, true, fc) replaces one of the type.
    // This by-value constructor is an explicit archive boundary. Production
    // world lifecycle must obtain a real pooled ref and run its host phases.
    let mut blobs = obj
        .components
        .try_encoded_blobs()
        .map_err(|error| super::io::SaveError(error.to_string()))?;
    match blobs.iter_mut().find(|c| c.type_id == blob.type_id) {
        Some(c) => *c = blob,
        None => blobs.push(blob),
    }
    obj.components =
        ItemComponents::from_encoded_blobs(blobs, pz_inventory::component::BLOB_WORLD_VERSION)
            .map_err(|rejected| super::io::SaveError(rejected.error.to_string()))?;
    Ok(())
}

fn sprite_or_null(ctx: &SaveCtx<'_>, id: i32) -> Option<SpriteIdx> {
    ctx.sprites.idx_by_id(id)
}

/// The existing by-value world archive decoder. It is deliberately restricted
/// to encoded carriers before publication. Live component loading requires
/// the actual world owner and ordered lifecycle/manager host, not this adapter.
fn load_entity_archive(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    obj: &mut IsoObject,
) -> SaveResult<()> {
    if obj.components.is_bound() {
        return err("owned world components require the native world load host");
    }
    let versions: std::collections::HashMap<_, _> = obj
        .components
        .entry_slots()
        .filter_map(|entry| match entry {
            pz_entity::RuntimeEntryView::Encoded(encoded) => {
                Some((encoded.type_id(), encoded.world_version()))
            }
            _ => None,
        })
        .collect();
    let mut blobs = obj
        .components
        .try_encoded_blobs()
        .map_err(|error| super::io::SaveError(error.to_string()))?;
    let loaded = ctx.items.components().load_entity(r, wv, &mut blobs)?;
    if loaded.contains(&SPRITE_CONFIG_ID) {
        if matches!(obj.sprite, ObjectSprite::Native(_)) {
            return err("native SpriteConfig load requires actual runtime sprite-grid ownership");
        }
        sprite_config_archive_projection(ctx, obj.sprite.tile_projection(), &mut blobs);
    }
    obj.components = ItemComponents::from_encoded_entries(
        blobs
            .into_iter()
            .map(|blob| {
                let version = if ctx.items.components().entity_context().is_some() {
                    pz_inventory::component::BLOB_WORLD_VERSION
                } else if loaded.contains(&blob.type_id) {
                    wv
                } else {
                    versions
                        .get(&blob.type_id)
                        .copied()
                        .unwrap_or(pz_inventory::component::BLOB_WORLD_VERSION)
                };
                pz_entity::EncodedComponent::new(blob.type_id, blob.data, version)
            })
            .collect(),
    )
    .map_err(|rejected| super::io::SaveError(rejected.error.to_string()))?;
    obj.has_components = !obj.components.is_empty();
    Ok(())
}

/// `ComponentType.SpriteConfig`.
const SPRITE_CONFIG_ID: i16 = 3;

/// `SpriteConfig.load`, then `onAddedToOwner` (`initObjectInfo`) on the
/// object: what `save` writes after the script (`isMultiSquareMaster()`)
/// is the tile info of the object's sprite in the script's
/// `SpriteConfigManager` entry, not the saved flag. A script reference the
/// dictionary does not resolve leaves no script (saved as a single 0).
/// Without a script dictionary the bytes stay as loaded.
fn sprite_config_archive_projection(
    ctx: &SaveCtx<'_>,
    sprite: Option<SpriteIdx>,
    blobs: &mut [ComponentBlob],
) {
    let Some(c) = blobs.iter_mut().find(|c| c.type_id == SPRITE_CONFIG_ID) else {
        return;
    };
    // [has script] [has id, id] [script version] [master]
    let d = &c.data;
    if d.first().is_none_or(|&b| b == 0) {
        return;
    }
    let (id, master_at) = match d.get(1) {
        Some(0) => (None, 10),
        Some(_) if d.len() >= 4 => (Some(i16::from_be_bytes([d[2], d[3]])), 12),
        _ => return,
    };
    if d.len() != master_at + 1 {
        return;
    }
    let script = match id {
        // `loadScript` reads no id: no script.
        None => None,
        Some(id) => match ctx.items.script_dictionary_name("SpriteConfigs", id) {
            None => return,
            Some(name) => name.and_then(|n| ctx.sprites.sprite_config_scripts.get(&n)),
        },
    };
    let Some(config) = script else {
        // `configScript` stays null: `save` writes only the flag.
        c.data = vec![0];
        return;
    };
    let sprite = sprite.and_then(|s| ctx.sprites.sprites[s].name.as_deref());
    c.data[master_at] = ctx.sprites.is_multi_square_master(config, sprite) as u8;
}

fn save_entity(w: &mut Writer, obj: &IsoObject) -> SaveResult<()> {
    if obj.has_components && obj.components.is_empty() {
        // Components made by GameEntityFactory at map load have no saved
        // form here.
        return err("object has entity components without saved data");
    }
    obj.components
        .write_to(w, pz_inventory::component::BLOB_WORLD_VERSION)
}

/// `IsoObject.load` (after the factory bytes).
fn load_base(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32, o: &mut IsoObject) -> SaveResult<()> {
    let m = ctx.sprites;
    let sid = r.i32()?;
    o.sprite = (if sid == -1 {
        m.named_map.get("").copied()
    } else {
        sprite_or_null(ctx, sid)
    })
    .into();
    let header = r.i8()? as i64 as u64;
    if header != 0 {
        if has(header, 1) {
            let child = if has(header, 2) {
                1
            } else {
                (r.i8()? as u8) as i32
            };
            for _ in 0..child {
                let list = o.attached_anim_sprite.get_or_insert_with(Vec::new);
                let wtf = r.i32()?;
                let spr = sprite_or_null(ctx, wtf);
                let mut a = spr.map(IsoSpriteInstance::new);
                let flags = r.i8()?;
                let read_data = flags & 2 != 0;
                let read_alpha = flags & 0x10 != 0;
                if let Some(a) = a.as_mut() {
                    if flags & 4 != 0 {
                        a.flip = true;
                    }
                    if flags & 8 != 0 {
                        a.copy_target_alpha = true;
                    }
                    if read_alpha {
                        a.multiply_object_alpha = true;
                    }
                }
                if read_data {
                    let (x, y, z) = (r.f32()?, r.f32()?, r.f32()?);
                    let (cr, cg, cb) = (r.unit_byte()?, r.unit_byte()?, r.unit_byte()?);
                    if let Some(a) = a.as_mut() {
                        a.off_x = x;
                        a.off_y = y;
                        a.off_z = z;
                        a.tint_r = cr;
                        a.tint_g = cg;
                        a.tint_b = cb;
                    }
                } else if let Some(a) = a.as_mut() {
                    a.off_x = 0.0;
                    a.off_y = 0.0;
                    a.off_z = 0.0;
                    a.tint_r = 1.0;
                    a.tint_g = 1.0;
                    a.tint_b = 1.0;
                    a.alpha = 1.0;
                    a.target_alpha = 1.0;
                }
                if read_alpha {
                    let alpha = r.f32()?;
                    if let Some(a) = a.as_mut() {
                        a.alpha = alpha;
                    }
                }
                let (Some(s), Some(a)) = (spr, a) else {
                    continue;
                };
                if m.sprites[s]
                    .name
                    .as_deref()
                    .is_some_and(|n| n.starts_with("overlay_blood_"))
                {
                    o.wall_blood_splats
                        .get_or_insert_with(Vec::new)
                        .push(WallBloodSplat {
                            world_age: ctx.settings.world_age_hours as f32,
                            sprite: Some(s),
                        });
                    continue;
                }
                list.push(a);
            }
        }
        if has(header, 4) {
            let flags = r.i8()?;
            if flags & 2 != 0 {
                o.name = Some(ObjectName::Grass);
            } else if flags & 4 != 0 {
                o.name = Some(ObjectName::Dict(r.i8()?));
            } else if flags & 8 != 0 {
                o.name = Some(ObjectName::Str(r.string()?));
            }
            if flags & 0x10 != 0 {
                o.sprite_name = Some(SpriteName::Dict(r.i32()?));
            } else if flags & 0x20 != 0 {
                o.sprite_name = Some(SpriteName::Str(r.string()?));
            }
        }
        if has(header, 8) {
            o.custom_color = Some([r.unit_byte()?, r.unit_byte()?, r.unit_byte()?]);
        }
        o.do_not_sync = has(header, 16);
        o.outline_on_mouseover = has(header, 32);
        if has(header, 64) {
            let bits = super::io::read_header(r, 2)?;
            if has(bits, 1) {
                let count = r.i8()?;
                if count > 0 {
                    let list = o.wall_blood_splats.get_or_insert_with(Vec::new);
                    let lifespan = ctx.settings.blood_splat_lifespan_days;
                    let wah = ctx.settings.world_age_hours as f32;
                    for _ in 0..count {
                        let age = r.f32()?;
                        let spr = sprite_or_null(ctx, r.i32()?);
                        let age = if age < wah { age } else { wah };
                        if lifespan > 0 && wah - age >= (lifespan * 24) as f32 {
                            continue;
                        }
                        list.push(WallBloodSplat {
                            world_age: age,
                            sprite: spr,
                        });
                    }
                }
            }
            if has(bits, 2) {
                let n = r.i8()?;
                for i in 0..n.max(0) {
                    let c = load_container(r, wv, ctx.items)?;
                    if i == 0 {
                        o.container = Some(c);
                    } else {
                        o.secondary_containers.push(c);
                    }
                }
            }
            if has(bits, 4) {
                o.mod_data = Some(LuaTable::load(r, wv)?);
            }
            o.special_tooltip = has(bits, 8);
            if has(bits, 16) {
                o.key_id = r.i32()?;
            }
            o.uses_external_water_source = has(bits, 32);
            if has(bits, 64) {
                o.sheet_rope = true;
                o.sheet_rope_health = r.f32()?;
            } else {
                o.sheet_rope = false;
            }
            if has(bits, 128) {
                o.render_y_offset = r.f32()?;
            }
            if has(bits, 256) {
                let name = if has(bits, 512) {
                    SpriteName::Str(r.string()?)
                } else {
                    SpriteName::Dict(r.i32()?)
                };
                if !matches!(&name, SpriteName::Str(s) if s.is_empty()) {
                    o.overlay_sprite = Some(OverlaySprite { name, color: None });
                }
            }
            if has(bits, 1024) {
                let c = [
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                    r.unit_byte()?,
                ];
                if let Some(ov) = o.overlay_sprite.as_mut() {
                    ov.color = Some(c);
                }
            }
            o.moved_thumpable = has(bits, 2048);
            if has(bits, 4096) {
                load_entity_archive(r, ctx, wv, o)?;
            }
            if has(bits, 8192) {
                o.sprite_model_name = Some(r.string()?);
            }
        }
    }
    if o.sprite.is_none() {
        o.sprite = ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID);
    }
    Ok(())
}

/// The id `IsoObject.save` writes for the object's sprite.
pub fn sprite_save_id(ctx: &SaveCtx<'_>, o: &IsoObject) -> i32 {
    try_sprite_save_id(ctx, o).expect("archive sprite id requires an unborrowed runtime sprite")
}

pub fn try_sprite_save_id(
    ctx: &SaveCtx<'_>,
    o: &IsoObject,
) -> pz_entity::runtime_load::HostResult<i32> {
    o.sprite.try_id(ctx.sprites)
}

/// `getObjectName()`.
pub fn object_name(ctx: &SaveCtx<'_>, o: &IsoObject) -> String {
    try_object_name(ctx, o).expect("archive object name requires unborrowed native sprite metadata")
}

pub fn try_object_name(
    ctx: &SaveCtx<'_>,
    o: &IsoObject,
) -> pz_entity::runtime_load::HostResult<String> {
    if let Some(n) = o.kind.object_name() {
        return Ok(n.to_string());
    }
    match &o.name {
        Some(ObjectName::Grass) => return Ok("Grass".into()),
        Some(ObjectName::Str(s)) => return Ok(s.clone()),
        // A dictionary name never names a factory class.
        Some(ObjectName::Dict(_)) => return Ok("IsoObject".into()),
        None => {}
    }
    // Archive-only IDs have no metadata body. This explicit archive helper
    // retains its existing factory-name policy; native sprites read actual data.
    if !matches!(o.sprite, ObjectSprite::ArchiveUnmanaged(_)) {
        if let Some(p) = o
            .sprite
            .try_read(ctx.sprites)?
            .and_then(|sprite| sprite.parent_object_name.clone())
        {
            return Ok(p);
        }
    }
    Ok("IsoObject".into())
}

/// `IsoObject.save`, with the `IsoMannequin`/`IsoFire`/`IsoGenerator`
/// tweaks applied by the caller through the arguments.
fn save_base(
    w: &mut Writer,
    ctx: &SaveCtx<'_>,
    o: &IsoObject,
    with_containers: bool,
    with_attached: bool,
    mod_data: Option<&LuaTable>,
) -> SaveResult<()> {
    w.i8(1);
    w.i8(ctx.class_id(&object_name(ctx, o))?);
    w.i32(
        try_sprite_save_id(ctx, o)
            .map_err(|error| super::io::SaveError(format!("archive sprite field: {error:?}")))?,
    );
    let mut header = BitHeaderWrite::alloc(w, 1);
    if let Some(list) = o.attached_anim_sprite.as_ref().filter(|_| with_attached) {
        header.add(1);
        if list.len() == 1 {
            header.add(2);
        }
        let s = list.len().min(255);
        if s != 1 {
            w.i8(s as u8 as i8);
        }
        for a in &list[..s] {
            w.i32(ctx.sprite_id(a.sprite));
            let mut flags = 0i8;
            let write_data = a.off_x != 0.0
                || a.off_y != 0.0
                || a.off_z != 0.0
                || a.tint_r != 1.0
                || a.tint_g != 1.0
                || a.tint_b != 1.0;
            if write_data {
                flags |= 2;
            }
            if a.flip {
                flags |= 4;
            }
            if a.copy_target_alpha {
                flags |= 8;
            }
            if a.multiply_object_alpha {
                flags |= 0x10;
            }
            w.i8(flags);
            if write_data {
                w.f32(a.off_x);
                w.f32(a.off_y);
                w.f32(a.off_z);
                w.unit_byte(a.tint_r);
                w.unit_byte(a.tint_g);
                w.unit_byte(a.tint_b);
            }
            if a.multiply_object_alpha {
                w.f32(a.alpha);
            }
        }
    }
    if o.name.is_some() || o.sprite_name.is_some() {
        header.add(4);
        let mut flags = 0i8;
        match &o.name {
            Some(ObjectName::Grass) => flags |= 2,
            Some(ObjectName::Str(s)) if s == "Grass" => flags |= 2,
            Some(ObjectName::Dict(_)) => flags |= 4,
            Some(ObjectName::Str(_)) => flags |= 8,
            None => {}
        }
        match &o.sprite_name {
            Some(SpriteName::Dict(_)) => flags |= 0x10,
            Some(SpriteName::Str(_)) => flags |= 0x20,
            None => {}
        }
        w.i8(flags);
        match &o.name {
            Some(ObjectName::Dict(id)) => w.i8(*id),
            Some(ObjectName::Str(s)) if s != "Grass" => w.string(s),
            _ => {}
        }
        match &o.sprite_name {
            Some(SpriteName::Dict(id)) => w.i32(*id),
            Some(SpriteName::Str(s)) => w.string(s),
            None => {}
        }
    }
    if let Some(c) = &o.custom_color {
        header.add(8);
        for &v in c {
            w.unit_byte(v);
        }
    }
    if o.do_not_sync {
        header.add(16);
    }
    if o.outline_on_mouseover {
        header.add(32);
    }
    let mut bits = BitHeaderWrite::alloc(w, 2);
    if let Some(list) = &o.wall_blood_splats {
        bits.add(1);
        let count = list.len().min(32);
        w.i8(count as i8);
        for s in &list[list.len() - count..] {
            w.f32(s.world_age);
            let Some(spr) = s.sprite else {
                return err("NullPointerException: wall blood splat without sprite");
            };
            w.i32(ctx.sprite_id(spr));
        }
    }
    if with_containers && o.container_count() > 0 {
        bits.add(2);
        w.i8(o.container_count() as i8);
        for c in o.containers() {
            save_container(w, c, ctx.items)?;
        }
    }
    if let Some(t) = mod_data.filter(|t| !t.is_empty()) {
        bits.add(4);
        t.save(w);
    }
    if o.special_tooltip {
        bits.add(8);
    }
    if o.key_id != -1 {
        bits.add(16);
        w.i32(o.key_id);
    }
    if o.uses_external_water_source {
        bits.add(32);
    }
    if o.sheet_rope {
        bits.add(64);
        w.f32(o.sheet_rope_health);
    }
    if o.render_y_offset != 0.0 {
        bits.add(128);
        w.f32(o.render_y_offset);
    }
    if let Some(ov) = &o.overlay_sprite {
        bits.add(256);
        match &ov.name {
            SpriteName::Str(s) => {
                bits.add(512);
                w.string(s);
            }
            SpriteName::Dict(id) => w.i32(*id),
        }
        if let Some(c) = &ov.color {
            bits.add(1024);
            for &v in c {
                w.unit_byte(v);
            }
        }
    }
    if o.moved_thumpable {
        bits.add(2048);
    }
    if o.has_components || !o.components.is_empty() {
        bits.add(4096);
        save_entity(w, o)?;
    }
    if let Some(s) = &o.sprite_model_name {
        bits.add(8192);
        w.string(s);
    }
    if bits.flags != 0 {
        header.add(64);
        bits.write(w);
    } else if w.pos() == bits.start() + 2 {
        bits.discard(w);
    } else {
        unreachable!("empty bit header followed by data");
    }
    header.write(w);
    Ok(())
}

fn opt_sprite(r: &mut Reader<'_>, ctx: &SaveCtx<'_>) -> SaveResult<Option<SpriteIdx>> {
    Ok(if r.bool()? {
        sprite_or_null(ctx, r.i32()?)
    } else {
        None
    })
}

fn save_opt_sprite(w: &mut Writer, ctx: &SaveCtx<'_>, s: Option<SpriteIdx>) {
    match s {
        Some(s) => {
            w.i8(1);
            w.i32(ctx.sprite_id(s));
        }
        None => w.i8(0),
    }
}

fn sprite_id_npe(ctx: &SaveCtx<'_>, s: Option<SpriteIdx>, what: &str) -> SaveResult<i32> {
    match s {
        Some(s) => Ok(ctx.sprite_id(s)),
        None => err(format!("NullPointerException: {what} is null")),
    }
}

fn load_device(r: &mut Reader<'_>, d: &mut DeviceData) -> SaveResult<()> {
    d.load(r)
}

fn save_device(w: &mut Writer, d: &DeviceData) {
    d.save(w)
}

fn load_washer(r: &mut Reader<'_>) -> SaveResult<WasherLogic> {
    Ok(WasherLogic {
        activated: r.bool()?,
        last_update: r.f32()?,
    })
}

fn load_dryer(r: &mut Reader<'_>) -> SaveResult<DryerLogic> {
    Ok(DryerLogic {
        activated: r.bool()?,
    })
}

/// The `load` overrides of classes that call `IsoObject.load` first.
fn load_tail(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32, o: &mut IsoObject) -> SaveResult<()> {
    let m = ctx.sprites;
    let sprite_data = o
        .sprite
        .metadata_projection(m)
        .map_err(|error| super::io::SaveError(format!("archive sprite metadata: {error:?}")))?;
    let prop = |name: &str| {
        sprite_data
            .as_ref()
            .and_then(|sprite| m.prop(sprite, name).map(str::to_string))
    };
    // Sprite-valued subclass fields in this legacy archive decoder are
    // managed indices. Native floor decoding uses the separate actual host.
    let sprite = o.sprite.tile_projection();
    match &mut o.kind {
        ObjectKind::Object
        | ObjectKind::Jukebox
        | ObjectKind::BrokenGlass
        | ObjectKind::ButcherHook
        | ObjectKind::Pushable(_) => {}
        ObjectKind::AnimalTrack(t) => **t = super::movers::AnimalTracks::load(r)?,
        ObjectKind::Door(d) => {
            d.open = r.bool()?;
            d.locked = r.bool()?;
            d.north = r.bool()?;
            d.health = r.i32()?;
            d.max_health = r.i32()?;
            d.closed_sprite = sprite_or_null(ctx, r.i32()?);
            d.open_sprite = sprite_or_null(ctx, r.i32()?);
            o.outline_on_mouseover = true;
            d.pushed_strength = 2500;
            d.pushed_max_strength = 2500;
            o.key_id = r.i32()?;
            d.locked_by_key = r.bool()?;
            let b = r.i8()?;
            if b & 1 != 0 {
                d.has_curtain = true;
                d.curtain_open = b & 2 != 0;
                d.curtain_inside = b & 4 != 0;
            }
        }
        ObjectKind::Window(win) => {
            win.open = r.bool()?;
            win.north = r.bool()?;
            win.health = r.i32()?;
            win.locked = r.bool()?;
            win.perma_locked = r.bool()?;
            win.destroyed = r.bool()?;
            win.glass_removed = r.bool()?;
            win.open_sprite = opt_sprite(r, ctx)?;
            win.closed_sprite = opt_sprite(r, ctx)?;
            win.smashed_sprite = opt_sprite(r, ctx)?;
            win.glass_removed_sprite = opt_sprite(r, ctx)?;
            win.max_health = r.i32()?;
        }
        ObjectKind::Curtain(c) => {
            c.open = r.bool()?;
            c.north = r.bool()?;
            c.health = r.i32()?;
            c.barricade_strength = r.i32()?;
            let other = sprite_or_null(ctx, r.i32()?);
            if c.open {
                c.closed_sprite = other;
                c.open_sprite = sprite;
            } else {
                c.open_sprite = other;
                c.closed_sprite = sprite;
            }
        }
        ObjectKind::LightSwitch(s) => {
            s.light_room = r.bool()?;
            if wv >= 206 {
                s.room_id = r.i64()?;
            } else {
                return err("IsoLightSwitch from world version < 206");
            }
            s.activated = r.bool()?;
            s.can_be_modified = r.bool()?;
            if s.can_be_modified {
                s.use_battery = r.bool()?;
                s.has_battery = r.bool()?;
                s.bulb_item = if r.bool()? { Some(r.string()?) } else { None };
                s.power = r.f32()?;
                s.delta = r.f32()?;
                s.primary_r = r.f32()?;
                s.primary_g = r.f32()?;
                s.primary_b = r.f32()?;
            }
            s.last_minute_stamp = r.i64()?;
            s.bulb_burn_minutes = r.i32()?;
            s.street_light = sprite_data
                .as_ref()
                .is_some_and(|sp| m.has_prop(sp, "streetlight"));
        }
        ObjectKind::Tree(t) => {
            t.log_yield = r.i8()? as i32;
            t.damage = r.i8()? as i32 * 10;
            if let Some(v) = prop("tree") {
                let size = crate::java::parse_int(crate::java::java_trim(&v))
                    .ok_or_else(|| super::io::SaveError(format!("NumberFormatException: {v}")))?;
                t.size = size.clamp(1, 8);
            }
        }
        ObjectKind::WindowFrame { north } => *north = r.bool()?,
        ObjectKind::Barbecue(b) => {
            b.has_propane_tank = r.bool()?;
            b.fuel_amount_minutes = r.i32()?;
            b.lit = r.bool()?;
            b.last_update_time = r.f32()?;
            b.minutes_since_extinguished = r.i32()?;
            b.normal_sprite = opt_sprite(r, ctx)?;
            b.no_tank_sprite = opt_sprite(r, ctx)?;
        }
        ObjectKind::Fireplace(f) => {
            f.fuel_amount_minutes = r.i32()?;
            f.lit = r.bool()?;
            f.last_update_time = r.f32()?;
            f.minutes_since_extinguished = r.i32()?;
        }
        ObjectKind::Stove(s) => {
            s.activated = r.bool()?;
            s.seconds_timer = r.i32()?;
            s.max_temperature = r.f32()?;
            s.first_turn_on = r.bool()?;
            s.broken = r.bool()?;
        }
        ObjectKind::ClothingDryer(d) => *d = load_dryer(r)?,
        ObjectKind::ClothingWasher(wl) => *wl = load_washer(r)?,
        ObjectKind::CombinationWasherDryer(wd) => {
            wd.dryer_mode = r.i8()? != 0;
            wd.washer = load_washer(r)?;
            wd.dryer = load_dryer(r)?;
        }
        ObjectKind::StackedWasherDryer(wd) => {
            wd.washer = load_washer(r)?;
            wd.dryer = load_dryer(r)?;
        }
        ObjectKind::Radio(d) | ObjectKind::Television(d) => {
            if r.bool()? {
                load_device(r, d)?;
            }
        }
        ObjectKind::Thumpable(t) => load_thumpable(r, ctx, wv, t, &mut o.key_id)?,
        ObjectKind::Mannequin(slot) => {
            let dir = r.i8()? & 7;
            let init = r.bool()?;
            let female = r.bool()?;
            let zombie = r.bool()?;
            let skeleton = r.bool()?;
            let mannequin_script_name = Some(r.string()?);
            let pose = Some(r.string()?);
            let human_visual = HumanVisual::load(r, ctx.tables.blood_body_parts)?;
            // The native getter resolves real skin lists/RNG when this name is
            // absent. The host performs that pending resolution; no dummy
            // style or random draw is introduced by the codec.
            let texture_name = human_visual.skin_texture_name.clone();
            let pending_loaded_skin_resolution = texture_name.is_none();
            let mut container = o.container.take().unwrap_or_else(|| {
                let mut container = ItemContainer::new(Some("mannequin"));
                container.explored = true;
                container
            });
            container.items.clear();
            let mut worn_items = Vec::new();
            if r.bool()? {
                let id = r.i32()?;
                let mut c = ItemContainer::new(None);
                let saved_items = c.load_saved(r, wv, ctx.items)?;
                c.id = id;
                container = c;
                let n = r.i8()?;
                for _ in 0..n.max(0) {
                    let location = r.string()?;
                    let index = r.i16()?;
                    if let Some(index) = usize::try_from(index)
                        .ok()
                        .and_then(|i| saved_items.get(i))
                        .copied()
                        .flatten()
                    {
                        worn_items.push((location, index as i16));
                    }
                }
            }
            *slot = Box::new(IsoMannequin {
                dir,
                init,
                female,
                zombie,
                skeleton,
                mannequin_script_name,
                pose,
                human_visual,
                texture_name,
                pending_loaded_skin_resolution,
                container: Some(container),
                worn_items,
                ..IsoMannequin::new(ctx.tables.blood_body_parts)
            });
        }
        _ => unreachable!("load_tail for {}", o.kind.class_name()),
    }
    if let ObjectKind::Television(_) = o.kind {
        o.overlay_sprite = None;
    }
    Ok(())
}

fn load_thumpable(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    t: &mut IsoThumpable,
    key_id: &mut i32,
) -> SaveResult<()> {
    let bits = super::io::read_header(r, 8)?;
    if bits != 0 {
        t.open = has(bits, 1);
        t.locked = has(bits, 2);
        t.north = has(bits, 4);
        if has(bits, 8) {
            t.max_health = r.i32()?;
        }
        t.health = if has(bits, 16) {
            r.i32()?
        } else {
            t.max_health
        };
        if has(bits, 32) {
            t.closed_sprite = sprite_or_null(ctx, r.i32()?);
        }
        if has(bits, 64) {
            t.open_sprite = sprite_or_null(ctx, r.i32()?);
        }
        if has(bits, 128) {
            t.thump_dmg = r.i32()?;
        }
        t.is_door = has(bits, 512);
        t.is_door_frame = has(bits, 1024);
        t.is_corner = has(bits, 2048);
        t.is_stairs = has(bits, 4096);
        t.is_container = has(bits, 8192);
        t.is_floor = has(bits, 16384);
        t.can_barricade = has(bits, 32768);
        t.can_pass_through = has(bits, 65536);
        t.dismantable = has(bits, 131072);
        t.can_be_plastered = has(bits, 262144);
        t.paintable = has(bits, 524288);
        if has(bits, 0x100000) {
            t.cross_speed = r.f32()?;
        }
        if has(bits, 0x200000) {
            t.table = Some(LuaTable::load(r, wv)?);
        }
        if has(bits, 0x400000) {
            t.mod_data = Some(LuaTable::load(r, wv)?);
        }
        t.block_all_the_square = has(bits, 0x800000);
        t.is_thumpable = has(bits, 0x1000000);
        t.is_hoppable = has(bits, 0x2000000);
        if has(bits, 0x4000000) {
            t.light_source_life = r.i32()?;
        }
        if has(bits, 0x8000000) {
            t.light_source_radius = r.i32()?;
        }
        if has(bits, 0x10000000) {
            t.light_source_x_offset = r.i32()?;
        }
        if has(bits, 0x20000000) {
            t.light_source_y_offset = r.i32()?;
        }
        if has(bits, 0x40000000) {
            let id = r.i16()?;
            // WorldDictionary.getItemTypeFromID: null for an unknown id.
            t.light_source_fuel = ctx.items.exists(id).then_some(id);
        }
        if has(bits, 0x80000000) {
            t.life_delta = r.f32()?;
        }
        if has(bits, 0x100000000) {
            t.life_left = r.f32()?;
        }
        if has(bits, 0x200000000) {
            // The IsoObject keyId field.
            *key_id = r.i32()?;
        }
        t.locked_by_key = has(bits, 0x400000000);
        t.locked_by_padlock = has(bits, 0x800000000);
        t.can_be_lock_by_padlock = has(bits, 0x1000000000);
        if has(bits, 0x2000000000) {
            t.locked_by_code = r.i32()?;
        }
        if has(bits, 0x4000000000) {
            let s = r.string()?;
            t.thump_sound = match s.as_str() {
                "thumpa2" => "ZombieThumpGeneric".into(),
                "metalthump" => "ZombieThumpMetal".into(),
                _ => s,
            };
        }
        if has(bits, 0x8000000000) {
            t.last_update_hours = r.f32()?;
        }
        if has(bits, 0x10000000000) {
            t.have_fuel = true;
        }
        if has(bits, 0x20000000000) {
            t.light_source_on = true;
        }
        if has(bits, 0x40000000000) {
            t.build_materials = Some(LuaTable::load(r, wv)?);
        }
    }
    if wv < 248 {
        if let Some(md) = &mut t.mod_data {
            let mut migrated = Vec::new();
            for (k, v) in &md.entries {
                if let LuaKey::Str(s) = k {
                    if let Some(rest) = s.strip_prefix("need:") {
                        migrated.push((s.clone(), rest.to_string(), v.clone()));
                    }
                }
            }
            for (key, rest, v) in migrated {
                t.build_materials
                    .get_or_insert_with(LuaTable::default)
                    .rawset(LuaKey::Str(rest), Some(v));
                md.rawset(LuaKey::Str(key), None);
            }
        }
    }
    Ok(())
}

/// `IsoThumpable.save`. `key_id` is the `IsoObject.keyId` field.
fn save_thumpable(w: &mut Writer, ctx: &SaveCtx<'_>, t: &IsoThumpable, key_id: i32) {
    let mut bits = BitHeaderWrite::alloc(w, 8);
    let mut flag = |b: bool, f: u64| {
        if b {
            bits.add(f);
        }
    };
    flag(t.open, 1);
    flag(t.locked, 2);
    flag(t.north, 4);
    if t.max_health != 500 {
        bits.add(8);
        w.i32(t.max_health);
    }
    if t.health != t.max_health {
        bits.add(16);
        w.i32(t.health);
    }
    if let Some(s) = t.closed_sprite {
        bits.add(32);
        w.i32(ctx.sprite_id(s));
    }
    if let Some(s) = t.open_sprite {
        bits.add(64);
        w.i32(ctx.sprite_id(s));
    }
    if t.thump_dmg != 8 {
        bits.add(128);
        w.i32(t.thump_dmg);
    }
    for (b, f) in [
        (t.is_door, 512),
        (t.is_door_frame, 1024),
        (t.is_corner, 2048),
        (t.is_stairs, 4096),
        (t.is_container, 8192),
        (t.is_floor, 16384),
        (t.can_barricade, 32768),
        (t.can_pass_through, 65536),
        (t.dismantable, 131072),
        (t.can_be_plastered, 262144),
        (t.paintable, 524288),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.cross_speed != 1.0 {
        bits.add(0x100000);
        w.f32(t.cross_speed);
    }
    if let Some(tb) = present(&t.table) {
        bits.add(0x200000);
        tb.save(w);
    }
    if let Some(tb) = present(&t.mod_data) {
        bits.add(0x400000);
        tb.save(w);
    }
    for (b, f) in [
        (t.block_all_the_square, 0x800000),
        (t.is_thumpable, 0x1000000),
        (t.is_hoppable, 0x2000000),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.light_source_life != -1 {
        bits.add(0x4000000);
        w.i32(t.light_source_life);
    }
    if t.light_source_radius != -1 {
        bits.add(0x8000000);
        w.i32(t.light_source_radius);
    }
    if t.light_source_x_offset != 0 {
        bits.add(0x10000000);
        w.i32(t.light_source_x_offset);
    }
    if t.light_source_y_offset != 0 {
        bits.add(0x20000000);
        w.i32(t.light_source_y_offset);
    }
    if let Some(f) = t.light_source_fuel {
        bits.add(0x40000000);
        w.i16(f);
    }
    if t.life_delta != 0.0 {
        bits.add(0x80000000);
        w.f32(t.life_delta);
    }
    if t.life_left != -1.0 {
        bits.add(0x100000000);
        w.f32(t.life_left);
    }
    if key_id != -1 {
        bits.add(0x200000000);
        w.i32(key_id);
    }
    for (b, f) in [
        (t.locked_by_key, 0x400000000),
        (t.locked_by_padlock, 0x800000000),
        (t.can_be_lock_by_padlock, 0x1000000000),
    ] {
        if b {
            bits.add(f);
        }
    }
    if t.locked_by_code != 0 {
        bits.add(0x2000000000);
        w.i32(t.locked_by_code);
    }
    // Compared with "ZombieThumbGeneric", which no sound is called: every
    // thumpable writes its sound.
    if t.thump_sound != "ZombieThumbGeneric" {
        bits.add(0x4000000000);
        w.string(&t.thump_sound);
    }
    if t.last_update_hours != -1.0 {
        bits.add(0x8000000000);
        w.f32(t.last_update_hours);
    }
    if t.have_fuel {
        bits.add(0x10000000000);
    }
    if t.light_source_on {
        bits.add(0x20000000000);
    }
    if let Some(tb) = present(&t.build_materials) {
        bits.add(0x40000000000);
        tb.save(w);
    }
    bits.write(w);
}

fn opt_item(r: &mut Reader<'_>, ctx: &SaveCtx<'_>, wv: i32) -> SaveResult<Option<ItemBlob>> {
    ItemBlob::load(r, wv, ctx.items)
}

/// `IsoHutch.addAnimalInside(animal, false)` while loading: a taken place
/// (or one a nest box animal prefers) draws a new one with `Rand`.
fn add_animal_inside(
    animals: &mut Vec<(i32, IsoAnimal)>,
    nest_boxes: &[NestBox],
    mut a: IsoAnimal,
    ctx: &SaveCtx<'_>,
    sprite_name: &str,
) -> SaveResult<()> {
    let taken = |p: i32, animals: &[(i32, IsoAnimal)]| {
        animals.iter().any(|(k, _)| *k == p)
            || nest_boxes
                .iter()
                .filter_map(|n| n.animal.as_ref())
                .any(|n| n.preferred_hutch_position == p)
    };
    let draw = |a: &mut IsoAnimal| -> SaveResult<()> {
        let Some(max) = ctx.hooks.hutch_max_animals(sprite_name) else {
            return err("hutch maxAnimals unknown");
        };
        a.preferred_hutch_position = ctx.hooks.rand_next(0, max);
        Ok(())
    };
    if a.preferred_hutch_position == -1 {
        draw(&mut a)?;
    }
    let mut tries = 0;
    while taken(a.preferred_hutch_position, animals) {
        tries += 1;
        if tries > 100 {
            break;
        }
        draw(&mut a)?;
    }
    let p = a.preferred_hutch_position;
    if animals.iter().any(|(k, _)| *k == p) {
        // No place: the animal is dropped.
        return Ok(());
    }
    a.hutch_position = p;
    a.item_id = 0;
    animals.push((p, a));
    Ok(())
}

/// Classes whose state is built entirely in their `load`.
fn load_special(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    o: &mut IsoObject,
    name: &str,
) -> SaveResult<()> {
    match name {
        "Pushable" | "WheelieBin" => {
            let wheelie = name == "WheelieBin";
            let (p, sprite, container) = super::movers::IsoPushableObject::load(
                r,
                wv,
                wheelie,
                ctx.settings.tile_scale,
                ctx.items,
            )?;
            // IsoSpriteManager.getSprite(int): null for an unknown id. A
            // wheelie bin keeps the unregistered sprite CreateSprite made.
            match sprite {
                Some(id) => o.sprite = (sprite_or_null(ctx, id)).into(),
                None => o.sprite = ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID),
            }
            o.container = container;
            o.kind = ObjectKind::Pushable(Box::new(p));
            return Ok(());
        }
        "WorldInventoryItem" => {
            let xoff = r.f32()?;
            let yoff = r.f32()?;
            let zoff = r.f32()?;
            let _offset_x = r.f32()?;
            let _offset_y = r.f32()?;
            let rec = super::items::read_record(r)?;
            let exists = ctx.items.exists(i16::from_be_bytes([rec[0], rec[1]]));
            let item = if exists {
                ItemBlob::decode(rec, wv, ctx.items)
            } else {
                ItemBlob::Raw(rec.to_vec())
            };
            let drop_time = r.f64()?;
            let bits = super::io::read_header(r, 1)?;
            o.kind = ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
                IsoWorldInventoryObject {
                    xoff,
                    yoff,
                    zoff,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    item: crate::floor_item::FloorItem::from_saved(item),
                    drop_time,
                    ignore_remove_sandbox: exists && has(bits, 1),
                    extended_placement: exists && has(bits, 4),
                },
            ));
            if !exists {
                // The item is null: the square drops the object. The entity
                // block (if any) is not read, as upstream.
                if matches!(o.sprite, ObjectSprite::ArchiveUnmanaged(_)) {
                    o.sprite = ObjectSprite::None;
                }
                return Ok(());
            }
            if has(bits, 2) {
                load_entity_archive(r, ctx, wv, o)?;
            }
            o.sprite = ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID);
            return Ok(());
        }
        "Barricade" => {
            let dir = r.i8()? & 7;
            let n = r.i8()?;
            let mut plank_health = [0i32; 4];
            for i in 0..n.max(0) as usize {
                let h = r.i16()?;
                if let Some(p) = plank_health.get_mut(i) {
                    *p = h as i32;
                }
            }
            let b = IsoBarricade {
                dir,
                plank_health,
                metal_health: r.i16()? as i32,
                metal_bar_health: r.i16()? as i32,
            };
            // `chooseSprite()`. `getSprite(name)` adds a sprite for a name it
            // does not know; the tiles it picks are all defined. A diagonal
            // direction leaves the sprite null and the original's
            // `LoadFramesNoDirPageSimple` throws.
            let Some(n) = b.sprite_name() else {
                return err("NullPointerException: IsoBarricade.chooseSprite with no sprite");
            };
            o.sprite = (ctx.sprites.named_map.get(&n).copied()).into();
            o.kind = ObjectKind::Barricade(b);
            return Ok(());
        }
        _ => {}
    }
    load_base(r, ctx, wv, o)?;
    o.kind = match name {
        "IsoCompost" => {
            if let Some(c) = &mut o.container {
                c.kind = Some("composter".into());
            }
            let compost = r.f32()?;
            let last_updated = r.f32()?;
            let (health, max_health) = if wv >= 213 {
                (r.i32()?, r.i32()?)
            } else {
                return err("IsoCompost from world version < 213");
            };
            ObjectKind::Compost(IsoCompost {
                compost,
                last_updated,
                health,
                max_health,
            })
        }
        "FeedingTrough" => {
            let n = r.i32()?;
            let mut feeding_types: Vec<(String, f32)> = Vec::new();
            for _ in 0..n.max(0) {
                let k = r.string()?;
                let v = r.f32()?;
                match feeding_types.iter_mut().find(|(e, _)| *e == k) {
                    Some(e) => e.1 = v,
                    None => feeding_types.push((k, v)),
                }
            }
            let water = r.f32()?;
            let (mut linked_x, mut linked_y) = (0, 0);
            if r.bool()? {
                linked_x = r.i32()?;
                linked_y = r.i32()?;
                o.container = None;
            }
            let north = r.bool()?;
            // initWithDef: masters only.
            let (max_water, max_feed) = if linked_x > 0 && linked_y > 0 {
                (0.0, 0)
            } else {
                let sprite_data = o.sprite.metadata_projection(ctx.sprites).map_err(|error| {
                    super::io::SaveError(format!(
                        "archive feeding-trough sprite metadata: {error:?}"
                    ))
                })?;
                sprite_data
                    .as_ref()
                    .and_then(|s| s.name.as_deref())
                    .and_then(|n| ctx.hooks.feeding_trough_def(n))
                    .unwrap_or((0.0, 0))
            };
            ObjectKind::FeedingTrough(IsoFeedingTrough {
                feeding_types,
                water,
                linked_x,
                linked_y,
                north,
                max_water,
                max_feed,
            })
        }
        "IsoHutch" => {
            let linked_x = r.i32()?;
            let linked_y = r.i32()?;
            let linked_z = r.i32()?;
            let mut master = None;
            if !(linked_x > 0 && linked_y > 0) {
                if wv < 212 {
                    return err("IsoHutch from world version < 212");
                }
                let sprite_name = r.string()?;
                // this.sprite = IsoSpriteManager.instance.getSprite(spriteName)
                if let Some(&s) = ctx.sprites.named_map.get(&sprite_name) {
                    o.sprite = (Some(s)).into();
                    if matches!(o.sprite, ObjectSprite::ArchiveUnmanaged(_)) {
                        o.sprite = ObjectSprite::None;
                    }
                }
                let open = r.bool()?;
                let open_egg_hatch = r.bool()?;
                let saved = (r.i32()?, r.i32()?, r.i32()?);
                let size = r.i32()?;
                if size < 0 || (size as usize) > r.remaining() {
                    return err("hutch animal block length");
                }
                let mut loaded = Vec::new();
                let nb = r.i8()?;
                for _ in 0..nb.max(0) {
                    let _serialize = r.bool()?;
                    let _class_id = r.i8()?;
                    loaded.push(IsoAnimal::load(r, wv, ctx)?);
                }
                let hutch_dirt = r.f32()?;
                let nest_box_dirt = r.f32()?;
                let n = r.i8()?;
                let max = ctx.hooks.hutch_max_nest_box(&sprite_name);
                let mut nest_boxes = Vec::new();
                for i in 0..n.max(0) as i32 {
                    let ne = r.i8()?;
                    let mut eggs = Vec::new();
                    for _ in 0..ne.max(0) {
                        // Only Food items are kept.
                        if let Some(e) = opt_item(r, ctx, wv)? {
                            eggs.push(e);
                        }
                    }
                    let animal = if r.bool()? {
                        Some(IsoAnimal::load(r, wv, ctx)?)
                    } else {
                        None
                    };
                    if max.is_none_or(|m| i <= m) {
                        nest_boxes.push(NestBox { eggs, animal });
                    }
                }
                let mut animals: Vec<(i32, IsoAnimal)> = Vec::new();
                for a in loaded {
                    add_animal_inside(&mut animals, &nest_boxes, a, ctx, &sprite_name)?;
                }
                master = Some(HutchMaster {
                    sprite_name,
                    open,
                    open_egg_hatch,
                    saved_x: saved.0,
                    saved_y: saved.1,
                    saved_z: saved.2,
                    animals,
                    hutch_dirt,
                    nest_box_dirt,
                    nest_boxes,
                });
            }
            ObjectKind::Hutch(Box::new(IsoHutch {
                linked_x,
                linked_y,
                linked_z,
                master,
            }))
        }
        "IsoGenerator" => {
            let md = o.mod_data.get_or_insert_with(LuaTable::default);
            let total_power_using = match md.rawget(&LuaKey::Str("totalPowerDraw".into())) {
                Some(LuaValue::Num(d)) => *d as f32,
                _ => 0.0,
            };
            ObjectKind::Generator(IsoGenerator {
                connected: r.bool()?,
                activated: r.bool()?,
                fuel: r.f32()?.min(10.0),
                condition: r.i32()?,
                last_hour: r.i32()?,
                total_power_using,
            })
        }
        "IsoTrap" => {
            let mut t = IsoTrap {
                sensor_range: r.i32()?,
                fire_starting_chance: r.i32()?,
                fire_starting_energy: r.i32()?,
                fire_range: r.i32()?,
                explosion_power: r.i32()?,
                explosion_range: r.i32()?,
                explosion_duration: 0,
                explosion_start_time: 0.0,
                smoke_range: 0,
                noise_range: 0,
                noise_duration: 0,
                noise_start_time: 0.0,
                extra_damage: 0.0,
                remote_control_id: 0,
                timer_before_explosion: 0,
                count_down_sound: String::new(),
                explosion_sound: String::new(),
                weapon: None,
            };
            if wv >= 219 {
                t.explosion_duration = r.i32()?;
                t.explosion_start_time = r.f32()?;
            }
            t.smoke_range = r.i32()?;
            t.noise_range = r.i32()?;
            t.noise_duration = r.i32()?;
            t.noise_start_time = r.f32()?;
            t.extra_damage = r.f32()?;
            t.remote_control_id = r.i32()?;
            t.timer_before_explosion = r.i32()?;
            t.count_down_sound = r.string()?;
            let s = r.string()?;
            t.explosion_sound = match s.as_str() {
                "bigExplosion" => "BigExplosion".into(),
                "smallExplosion" => "SmallExplosion".into(),
                "feedback" => "NoiseTrapExplosion".into(),
                _ => s,
            };
            if r.bool()? {
                t.weapon = opt_item(r, ctx, wv)?;
            }
            ObjectKind::Trap(Box::new(t))
        }
        "IsoCarBatteryCharger" => {
            let item = if r.bool()? {
                opt_item(r, ctx, wv)?
            } else {
                None
            };
            let battery = if r.bool()? {
                opt_item(r, ctx, wv)?
            } else {
                None
            };
            ObjectKind::CarBatteryCharger(Box::new(IsoCarBatteryCharger {
                item,
                battery,
                activated: r.bool()?,
                last_update: r.f32()?,
                charge_rate: r.f32()?,
            }))
        }
        "Fire" => {
            o.sprite = (None).into();
            if matches!(o.sprite, ObjectSprite::ArchiveUnmanaged(_)) {
                o.sprite = ObjectSprite::None;
            }
            let mut life = || r.i32();
            let mut s = crate::fire::FireSave {
                life: life()?,
                spread_delay: life()?,
                life_stage: life()?,
                life_stage_timer: life()?,
                life_stage_duration: life()?,
                energy: life()?,
                num_flame_particles: life()?,
                spread_timer: life()?,
                age: life()?,
                ..Default::default()
            };
            s.perm = r.bool()?;
            s.light_radius = (r.i8()? as u8) as i32;
            s.smoke = r.bool()?;
            if wv >= 242 {
                s.saved_update_time = Some(r.i64()?);
            }
            // The animations IsoFire.load attaches draw from Rand; chunk
            // loading leaves them to the fire's owner
            // (IsoFire::attach_loaded_anims).
            let f = IsoFire::from_save(&s);
            ObjectKind::Fire(f)
        }
        _ => unreachable!(),
    };
    Ok(())
}

/// `IsoObject.save` of `o` with its class's override.
pub fn save_object(w: &mut Writer, ctx: &SaveCtx<'_>, o: &IsoObject) -> SaveResult<()> {
    match &o.kind {
        ObjectKind::WorldInventory(wi) => {
            let wi = wi.borrow();
            w.i8(1);
            w.i8(ctx.class_id("WorldInventoryItem")?);
            w.f32(wi.xoff);
            w.f32(wi.yoff);
            w.f32(wi.zoff);
            w.f32(wi.offset_x);
            w.f32(wi.offset_y);
            wi.item.save(w, ctx.items)?;
            w.f64(wi.drop_time);
            let mut bits = BitHeaderWrite::alloc(w, 1);
            if wi.ignore_remove_sandbox {
                bits.add(1);
            }
            if o.has_components || !o.components.is_empty() {
                bits.add(2);
                save_entity(w, o)?;
            }
            if wi.extended_placement {
                bits.add(4);
            }
            bits.write(w);
            return Ok(());
        }
        ObjectKind::Pushable(p) => {
            let class = ctx.class_id(p.object_name())?;
            let sprite = if o.sprite.is_none() {
                None
            } else {
                Some(try_sprite_save_id(ctx, o).map_err(|error| {
                    super::io::SaveError(format!("archive pushable sprite field: {error:?}"))
                })?)
            };
            return p.save(w, class, sprite, o.container.as_ref(), ctx.items);
        }
        ObjectKind::Barricade(b) => {
            w.i8(1);
            w.i8(ctx.class_id("Barricade")?);
            w.i8(b.dir);
            w.i8(4);
            for &h in &b.plank_health {
                w.i16(h as i16);
            }
            w.i16(b.metal_health as i16);
            w.i16(b.metal_bar_health as i16);
            return Ok(());
        }
        _ => {}
    }
    let generator_md;
    let mut mod_data = o.mod_data.as_ref();
    let mut with_containers = true;
    let mut with_attached = true;
    match &o.kind {
        ObjectKind::Mannequin(_) => with_containers = false,
        ObjectKind::Fire(_) => with_attached = false,
        ObjectKind::Generator(_) => {
            // getModData().rawset("totalPowerDraw", (Object) float): a Float
            // the table cannot save, which still makes it non-empty.
            let mut md = o.mod_data.clone().unwrap_or_default();
            md.rawset(
                LuaKey::Str("totalPowerDraw".into()),
                Some(LuaValue::Unsaveable),
            );
            generator_md = md;
            mod_data = Some(&generator_md);
        }
        _ => {}
    }
    save_base(w, ctx, o, with_containers, with_attached, mod_data)?;
    match &o.kind {
        ObjectKind::Object
        | ObjectKind::Jukebox
        | ObjectKind::BrokenGlass
        | ObjectKind::ButcherHook
        | ObjectKind::Pushable(_) => {}
        ObjectKind::AnimalTrack(t) => t.save(w),
        ObjectKind::Door(d) => {
            w.bool(d.open);
            w.bool(d.locked);
            w.bool(d.north);
            w.i32(d.health);
            w.i32(d.max_health);
            w.i32(sprite_id_npe(ctx, d.closed_sprite, "closedSprite")?);
            w.i32(sprite_id_npe(ctx, d.open_sprite, "openSprite")?);
            w.i32(o.key_id);
            w.bool(d.locked_by_key);
            let mut b = 0i8;
            if d.has_curtain {
                b |= 1;
                if d.curtain_open {
                    b |= 2;
                }
                if d.curtain_inside {
                    b |= 4;
                }
            }
            w.i8(b);
        }
        ObjectKind::Window(win) => {
            w.bool(win.open);
            w.bool(win.north);
            w.i32(win.health);
            w.bool(win.locked);
            w.bool(win.perma_locked);
            w.bool(win.destroyed);
            w.bool(win.glass_removed);
            save_opt_sprite(w, ctx, win.open_sprite);
            save_opt_sprite(w, ctx, win.closed_sprite);
            save_opt_sprite(w, ctx, win.smashed_sprite);
            save_opt_sprite(w, ctx, win.glass_removed_sprite);
            w.i32(win.max_health);
        }
        ObjectKind::Curtain(c) => {
            w.bool(c.open);
            w.bool(c.north);
            w.i32(c.health);
            w.i32(c.barricade_strength);
            if c.open {
                w.i32(sprite_id_npe(ctx, c.closed_sprite, "closedSprite")?);
            } else {
                w.i32(sprite_id_npe(ctx, c.open_sprite, "openSprite")?);
            }
        }
        ObjectKind::LightSwitch(s) => {
            w.bool(s.light_room);
            w.i64(s.room_id);
            w.bool(s.activated);
            w.bool(s.can_be_modified);
            if s.can_be_modified {
                w.bool(s.use_battery);
                w.bool(s.has_battery);
                w.bool(s.bulb_item.is_some());
                if let Some(b) = &s.bulb_item {
                    w.string(b);
                }
                w.f32(s.power);
                w.f32(s.delta);
                w.f32(s.primary_r);
                w.f32(s.primary_g);
                w.f32(s.primary_b);
            }
            w.i64(s.last_minute_stamp);
            w.i32(s.bulb_burn_minutes);
        }
        ObjectKind::Tree(t) => {
            w.i8(t.log_yield as i8);
            w.i8((t.damage / 10) as i8);
        }
        ObjectKind::WindowFrame { north } => w.bool(*north),
        ObjectKind::Barbecue(b) => {
            w.bool(b.has_propane_tank);
            w.i32(b.fuel_amount_minutes);
            w.bool(b.lit);
            w.f32(b.last_update_time);
            w.i32(b.minutes_since_extinguished);
            save_opt_sprite(w, ctx, b.normal_sprite);
            save_opt_sprite(w, ctx, b.no_tank_sprite);
        }
        ObjectKind::Fireplace(f) => {
            w.i32(f.fuel_amount_minutes);
            w.bool(f.lit);
            w.f32(f.last_update_time);
            w.i32(f.minutes_since_extinguished);
        }
        ObjectKind::Stove(s) => {
            w.bool(s.activated);
            w.i32(s.seconds_timer);
            w.f32(s.max_temperature);
            w.bool(s.first_turn_on);
            w.bool(s.broken);
        }
        ObjectKind::ClothingDryer(d) => w.bool(d.activated),
        ObjectKind::ClothingWasher(wl) => {
            w.bool(wl.activated);
            w.f32(wl.last_update);
        }
        ObjectKind::CombinationWasherDryer(wd) => {
            w.bool(wd.dryer_mode);
            w.bool(wd.washer.activated);
            w.f32(wd.washer.last_update);
            w.bool(wd.dryer.activated);
        }
        ObjectKind::StackedWasherDryer(wd) => {
            w.bool(wd.washer.activated);
            w.f32(wd.washer.last_update);
            w.bool(wd.dryer.activated);
        }
        ObjectKind::Radio(d) | ObjectKind::Television(d) => {
            w.i8(1);
            save_device(w, d);
        }
        ObjectKind::Thumpable(t) => save_thumpable(w, ctx, t, o.key_id),
        ObjectKind::Mannequin(m) => {
            w.i8(m.dir);
            w.bool(m.init);
            w.bool(m.female);
            w.bool(m.zombie);
            w.bool(m.skeleton);
            w.string(m.mannequin_script_name.as_deref().unwrap_or(""));
            w.string(m.pose.as_deref().unwrap_or(""));
            m.human_visual.save(w);
            match m.save_inventory_snapshot()? {
                Some((c, worn)) => {
                    w.i8(1);
                    w.i32(c.id);
                    let saved_items = c.save_saved(w, ctx.items, None)?;
                    if worn.len() > i8::MAX as usize {
                        return err("too many worn items");
                    }
                    w.i8(worn.len() as i8);
                    for (loc, index) in &worn {
                        w.string(loc);
                        let saved_index = usize::try_from(*index)
                            .ok()
                            .and_then(|i| saved_items.iter().position(|&entry| entry == i));
                        w.i16(saved_index.map_or(-1, |i| i as i16));
                    }
                }
                None => w.i8(0),
            }
        }
        ObjectKind::Compost(c) => {
            w.f32(c.compost);
            w.f32(c.last_updated);
            w.i32(c.health);
            w.i32(c.max_health);
        }
        ObjectKind::FeedingTrough(t) => {
            w.i32(t.feeding_types.len() as i32);
            let keys: Vec<&str> = t.feeding_types.iter().map(|(k, _)| k.as_str()).collect();
            for k in crate::java::hashmap_order(&keys) {
                let v = t.feeding_types.iter().find(|(e, _)| e == k).unwrap().1;
                w.string(k);
                w.f32(v);
            }
            w.f32(t.water);
            if t.linked_x > 0 && t.linked_y > 0 {
                w.i8(1);
                w.i32(t.linked_x);
                w.i32(t.linked_y);
            } else {
                w.i8(0);
            }
            w.bool(t.north);
        }
        ObjectKind::Hutch(h) => {
            w.i32(h.linked_x);
            w.i32(h.linked_y);
            w.i32(h.linked_z);
            if !(h.linked_x > 0 && h.linked_y > 0) {
                let Some(m) = &h.master else {
                    return err("IsoHutch master without state");
                };
                w.string(&m.sprite_name);
                w.bool(m.open);
                w.bool(m.open_egg_hatch);
                w.i32(m.saved_x);
                w.i32(m.saved_y);
                w.i32(m.saved_z);
                let len_at = w.pos();
                w.i32(0);
                let start = w.pos();
                let mut inside: Vec<&(i32, IsoAnimal)> = m.animals.iter().collect();
                pz_inventory::java::sort_int_hash_map(&mut inside, |e| e.0);
                w.i8(inside.len() as i8);
                for (_, a) in inside {
                    a.save(w, ctx, true)?;
                }
                let end = w.pos();
                w.patch_i32(len_at, (end - start) as i32);
                w.f32(m.hutch_dirt);
                w.f32(m.nest_box_dirt);
                w.i8(m.nest_boxes.len() as i8);
                for nb in &m.nest_boxes {
                    w.i8(nb.eggs.len() as i8);
                    for e in &nb.eggs {
                        e.save(w, ctx.items)?;
                    }
                    match &nb.animal {
                        Some(a) => {
                            w.i8(1);
                            a.save(w, ctx, false)?;
                        }
                        None => w.i8(0),
                    }
                }
            }
        }
        ObjectKind::Generator(g) => {
            w.bool(g.connected);
            w.bool(g.activated);
            w.f32(g.fuel);
            w.i32(g.condition);
            w.i32(g.last_hour);
        }
        ObjectKind::Trap(t) => {
            w.i32(t.sensor_range);
            w.i32(t.fire_starting_chance);
            w.i32(t.fire_starting_energy);
            w.i32(t.fire_range);
            w.i32(t.explosion_power);
            w.i32(t.explosion_range);
            w.i32(t.explosion_duration);
            w.f32(t.explosion_start_time);
            w.i32(t.smoke_range);
            w.i32(t.noise_range);
            w.i32(t.noise_duration);
            w.f32(t.noise_start_time);
            w.f32(t.extra_damage);
            w.i32(t.remote_control_id);
            w.i32(t.timer_before_explosion);
            w.string(&t.count_down_sound);
            w.string(&t.explosion_sound);
            match &t.weapon {
                Some(i) => {
                    w.i8(1);
                    i.save(w, ctx.items)?;
                }
                None => w.i8(0),
            }
        }
        ObjectKind::CarBatteryCharger(c) => {
            for i in [&c.item, &c.battery] {
                match i {
                    Some(i) => {
                        w.i8(1);
                        i.save(w, ctx.items)?;
                    }
                    None => w.i8(0),
                }
            }
            w.bool(c.activated);
            w.f32(c.last_update);
            w.f32(c.charge_rate);
        }
        ObjectKind::Fire(f) => {
            w.i32(f.life);
            w.i32(f.spread_delay);
            w.i32(f.life_stage - 1);
            w.i32(f.life_stage_timer);
            w.i32(f.life_stage_duration);
            w.i32(f.energy);
            w.i32(f.num_flame_particles);
            w.i32(f.spread_timer);
            w.i32(f.age);
            w.bool(f.perm);
            w.i8(f.light_radius as i8);
            w.bool(f.smoke);
            w.i64(ctx.settings.calendar_millis);
        }
        ObjectKind::WorldInventory(_) | ObjectKind::Barricade(_) => unreachable!(),
    }
    Ok(())
}
