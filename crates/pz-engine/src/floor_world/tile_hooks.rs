//! Runtime services used by Lua's IsoGridSquare tile placement and removal.
use super::*;
use pz_world::process_items::ProcessItemQueues;
use pz_world::process_objects::ProcessObjects;
use pz_world::square_mutation::{
    try_flag_object_for_hot_save, ObjectRef, SquareMutationHooks, WorldCall,
};
use pz_world::{GameRand, IsoObjectRef, ObjectKind};

pub(super) struct TileHooks {
    pub objects: Rc<RefCell<ProcessObjects>>,
    pub items: ProcessItemQueues,
}

impl SquareMutationHooks for TileHooks {
    fn world_call(
        &mut self,
        _: ObjectRef,
        obj: IsoObjectRef,
        _: &mut dyn GameRand,
        call: WorldCall,
    ) {
        // addToWorld records these registrations before the subclass Lua
        // continuation. A removal can run reentrantly from that continuation.
        match call {
            WorldCall::ProcessIsoObjectAdd => self.objects.borrow_mut().add(&obj),
            WorldCall::StaticUpdater { add } => self.objects.borrow_mut().static_updater(&obj, add),
            _ => {}
        }
    }

    fn try_world_call(
        &mut self,
        _: ObjectRef,
        obj: IsoObjectRef,
        _: &mut dyn GameRand,
        call: WorldCall,
    ) -> HostResult<()> {
        match call {
            WorldCall::ProcessIsoObjectRemove => self.objects.borrow_mut().queue_remove(&obj),
            WorldCall::StaticUpdater { add } => self.objects.borrow_mut().static_updater(&obj, add),
            WorldCall::FlagForHotSave => return try_flag_object_for_hot_save(&obj),
            WorldCall::ProcessItems {
                container,
                add: false,
            } => {
                let body = obj
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("tile inventories"))?;
                if let Some(runtime) = &body.runtime_containers {
                    let c = runtime
                        .iter()
                        .nth(container)
                        .ok_or_else(|| unsupported("tile container index"))?;
                    let c = c
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("tile container items"))?;
                    self.items.queue_remove_all(c.items.iter().cloned());
                    if c.kind != "floor" {
                        pz_lua_api::item_sound::remove_items(c.items.iter().cloned());
                    }
                } else if !body
                    .containers()
                    .nth(container)
                    .is_some_and(|c| c.items.is_empty())
                {
                    return Err(unsupported("removing unadopted map inventory"));
                }
            }
            WorldCall::UnregisterEntity => {
                if obj
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("map entity registration"))?
                    .added_to_entity_manager
                {
                    return Err(unsupported("removing a registered map entity"));
                }
            }
            // The render highlight/outline pass reads the geometry snapshot.
            // Its retained square is refreshed as soon as detach finishes.
            WorldCall::UnregisterHighlight => {}
            // Ordinary tiles have no private sound emitter, lamppost or
            // furnace/switch power overlay. Subclasses have their own owners.
            WorldCall::StopEmitter
            | WorldCall::RemoveAmbientEmitters
            | WorldCall::ClearOnOverlay
            | WorldCall::LightSource { add: false } => {
                if !matches!(
                    obj.try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("map tile resources"))?
                        .kind,
                    ObjectKind::Object
                ) {
                    return Err(unsupported("subclass tile audio or lighting removal"));
                }
            }
            _ => return Err(unsupported("subclass map tile removal service")),
        }
        Ok(())
    }
}

// Gate the cases that need an independent audio or lighting owner before
// changing any memberships. Ordinary authored furniture needs none of them.
pub(super) fn check_tile_resources(
    object: &IsoObjectRef,
    sprites: &IsoSpriteManager,
) -> HostResult<()> {
    let body = object
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("tile render resources"))?;
    if !matches!(body.kind, ObjectKind::Object) {
        return Ok(());
    }
    let Some(sprite) = body.sprite.try_read(sprites)? else {
        return Ok(());
    };
    let flag = |name| {
        sprites
            .tables
            .flag_from_string(name)
            .is_some_and(|f| sprite.properties.has(f))
    };
    let tent = sprite
        .name
        .as_deref()
        .is_some_and(|name| name.starts_with("camping_01"))
        && matches!(sprite.tile_sheet_index, 0 | 3);
    if sprites.has_prop(&sprite, "AmbientSound")
        || flag("waterPiped")
        || tent
        || body
            .container_types()
            .iter()
            .flatten()
            .any(|k| k == "fridge" || k == "freezer")
    {
        return Err(unsupported("removing an ambient sound or powered map tile"));
    }
    if flag("HasLightOnSprite")
        || ["RedLight", "GreenLight", "BlueLight"]
            .iter()
            .all(|name| sprites.has_prop(&sprite, name))
    {
        return Err(unsupported("removing a light-emitting map tile"));
    }
    Ok(())
}
