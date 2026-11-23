//! GameEntityFactory.CreateIsoObjectEntity for scripted map furniture.
use super::*;
use crate::bind_scripts::{handle_arg, ScriptHandle};
use pz_entity::factory::{create_from_script, EntityKind, FactoryInput, FactoryRng};
use pz_entity::{ComponentType, RuntimeEntryView};
use pz_scripts::objects::entity::GameEntityScript;

struct StandardRandom;
impl FactoryRng for StandardRandom {
    fn next_int(&mut self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
}

fn create(obj: &IsoObjectRef, handle: ScriptHandle) -> LuaResult<()> {
    let script = handle
        .root
        .downcast::<GameEntityScript>()
        .ok_or_else(|| unsupported("non-map entity script"))?;
    let entity = script
        .try_borrow()
        .map_err(|_| borrowed("GameEntityScript"))?;
    let full = entity
        .full_name()
        .map_err(|e| LuaError::runtime(e.to_string()))?;
    let env = crate::bind_items::item_env();
    let ctx = env
        .entity()
        .ok_or_else(|| unsupported("map entity registries"))?;
    let master = {
        let body = obj.try_borrow().map_err(|_| borrowed("map entity"))?;
        if !body.components.is_empty() {
            return Err(LuaError::runtime(
                "Calling CreateEntity on entity that already has components",
            ));
        }
        let manager = crate::bind_sprites::manager()?;
        let sprites = manager.try_borrow().map_err(|_| borrowed("map sprites"))?;
        let sprite = body.sprite.try_read(&sprites).map_err(host_error)?;
        sprites.is_multi_square_master(
            entity.base.script_object_name(),
            sprite.as_ref().and_then(|s| s.name.as_deref()),
        )
    };
    // The furniture component classes have inherited empty onFirstCreation.
    // Classes with additional owner, connection or initialization effects
    // need their actual hooks before reaching this path.
    for entry in entity.component_scripts() {
        if !matches!(
            entry.component_type(),
            ComponentType::SpriteConfig
                | ComponentType::FluidContainer
                | ComponentType::ContextMenuConfig
                | ComponentType::CraftRecipe
                | ComponentType::UiConfig
        ) {
            return Err(unsupported(format!(
                "map factory lifecycle for {:?}",
                entry.component_type()
            )));
        }
    }
    let dictionary = crate::bind_world::try_with_world_read(|w| Ok(w.script_dictionary.clone()))
        .map_err(host_error)?;
    let lookup = |reg, name: &str| {
        let id = dictionary.id_of(reg, name)?;
        Some((id, dictionary.get(reg, id)?.script_version))
    };
    let values = create_from_script(
        &entity,
        &full,
        EntityKind::IsoObject {
            multi_square_master: master,
        },
        &mut FactoryInput {
            ctx,
            rng: &mut StandardRandom,
            dictionary: &lookup,
        },
    );
    drop(entity);
    let mut components = pz_inventory::item_components::ItemComponents::from_values_unowned(values)
        .map_err(|e| runtime_error(e.error))?;
    let owner = EntityOwner::from_ref(obj);
    let script_owner = EntityOwner::from_ref(&script);
    crate::component_gc::register_entity_script(&script)
        .map_err(|e| LuaError::runtime(format!("map entity script: {e:?}")))?;
    for entry in components.entry_slots() {
        let RuntimeEntryView::Live { kind, component } = entry else {
            unreachable!("factory produced an encoded component")
        };
        let mut body = component.borrow_mut();
        match kind {
            ComponentType::Script => {
                drop(
                    body.set_original_script_actual(false, full.clone(), &script_owner)
                        .map_err(runtime_error)?,
                );
            }
            ComponentType::FluidContainer => {
                body.initialize_fluid_data(ctx).map_err(runtime_error)?
            }
            _ => {}
        }
        drop(body.replace_owner(Some(&owner)));
    }
    components.mark_bound();
    {
        let mut body = obj
            .try_borrow_mut()
            .map_err(|_| borrowed("map entity components"))?;
        body.has_components = !components.is_empty();
        body.components = components;
    }
    pz_world::object_slot::observe_object(obj, class_of(obj)?);
    Ok(())
}

pub(super) fn register(thread: &mut Thread) {
    let class = crate::expose::expose_class(
        thread,
        "zombie.entity.GameEntityFactory",
        Some("java.lang.Object"),
    );
    class.static_fn("CreateIsoObjectEntity", |c| {
        let target = c.arg(0);
        let object = object_from_value(&target).ok_or_else(|| unsupported("expected IsoObject"))?;
        let script = handle_arg(c, 1)?.ok_or_else(|| unsupported("expected entity script"))?;
        create(&object.object, script)?;
        Ok(0)
    });
}
