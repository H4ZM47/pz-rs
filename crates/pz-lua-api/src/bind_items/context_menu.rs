//! The `GameEntity`, `InventoryItem` and `HandWeapon` getters that
//! `ISInventoryPaneContextMenu.createMenu` reads for each selected item.
//! Instance getters read the fields that `Item.InstanceItem` sets on the
//! item. The getters that read through the item's retained `scriptItem`
//! see later script changes, and a null script fails like the original.

use pz_entity::ComponentType;
use pz_inventory::item::{ItemKind, WeaponData};
use pz_inventory::template::ScriptItemRef;
use pz_inventory::InventoryItem;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::items::Item;

use super::{this, GAME_ENTITY_CLASS, INVENTORY_ITEM_CLASS};
use crate::bind_scripts::{opt_string_list, COMPONENT_TYPE_CLASS};
use crate::bind_world_object::{runtime_error, unsupported};
use crate::item_tooltip::components;
use crate::expose::{expose_class, JavaEnum};

const FOOD_CLASS: &str = "zombie.inventory.types.Food";
const HAND_WEAPON_CLASS: &str = "zombie.inventory.types.HandWeapon";

fn null_pointer() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

/// The item's retained `scriptItem`.
fn script(c: &NativeCall<'_>) -> LuaResult<ScriptItemRef> {
    this(c)?
        .borrow()
        .template
        .script
        .clone()
        .ok_or_else(null_pointer)
}

/// A `ComponentType` argument; any other value fails the cast.
fn component_type(value: &Value) -> LuaResult<ComponentType> {
    if let Value::Object(o) = value {
        if let Some(e) = o.as_any().downcast_ref::<JavaEnum>() {
            if e.class == COMPONENT_TYPE_CLASS {
                if let Ok(kind) = ComponentType::value_of(e.name) {
                    return Ok(kind);
                }
            }
        }
    }
    Err(crate::bind_java_util::class_cast(value, COMPONENT_TYPE_CLASS))
}

pub(super) fn register(thread: &mut Thread) {
    let entity = expose_class(thread, GAME_ENTITY_CLASS, None);
    // `hasComponent(type)`: false for a null type.
    entity.method("hasComponent", |c| {
        let has = match c.arg(1) {
            Value::Nil => false,
            value => crate::bind_world_object::has_component(&c.arg(0), component_type(&value)?)?,
        };
        Ok(c.push(has))
    });

    let item = expose_class(thread, INVENTORY_ITEM_CLASS, Some(GAME_ENTITY_CLASS));
    type Getter = fn(&InventoryItem) -> Value;
    for (name, get) in [
        (
            "isRequiresEquippedBothHands",
            (|i| Value::from(i.requires_equipped_both_hands)) as Getter,
        ),
        // `HandWeapon` asks its active light part first.
        ("canBeActivated", |i| Value::from(i.can_be_activated())),
        ("getCustomMenuOption", |i| {
            i.custom_menu_option
                .as_deref()
                .map(Value::from)
                .unwrap_or_default()
        }),
        ("isRemoteController", |i| Value::from(i.remote_controller)),
        ("canBeRemote", |i| Value::from(i.can_be_remote)),
        ("getRemoteControlID", |i| Value::from(i.remote_control_id)),
        ("haveExtraItems", |i| {
            Value::from(i.extra_items.as_ref().is_some_and(|l| !l.is_empty()))
        }),
        ("isRecordedMedia", |i| Value::from(i.recorded_media_index >= 0)),
        ("isWaterSource", |i| {
            Value::from(crate::bind_item_container::is_water_source(i))
        }),
    ] {
        item.method(name, move |c| {
            let v = get(&this(c)?.borrow());
            Ok(c.push(v))
        });
    }

    type ScriptGetter = fn(&Item) -> Value;
    for (name, get) in [
        ("isCanBandage", (|s| Value::from(s.can_bandage)) as ScriptGetter),
        ("isTrap", |s| Value::from(s.trap)),
        ("getMakeUpType", |s| {
            s.make_up_type.as_deref().map(Value::from).unwrap_or_default()
        }),
        ("getOpeningRecipe", |s| {
            s.opening_recipe.as_deref().map(Value::from).unwrap_or_default()
        }),
        ("getEatType", |s| s.eat_type.as_deref().map(Value::from).unwrap_or_default()),
        // The item shares the script's list (`setRequireInHandOrInventory`).
        ("getRequireInHandOrInventory", |s| {
            opt_string_list(s.require_in_hand_or_inventory.as_ref().map(|l| l.iter().flatten()))
        }),
        ("getClothingItemExtraOption", |s| {
            opt_string_list(s.clothing_item_extra_option.as_ref())
        }),
    ] {
        item.method(name, move |c| {
            let script = script(c)?;
            let v = get(&script.0.borrow());
            Ok(c.push(v))
        });
    }

    // `attachmentsProvided`, `attachmentReplacement` and `attachmentType`:
    // `InstanceItem` copies them from the script (the list is the
    // script's own); null without a script.
    type AttachmentGetter = fn(&Item) -> Value;
    for (name, get) in [
        (
            "getAttachmentsProvided",
            (|s| opt_string_list(s.attachments_provided.as_ref())) as AttachmentGetter,
        ),
        ("getAttachmentReplacement", |s| {
            s.attachment_replacement.as_deref().map(Value::from).unwrap_or_default()
        }),
        ("getAttachmentType", |s| {
            s.attachment_type.as_deref().map(Value::from).unwrap_or_default()
        }),
    ] {
        item.method(name, move |c| {
            let script = this(c)?.borrow().template.script.clone();
            Ok(c.push(script.map_or(Value::Nil, |s| get(&s.0.borrow()))))
        });
    }
    // `scriptItem == null ? 0 : scriptItem.getEatTime()`.
    item.method("getEatTime", |c| {
        let script = this(c)?.borrow().template.script.clone();
        Ok(c.push(script.map_or(0, |s| s.0.borrow().eat_time)))
    });
    // `Food.customEatSound`, copied from the script at creation.
    let food = expose_class(thread, FOOD_CLASS, Some(INVENTORY_ITEM_CLASS));
    food.method("getCustomEatSound", |c| {
        let v = script(c)?.0.borrow().custom_eat_sound.as_deref().map(Value::from).unwrap_or_default();
        Ok(c.push(v))
    });

    item.method("getScriptItem", |c| {
        let script = this(c)?.borrow().template.script.clone();
        let v = match script {
            Some(s) => crate::bind_scripts::script_value(Some(
                &pz_scripts::ScriptRef::from_shared(s.0),
            )),
            None => Value::Nil,
        };
        Ok(c.push(v))
    });
    // `isRecordedMedia() ? RecordedMedia.getMediaDataFromIndex(index) : null`.
    item.method("getMediaData", |c| {
        let index = this(c)?.borrow().recorded_media_index;
        let v = if index >= 0 {
            crate::bind_radio::media_data_from_index(index)
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
    // `attrib() != null`, both `OriginX` and `OriginY` present, and one of
    // them not 0.
    item.method("hasOrigin", |c| {
        let item = this(c)?;
        let attributes = item
            .borrow()
            .get_attributes_ref()
            .map_err(runtime_error)?
            .is_some();
        if !attributes {
            return Ok(c.push(false));
        }
        let env = super::env();
        let context = env
            .entity()
            .ok_or_else(|| unsupported("InventoryItem.hasOrigin without entity registries"))?;
        let mut origin = [0; 2];
        for (value, name) in origin.iter_mut().zip(["OriginX", "OriginY"]) {
            let Some(attribute) =
                components::actual_attribute_ref(&item, context, name).map_err(runtime_error)?
            else {
                return Ok(c.push(false));
            };
            let attribute = components::actual_attribute_value(&attribute).map_err(runtime_error)?;
            *value = components::int_of(&attribute);
        }
        Ok(c.push(origin != [0, 0]))
    });
    item.method("getAnimalTracks", |c| {
        if this(c)?.borrow().animal_tracks.is_some() {
            return Err(unsupported("InventoryItem.getAnimalTracks AnimalTracks userdata"));
        }
        Ok(c.push(Value::Nil))
    });

    let weapon = expose_class(thread, HAND_WEAPON_CLASS, Some(INVENTORY_ITEM_CLASS));
    type WeaponGetter = fn(&WeaponData) -> Value;
    for (name, get) in [
        ("canBePlaced", (|w| Value::from(w.can_be_placed)) as WeaponGetter),
        ("getSensorRange", |w| Value::from(w.sensor_range)),
        ("getExplosionTimer", |w| Value::from(w.explosion_timer)),
        ("getFireModePossibilities", |w| {
            opt_string_list(w.fire_mode_possibilities.as_deref())
        }),
    ] {
        weapon.method(name, move |c| {
            let item = this(c)?;
            let item = item.borrow();
            let ItemKind::Weapon(w) = &item.kind else {
                return Err(unsupported("HandWeapon getter receiver class"));
            };
            let v = get(w);
            drop(item);
            Ok(c.push(v))
        });
    }
    // A new list of the attached parts whose `canDetach(chr, this)` is true.
    // `canDetach` calls the part's Lua callback and `getWeaponPart`, which
    // need the attached parts as shared items. Only a weapon without parts
    // is supported.
    weapon.method("getDetachableWeaponParts", |c| {
        let item = this(c)?;
        let attached = match &item.borrow().kind {
            ItemKind::Weapon(w) => !w.attachments.is_empty(),
            _ => return Err(unsupported("HandWeapon getter receiver class")),
        };
        if attached {
            return Err(unsupported(
                "HandWeapon.getDetachableWeaponParts with attached weapon parts",
            ));
        }
        Ok(c.push(crate::bind_java_util::list_value(Vec::new())))
    });
}

#[cfg(test)]
#[path = "context_menu/tests.rs"]
mod tests;
