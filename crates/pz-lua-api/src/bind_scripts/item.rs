//! `zombie.scripting.objects.Item`.
//!
//! Not exposed yet: `InstanceItem` (TODO(inventory)), members taking a
//! character or player (`isUnwanted`, `setUnwanted`, `setWanted`,
//! `researchRecipes`, `isFavouriteRecipeInput`, `getUsedInFavouriteRecipes`,
//! the character overloads of `getUsedInRecipes`, `getResearchableRecipes`
//! and `isUsedInBuildRecipes`; TODO(IsoGameCharacter)), `getLootType`
//! (TODO(ItemPickerJava)), `getNumSpawned` (TODO(InstanceTracker)),
//! `getDiscomfortModifier` (TODO(SandboxOptions)), `getNormalTexture`
//! (TODO(Texture)), `getClothingItemAsset`/`setClothingItemAsset`
//! (TODO(ClothingItem)), `getBloodClothingType` (TODO(BloodClothingType)),
//! `getMuzzleFlashModelKey` (TODO(ModelKey)), `getVehiclePartModels`,
//! `getReplaceTypesMap`, `setPaletteChoices` and `setWeaponCategories`
//! (Java collections), `getSoundByID(SoundMapKey)`.

use super::{
    get, handle_arg, key_list, list_arg, method, opt_str, opt_string_list, req_str, script_value, set, string_list, void,
    IntoLua, ScriptHandle, ITEM_CLASS,
};
use crate::bind_registry::{
    java_error, key_arg, key_value, AMMO_TYPE_CLASS, ITEM_BODY_LOCATION_CLASS, ITEM_TAG_CLASS, ITEM_TYPE_CLASS,
    WEAPON_CATEGORY_CLASS,
};
use crate::expose::{expose_class, ClassBuilder};
use pz_kahlua::{LuaResult, NativeCall, Thread};
use pz_scripts::objects::items::Item;
use pz_scripts::{ScriptManager, ScriptType};
use std::rc::Rc;

/// The `checkedExtra` lookups (`canBeForaged`, `canSpawnAsLoot`,
/// `isCraftRecipeProduct`): unless set or already checked, the flag is taken
/// from the clothing extras and the cleaned item.
fn checked_extra(
    sm: &ScriptManager,
    h: &ScriptHandle,
    flag: fn(&mut Item) -> (&mut bool, &mut bool),
) -> LuaResult<bool> {
    let (set_, checked, extras, after) = h.write(|i: &mut Item| {
        let extras = i.clothing_item_extra.clone();
        let after = i.item_after_cleaning.clone();
        let (v, c) = flag(i);
        (*v, *c, extras, after)
    })?;
    if !set_ && !checked {
        let mut found = false;
        let names = extras.unwrap_or_default().into_iter().chain(after);
        for name in names {
            let Some(other) = sm.get_script(ScriptType::Item, &name) else { continue };
            let v = if other.ptr_eq(&h.root) {
                false
            } else {
                other.downcast::<Item>().is_some_and(|o| *flag(&mut o.borrow_mut()).0)
            };
            if v {
                found = true;
                break;
            }
        }
        h.write(|i: &mut Item| {
            let (v, c) = flag(i);
            *v |= found;
            *c = true;
        })?;
        return Ok(found);
    }
    h.write(|i: &mut Item| *flag(i).1 = true)?;
    Ok(set_)
}

/// `getResearchableRecipes()`: the item's own, plus the clothing extras' and
/// the cleaned item's the first time.
fn researchable_recipes(sm: &ScriptManager, h: &ScriptHandle) -> LuaResult<Vec<String>> {
    let (checked, extras, after) = h.read(|i: &Item| {
        (i.is_researchable_recipes_checked_extra, i.clothing_item_extra.clone(), i.item_after_cleaning.clone())
    })?;
    if !checked {
        let mut names: Vec<String> = Vec::new();
        for name in extras.unwrap_or_default().into_iter().chain(after) {
            let Some(other) = sm.get_script(ScriptType::Item, &name) else { continue };
            if other.ptr_eq(&h.root) {
                continue;
            }
            if let Some(o) = other.downcast::<Item>() {
                names.extend(o.borrow().researchable_recipes.iter().cloned());
            }
        }
        h.write(|i: &mut Item| {
            i.add_researchable_recipes(sm, &names);
            i.is_researchable_recipes_checked_extra = true;
        })?;
    }
    h.read(|i: &Item| i.researchable_recipes.clone())
}

fn key_args(c: &NativeCall<'_>, from: usize) -> LuaResult<Vec<Option<Rc<pz_scripts::registry::RegistryKey>>>> {
    (from..c.nargs().max(from)).map(|i| key_arg(c, i)).collect()
}

pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, ITEM_CLASS, Some(super::GAME_ENTITY_SCRIPT_CLASS));
    register_fields(&b);

    method(&b, "hasTag", |c, h| {
        let tags = key_args(c, 1)?;
        h.read(|i: &Item| tags.iter().any(|t| i.has_tag(t.as_ref())).into_lua())
    });
    get(&b, "getActualWeight", |i: &Item| i.actual_weight());
    set(&b, "setActualWeight", |i: &mut Item, w: f32| i.set_actual_weight(w));
    get(&b, "getR", |i: &Item| i.r());
    get(&b, "getG", |i: &Item| i.g());
    get(&b, "getB", |i: &Item| i.b());
    get(&b, "getColorRed", |i: &Item| i.r());
    get(&b, "getColorGreen", |i: &Item| i.g());
    get(&b, "getColorBlue", |i: &Item| i.b());
    get(&b, "getTicksPerEquipUse", |i: &Item| i.ticks_per_equip_use());
    method(&b, "isItemType", |c, h| {
        let t = key_arg(c, 1)?;
        h.read(|i: &Item| i.is_item_type_key(t.as_ref()).into_lua())
    });
    method(&b, "isBodyLocation", |c, h| {
        let l = key_arg(c, 1)?;
        h.read(|i: &Item| i.is_body_location(l.as_ref()).into_lua())
    });
    method(&b, "containsWeaponCategory", |c, h| {
        let w = key_arg(c, 1)?;
        h.read(|i: &Item| i.contains_weapon_category(w.as_ref()).into_lua())
    });
    method(&b, "getSoundParameter", |c, h| {
        let p = opt_str(c, 1).unwrap_or_default();
        h.read(|i: &Item| i.sound_parameter(&p).into_lua())
    });
    method(&b, "getSoundByID", |c, h| {
        let id = opt_str(c, 1).unwrap_or_default();
        h.read(|i: &Item| i.sound_by_id(&id).into_lua())
    });
    method(&b, "getReplaceType", |c, h| {
        let k = opt_str(c, 1).unwrap_or_default();
        h.read(|i: &Item| i.replace_type(&k).into_lua())
    });
    method(&b, "hasReplaceType", |c, h| {
        let k = opt_str(c, 1).unwrap_or_default();
        h.read(|i: &Item| i.has_replace_type(&k).into_lua())
    });
    get(&b, "isKeepOnDeplete", |i: &Item| i.is_keep_on_deplete());
    set(&b, "setKeepOnDeplete", |i: &mut Item, v: bool| i.set_keep_on_deplete(v));
    get(&b, "getMaxLevelTrained", |i: &Item| i.max_level_trained());
    get(&b, "isCookwareLoot", |i: &Item| i.is_cookware_loot());
    get(&b, "isMaterialLoot", |i: &Item| i.is_material_loot());
    get(&b, "isToolLoot", |i: &Item| i.is_tool_loot());
    get(&b, "isSurvivalGearLoot", |i: &Item| i.is_survival_gear_loot());
    get(&b, "isMedicalLoot", |i: &Item| i.is_medical_loot());
    get(&b, "isMechanicsLoot", |i: &Item| i.is_mechanics_loot());
    get(&b, "hasResearchableRecipes", |i: &Item| i.has_researchable_recipes());
    get(&b, "toString", |i: &Item| i.to_java_string());
    get(&b, "getDisplayName", |i: &Item| i.display_name());
    get(&b, "getModuleName", |i: &Item| i.module_name());
    void(&b, "addForageFocusCategory", |c, h| {
        let name = opt_str(c, 1).unwrap_or_else(|| "null".into());
        h.write(|i: &mut Item| {
            i.forage_focus_categories.insert(name);
        })
    });
    void(&b, "clearForageFocusCategories", |_, h| h.write(|i: &mut Item| i.forage_focus_categories.clear()));
    void(&b, "setName", |c, h| {
        let name = opt_str(c, 1).unwrap_or_else(|| "null".into());
        h.write(|i: &mut Item| {
            let module = i.module_name().ok_or_else(super::npe)?;
            i.module_dot_type = format!("{module}.{name}");
            i.name = name;
            Ok(())
        })?
    });
    method(&b, "getUsedInRecipes", |c, h| {
        if !c.arg(1).is_nil() {
            return Err(pz_kahlua::LuaError::runtime("TODO(IsoGameCharacter): getUsedInRecipes(character)"));
        }
        h.read(|i: &Item| string_list(i.used_in_recipes()))
    });
    method(&b, "isUsedInRecipes", |c, h| {
        if !c.arg(1).is_nil() {
            return Err(pz_kahlua::LuaError::runtime("TODO(IsoGameCharacter): isUsedInRecipes(character)"));
        }
        h.read(|i: &Item| i.is_used_in_recipes().into_lua())
    });

    let s = sm.clone();
    method(&b, "isSpice", move |_, h| h.read(|i: &Item| i.is_spice(&s).into_lua()));
    let s = sm.clone();
    method(&b, "isMementoLoot", move |_, h| h.read(|i: &Item| i.is_memento_loot(&s).into_lua()));
    let s = sm.clone();
    method(&b, "isFarmingLoot", move |_, h| h.read(|i: &Item| i.is_farming_loot(&s).into_lua()));
    let s = sm.clone();
    method(&b, "ignoreZombieDensity", move |_, h| h.read(|i: &Item| i.ignore_zombie_density(&s).into_lua()));
    let s = sm.clone();
    method(&b, "getStaticModelException", move |_, h| {
        h.read(|i: &Item| i.static_model_exception(&s).map(str::to_string).into_lua())
    });
    let s = sm.clone();
    void(&b, "resolveItemTypes", move |_, h| h.write(|i: &mut Item| i.resolve_item_types(&s)));
    let s = sm.clone();
    void(&b, "resolveModelScripts", move |_, h| h.write(|i: &mut Item| i.resolve_model_scripts(&s)));
    let s = sm.clone();
    void(&b, "DoParam", move |c, h| {
        let r = if c.nargs() > 2 {
            let (p, v) = (req_str(c, 1)?, req_str(c, 2)?);
            h.write(|i: &mut Item| i.do_param(&s, &p, &v))?
        } else {
            let p = req_str(c, 1)?;
            h.write(|i: &mut Item| i.do_param_str(&s, &p))?
        };
        r.map_err(java_error)
    });
    let s = sm.clone();
    void(&b, "addResearchableRecipe", move |c, h| {
        if let Ok(Some(r)) = handle_arg(c, 1) {
            // The CraftRecipe overload.
            return r.write(|recipe: &mut pz_scripts::objects::entity::components::crafting::CraftRecipe| {
                h.write(|i: &mut Item| i.add_researchable_craft_recipe(recipe))
            })?;
        }
        let Some(name) = opt_str(c, 1) else { return Ok(()) };
        h.write(|i: &mut Item| i.add_researchable_recipe(&s, &name))
    });
    let s = sm.clone();
    void(&b, "addResearchableRecipes", move |c, h| {
        let names: Vec<String> = list_arg(c, 1).iter().map(pz_kahlua::exec::java_value_of).collect();
        h.write(|i: &mut Item| i.add_researchable_recipes(&s, &names))
    });
    let s = sm.clone();
    method(&b, "getResearchableRecipes", move |c, h| {
        if !c.arg(1).is_nil() {
            return Err(pz_kahlua::LuaError::runtime("TODO(IsoGameCharacter): getResearchableRecipes(character)"));
        }
        Ok(string_list(researchable_recipes(&s, h)?))
    });
    let s = sm.clone();
    method(&b, "canBeForaged", move |_, h| {
        checked_extra(&s, h, |i| (&mut i.can_be_foraged, &mut i.can_be_foraged_checked_extra)).map(IntoLua::into_lua)
    });
    let s = sm.clone();
    method(&b, "canSpawnAsLoot", move |_, h| {
        checked_extra(&s, h, |i| (&mut i.can_spawn_as_loot, &mut i.can_spawn_as_loot_checked_extra)).map(IntoLua::into_lua)
    });
    let s = sm.clone();
    method(&b, "isCraftRecipeProduct", move |_, h| {
        checked_extra(&s, h, |i| (&mut i.is_craft_recipe_product, &mut i.is_craft_recipe_product_checked_extra))
            .map(IntoLua::into_lua)
    });
}

/// Plain field getters and setters.
fn register_fields(b: &ClassBuilder) {
    get(b, "getStressChange", |i: &Item| i.stress_change);
    get(b, "getLearnedRecipes", |i: &Item| opt_string_list(i.learned_recipes.as_ref()));
    get(b, "getShoutType", |i: &Item| i.shout_type.clone());
    get(b, "getShoutMultiplier", |i: &Item| i.shout_multiplier);
    get(b, "getThirstChange", |i: &Item| i.thirst_change);
    get(b, "getHungerChange", |i: &Item| i.hunger_change);
    get(b, "getEnduranceChange", |i: &Item| i.endurance_change);
    get(b, "getUnhappyChange", |i: &Item| i.unhappy_change);
    get(b, "getPushBackMod", |i: &Item| i.push_back_mod);
    get(b, "isRanged", |i: &Item| i.ranged);
    get(b, "getMaxRange", |i: &Item| i.max_range);
    get(b, "getSwingAnim", |i: &Item| i.swing_anim.clone());
    get(b, "getMinAngle", |i: &Item| i.min_angle);
    get(b, "getPhysicsObject", |i: &Item| i.physics_object.clone());
    get(b, "getChanceToFall", |i: &Item| i.chance_to_fall);
    get(b, "getDropSound", |i: &Item| i.drop_sound.clone());
    get(b, "getPoisonPower", |i: &Item| i.poison_power as f32);
    get(b, "getHerbalistType", |i: &Item| i.herbalist_type.clone());
    get(b, "getItemType", |i: &Item| key_value(ITEM_TYPE_CLASS, i.item_type.as_ref()));
    void(b, "setItemType", |c, h| { let k = key_arg(c, 1)?; h.write(|i: &mut Item| i.item_type = k) });
    get(b, "getVisionModifier", |i: &Item| i.vision_modifier);
    get(b, "getHearingModifier", |i: &Item| i.hearing_modifier);
    get(b, "getStaticModel", |i: &Item| i.static_model.clone());
    get(b, "isVisualAid", |i: &Item| i.visual_aid);
    get(b, "getTags", |i: &Item| key_list(ITEM_TAG_CLASS, &i.item_tags));
    set(b, "setDisplayName", |i: &mut Item, v: Option<String>| i.display_name = v);
    get(b, "isUseEndurance", |i: &Item| i.use_endurance);
    get(b, "getEnduranceMod", |i: &Item| i.endurance_mod);
    get(b, "isAlwaysKnockdown", |i: &Item| i.always_knockdown);
    get(b, "getTemperature", |i: &Item| i.temperature);
    get(b, "getInsulation", |i: &Item| i.insulation);
    get(b, "getAimReleaseSound", |i: &Item| i.aim_release_sound.clone());
    get(b, "getTooltip", |i: &Item| i.tooltip.clone());
    get(b, "isWorldRender", |i: &Item| i.world_render);
    set(b, "setIcon", |i: &mut Item, v: Option<String>| i.icon = v);
    get(b, "isAlcoholic", |i: &Item| i.alcoholic);
    get(b, "getBoredomChange", |i: &Item| i.boredom_change);
    get(b, "getMinDamage", |i: &Item| i.min_damage);
    get(b, "getMaxDamage", |i: &Item| i.max_damage);
    get(b, "getBodyLocation", |i: &Item| key_value(ITEM_BODY_LOCATION_CLASS, i.body_location.as_ref()));
    get(b, "getWindresist", |i: &Item| i.windresist);
    get(b, "getReplaceOnUse", |i: &Item| i.replace_on_use.clone());
    set(b, "setThirstChange", |i: &mut Item, v: f32| i.thirst_change = v);
    get(b, "getSpawnWith", |i: &Item| i.spawn_with.clone());
    set(b, "setUnhappyChange", |i: &mut Item, v: f32| i.unhappy_change = v);
    set(b, "setUseDelta", |i: &mut Item, v: f32| i.use_delta = v);
    get(b, "getFabricType", |i: &Item| i.fabric_type.clone());
    get(b, "getConditionMax", |i: &Item| i.condition_max);
    set(b, "setHungerChange", |i: &mut Item, v: f32| i.hunger_change = v);
    set(b, "setStressChange", |i: &mut Item, v: f32| i.stress_change = v);
    set(b, "setEnduranceChange", |i: &mut Item, v: f32| i.endurance_change = v);
    set(b, "setCanSpawnAsLoot", |i: &mut Item, v: bool| i.can_spawn_as_loot = v);
    get(b, "getMapID", |i: &Item| i.map.clone());
    get(b, "getDaysFresh", |i: &Item| i.days_fresh);
    get(b, "getEvolvedRecipe", |i: &Item| string_list(&i.evolved_recipe));
    set(b, "setCanBarricade", |i: &mut Item, v: bool| i.can_barricade = v);
    get(b, "getSpriteName", |i: &Item| i.sprite_name.clone());
    get(b, "getSplatNumber", |i: &Item| i.splat_number);
    get(b, "getClothingItem", |i: &Item| i.clothing_item.clone());
    get(b, "getDoorHitSound", |i: &Item| i.door_hit_sound.clone());
    get(b, "getMaxHitCount", |i: &Item| i.max_hit_count);
    get(b, "getAmmoType", |i: &Item| key_value(AMMO_TYPE_CLASS, i.ammo_type.as_ref()));
    get(b, "getSwingSound", |i: &Item| i.swing_sound.clone());
    get(b, "getSoundRadius", |i: &Item| i.sound_radius);
    get(b, "getSoundVolume", |i: &Item| i.sound_volume);
    get(b, "isUseSelf", |i: &Item| i.use_self);
    get(b, "isOtherHandUse", |i: &Item| i.other_hand_use);
    get(b, "getDoorDamage", |i: &Item| i.door_damage);
    set(b, "setSoundVolume", |i: &mut Item, v: i32| i.sound_volume = v);
    get(b, "getBreakSound", |i: &Item| i.break_sound.clone());
    get(b, "getBulletOutSound", |i: &Item| i.bullet_out_sound.clone());
    get(b, "getCloseSound", |i: &Item| i.close_sound.clone());
    get(b, "getCustomEatSound", |i: &Item| i.custom_eat_sound.clone());
    get(b, "getCountDownSound", |i: &Item| i.count_down_sound.clone());
    get(b, "getExplosionSound", |i: &Item| i.explosion_sound.clone());
    get(b, "getImpactSound", |i: &Item| i.impact_sound.clone());
    get(b, "getOpenSound", |i: &Item| i.open_sound.clone());
    get(b, "getPutInSound", |i: &Item| i.put_in_sound.clone());
    get(b, "getPlaceOneSound", |i: &Item| i.place_one_sound.clone());
    get(b, "getShellFallSound", |i: &Item| i.shell_fall_sound.clone());
    get(b, "getInsertAmmoSound", |i: &Item| i.insert_ammo_sound.clone());
    get(b, "getEjectAmmoSound", |i: &Item| i.eject_ammo_sound.clone());
    get(b, "getObsolete", |i: &Item| i.obsolete);
    get(b, "getIcon", |i: &Item| i.icon.clone());
    get(b, "getEquipSound", |i: &Item| i.equip_sound.clone());
    get(b, "getUnequipSound", |i: &Item| i.unequip_sound.clone());
    get(b, "isDisappearOnUse", |i: &Item| i.disappear_on_use);
    get(b, "getIconsForTexture", |i: &Item| opt_string_list(i.icons_for_texture.as_ref()));
    get(b, "getMinutesToCook", |i: &Item| i.minutes_to_cook);
    get(b, "getMinutesToBurn", |i: &Item| i.minutes_to_burn);
    get(b, "getDamagedSound", |i: &Item| i.damaged_sound.clone());
    get(b, "isCanBandage", |i: &Item| i.can_bandage);
    get(b, "getReplaceTypes", |i: &Item| i.replace_types.clone());
    get(b, "getPourType", |i: &Item| i.pour_type.clone());
    get(b, "getDisplayCategory", |i: &Item| i.display_category.clone());
    get(b, "getLuaCreate", |i: &Item| i.lua_create.clone());
    get(b, "getWithDrainable", |i: &Item| i.with_drainable.clone());
    get(b, "getStrainModifier", |i: &Item| i.strain_modifier);
    get(b, "getOnBreak", |i: &Item| i.on_break.clone());
    get(b, "getEatTime", |i: &Item| i.eat_time);
    get(b, "getFireFuelRatio", |i: &Item| i.fire_fuel_ratio);
    get(b, "getOpeningRecipe", |i: &Item| i.opening_recipe.clone());
    set(b, "setReplaceOnUse", |i: &mut Item, v: Option<String>| i.replace_on_use = v);
    set(b, "setConditionMax", |i: &mut Item, v: i32| i.condition_max = v);
    get(b, "getUseDelta", |i: &Item| i.use_delta);
    get(b, "isIsCookable", |i: &Item| i.is_cookable);
    set(b, "setIsCookable", |i: &mut Item, v: bool| i.is_cookable = v);
    set(b, "setMinutesToCook", |i: &mut Item, v: i32| i.minutes_to_cook = v);
    set(b, "setMinutesToBurn", |i: &mut Item, v: i32| i.minutes_to_burn = v);
    set(b, "setBoredomChange", |i: &mut Item, v: f32| i.boredom_change = v);
    set(b, "setCount", |i: &mut Item, v: i32| i.count = v);
    set(b, "setAlcoholic", |i: &mut Item, v: bool| i.alcoholic = v);
    get(b, "getEatType", |i: &Item| i.eat_type.clone());
    void(b, "setAmmoType", |c, h| { let k = key_arg(c, 1)?; h.write(|i: &mut Item| i.ammo_type = k) });
    get(b, "getDigType", |i: &Item| i.dig_type.clone());
    set(b, "setSoundRadius", |i: &mut Item, v: i32| i.sound_radius = v);
    get(b, "isCosmetic", |i: &Item| i.cosmetic);
    set(b, "setSpriteName", |i: &mut Item, v: Option<String>| i.sprite_name = v);
    get(b, "getWeightWet", |i: &Item| i.weight_wet);
    set(b, "setTemperature", |i: &mut Item, v: f32| i.temperature = v);
    set(b, "setWeightWet", |i: &mut Item, v: f32| i.weight_wet = v);
    set(b, "setInsulation", |i: &mut Item, v: f32| i.insulation = v);
    get(b, "getWeightEmpty", |i: &Item| i.weight_empty);
    set(b, "setWeightEmpty", |i: &mut Item, v: f32| i.weight_empty = v);
    get(b, "getCookingSound", |i: &Item| i.cooking_sound.clone());
    get(b, "getWeaponSprite", |i: &Item| i.weapon_sprite.clone());
    set(b, "setMaxRange", |i: &mut Item, v: f32| i.max_range = v);
    set(b, "setMinDamage", |i: &mut Item, v: f32| i.min_damage = v);
    set(b, "setMaxDamage", |i: &mut Item, v: f32| i.max_damage = v);
    set(b, "setMinAngle", |i: &mut Item, v: f32| i.min_angle = v);
    set(b, "setWeaponSprite", |i: &mut Item, v: Option<String>| i.weapon_sprite = v);
    get(b, "getNoiseDuration", |i: &Item| i.noise_duration);
    get(b, "isAngleFalloff", |i: &Item| i.angle_falloff);
    set(b, "setAngleFalloff", |i: &mut Item, v: bool| i.angle_falloff = v);
    set(b, "setImpactSound", |i: &mut Item, v: Option<String>| i.impact_sound = v);
    set(b, "setMaxHitCount", |i: &mut Item, v: i32| i.max_hit_count = v);
    set(b, "setRanged", |i: &mut Item, v: bool| i.ranged = v);
    set(b, "setOtherHandUse", |i: &mut Item, v: bool| i.other_hand_use = v);
    set(b, "setPhysicsObject", |i: &mut Item, v: Option<String>| i.physics_object = v);
    set(b, "setPushBackMod", |i: &mut Item, v: f32| i.push_back_mod = v);
    get(b, "isRangeFalloff", |i: &Item| i.range_falloff);
    set(b, "setRangeFalloff", |i: &mut Item, v: bool| i.range_falloff = v);
    set(b, "setSplatNumber", |i: &mut Item, v: i32| i.splat_number = v);
    set(b, "setSwingSound", |i: &mut Item, v: Option<String>| i.swing_sound = v);
    get(b, "getSwingTime", |i: &Item| i.swing_time);
    set(b, "setSwingTime", |i: &mut Item, v: f32| i.swing_time = v);
    get(b, "getToHitModifier", |i: &Item| i.to_hit_modifier);
    set(b, "setToHitModifier", |i: &mut Item, v: f32| i.to_hit_modifier = v);
    set(b, "setUseEndurance", |i: &mut Item, v: bool| i.use_endurance = v);
    set(b, "setUseSelf", |i: &mut Item, v: bool| i.use_self = v);
    set(b, "setDoorDamage", |i: &mut Item, v: i32| i.door_damage = v);
    set(b, "setDoorHitSound", |i: &mut Item, v: Option<String>| i.door_hit_sound = v);
    get(b, "isShareEndurance", |i: &Item| i.share_endurance);
    set(b, "setShareEndurance", |i: &mut Item, v: bool| i.share_endurance = v);
    set(b, "setAlwaysKnockdown", |i: &mut Item, v: bool| i.always_knockdown = v);
    set(b, "setEnduranceMod", |i: &mut Item, v: f32| i.endurance_mod = v);
    get(b, "getKnockdownMod", |i: &Item| i.knockdown_mod);
    set(b, "setKnockdownMod", |i: &mut Item, v: f32| i.knockdown_mod = v);
    get(b, "getMaxItemSize", |i: &Item| i.max_item_size);
    get(b, "getNumberOfPages", |i: &Item| i.number_of_pages);
    get(b, "getSkillTrained", |i: &Item| i.skill_trained.clone());
    get(b, "getReadType", |i: &Item| i.read_type.clone());
    get(b, "isCantBeFrozen", |i: &Item| i.cant_be_frozen);
    get(b, "getItemConfig", |i: &Item| script_value(i.item_config.as_ref()));
    get(b, "getItemConfigKey", |i: &Item| i.item_config_key.clone());
    void(b, "setItemConfig", |c, h| { let s = handle_arg(c, 1)?.map(|s| s.root); h.write(|i: &mut Item| i.item_config = s) });
    set(b, "setCanBeForaged", |i: &mut Item, v: bool| i.can_be_foraged = v);
    set(b, "setDaysFresh", |i: &mut Item, v: i32| i.days_fresh = v);
    set(b, "setCanBandage", |i: &mut Item, v: bool| i.can_bandage = v);
    set(b, "setSwingAnim", |i: &mut Item, v: Option<String>| i.swing_anim = v);
    get(b, "getWeaponWeight", |i: &Item| i.weapon_weight);
    set(b, "setWeaponWeight", |i: &mut Item, v: f32| i.weapon_weight = v);
    get(b, "getNPCSoundBoost", |i: &Item| i.npc_sound_boost);
    set(b, "setNPCSoundBoost", |i: &mut Item, v: f32| i.npc_sound_boost = v);
    get(b, "isCanBarricade", |i: &Item| i.can_barricade);
    get(b, "isUseWhileEquipped", |i: &Item| i.use_while_equipped);
    set(b, "setDisappearOnUse", |i: &mut Item, v: bool| i.disappear_on_use = v);
    void(b, "setBodyLocation", |c, h| { let k = key_arg(c, 1)?; h.write(|i: &mut Item| i.body_location = k) });
    get(b, "getPaletteChoices", |i: &Item| string_list(&i.palette_choices));
    get(b, "getPalettesStart", |i: &Item| i.palettes_start.clone());
    set(b, "setPalettesStart", |i: &mut Item, v: Option<String>| i.palettes_start = v);
    set(b, "setWindresist", |i: &mut Item, v: f32| i.windresist = v);
    get(b, "getWaterresist", |i: &Item| i.waterresist);
    set(b, "setWaterresist", |i: &mut Item, v: f32| i.waterresist = v);
    get(b, "isCantEat", |i: &Item| i.cant_eat);
    set(b, "setLuaCreate", |i: &mut Item, v: Option<String>| i.lua_create = v);
    set(b, "setOpeningRecipe", |i: &mut Item, v: Option<String>| i.opening_recipe = v);
    get(b, "getIconFluidMask", |i: &Item| i.icon_fluid_mask.clone());
    get(b, "getOtherHandRequire", |i: &Item| key_value(ITEM_TAG_CLASS, i.other_hand_require.as_ref()));
    get(b, "getPlaceMultipleSound", |i: &Item| i.place_multiple_sound.clone());
    get(b, "getInsertAmmoStartSound", |i: &Item| i.insert_ammo_start_sound.clone());
    get(b, "getInsertAmmoStopSound", |i: &Item| i.insert_ammo_stop_sound.clone());
    get(b, "getEjectAmmoStartSound", |i: &Item| i.eject_ammo_start_sound.clone());
    get(b, "getEjectAmmoStopSound", |i: &Item| i.eject_ammo_stop_sound.clone());
    get(b, "getFoodSicknessChange", |i: &Item| i.food_sickness_change);
    get(b, "isSplatBloodOnNoDeath", |i: &Item| i.splat_blood_on_no_death);
    get(b, "isKnockBackOnNoDeath", |i: &Item| i.knock_back_on_no_death);
    get(b, "isCantAttackWithLowestEndurance", |i: &Item| i.cant_attack_with_lowest_endurance);
    get(b, "getBulletHitArmourSound", |i: &Item| i.bullet_hit_armour_sound.clone());
    get(b, "getWeaponHitArmourSound", |i: &Item| i.weapon_hit_armour_sound.clone());
    get(b, "getPoisonDetectionLevel", |i: &Item| i.poison_detection_level);
    get(b, "getCorpseSicknessDefense", |i: &Item| i.corpse_sickness_defense);
    get(b, "getBringToBearSound", |i: &Item| i.bring_to_bear_sound.clone());
    get(b, "getConditionLowerChance", |i: &Item| i.condition_lower_chance);
    set(b, "setIsCraftRecipeProduct", |i: &mut Item, v: bool| i.is_craft_recipe_product = v);
    get(b, "getHeadConditionLowerChanceMultiplier", |i: &Item| i.head_condition_lower_chance_multiplier);
    get(b, "getForageFocusCategories", |i: &Item| string_list(&i.forage_focus_categories));
    get(b, "isConditionAffectsCapacity", |i: &Item| i.condition_affects_capacity);
    get(b, "getWeaponSpritesByIndex", |i: &Item| opt_string_list(i.weapon_sprites_by_index.as_ref()));
    get(b, "getStaticModelsByIndex", |i: &Item| opt_string_list(i.static_models_by_index.as_ref()));
    get(b, "getWorldStaticModelsByIndex", |i: &Item| opt_string_list(i.world_static_models_by_index.as_ref()));
    get(b, "isAlwaysWelcomeGift", |i: &Item| i.always_welcome_gift);
    get(b, "getFillFromDispenserSound", |i: &Item| i.fill_from_dispenser_sound.clone());
    get(b, "getFillFromLakeSound", |i: &Item| i.fill_from_lake_sound.clone());
    get(b, "getFillFromTapSound", |i: &Item| i.fill_from_tap_sound.clone());
    get(b, "getFillFromToiletSound", |i: &Item| i.fill_from_toilet_sound.clone());
    get(b, "getWorldObjectSprite", |i: &Item| i.world_object_sprite.clone());
    get(b, "getItemAfterCleaning", |i: &Item| i.item_after_cleaning.clone());
    get(b, "getDoubleClickRecipe", |i: &Item| i.double_click_recipe.clone());
    get(b, "getRecordedMediaCat", |i: &Item| i.recorded_media_cat.clone());
    get(b, "getReplaceOnExtinguish", |i: &Item| i.replace_on_extinguish.clone());
    get(b, "getClothingItemExtra", |i: &Item| opt_string_list(i.clothing_item_extra.as_ref()));
    get(b, "getClothingItemExtraOption", |i: &Item| opt_string_list(i.clothing_item_extra_option.as_ref()));
    set(b, "setConditionLowerChance", |i: &mut Item, v: i32| i.condition_lower_chance = v);
    set(b, "setTicksPerEquipUse", |i: &mut Item, v: i32| i.ticks_per_equip_use = v);
    set(b, "setReplaceOnDeplete", |i: &mut Item, v: Option<String>| i.replace_on_deplete = v);
    get(b, "isManuallyRemoveSpentRounds", |i: &Item| i.manually_remove_spent_rounds);
    set(b, "setKnockBackOnNoDeath", |i: &mut Item, v: bool| i.knock_back_on_no_death = v);
    get(b, "getMinimumSwingTime", |i: &Item| i.minimum_swing_time);
    set(b, "setMinimumSwingTime", |i: &mut Item, v: f32| i.minimum_swing_time = v);
    void(b, "setOtherHandRequire", |c, h| { let k = key_arg(c, 1)?; h.write(|i: &mut Item| i.other_hand_require = k) });
    set(b, "setSplatBloodOnNoDeath", |i: &mut Item, v: bool| i.splat_blood_on_no_death = v);
    get(b, "isMultipleHitConditionAffected", |i: &Item| i.multiple_hit_condition_affected);
    set(b, "setMultipleHitConditionAffected", |i: &mut Item, v: bool| i.multiple_hit_condition_affected = v);
    set(b, "setCantAttackWithLowestEndurance", |i: &mut Item, v: bool| i.cant_attack_with_lowest_endurance = v);
    get(b, "getNumLevelsTrained", |i: &Item| i.num_levels_trained);
    get(b, "isUsedInBuildRecipes", |i: &Item| i.is_used_in_build_recipes);
    get(b, "getDaysTotallyRotten", |i: &Item| i.days_totally_rotten);
    set(b, "setDaysTotallyRotten", |i: &mut Item, v: i32| i.days_totally_rotten = v);
    set(b, "setAlwaysWelcomeGift", |i: &mut Item, v: bool| i.always_welcome_gift = v);
    get(b, "getOtherCharacterVolumeBoost", |i: &Item| i.other_character_volume_boost);
    set(b, "setOtherCharacterVolumeBoost", |i: &mut Item, v: f32| i.other_character_volume_boost = v);
    get(b, "getWeaponCategories", |i: &Item| key_list(WEAPON_CATEGORY_CLASS, &i.weapon_categories));
    get(b, "getSwingAmountBeforeImpact", |i: &Item| i.swing_amount_before_impact);
    set(b, "setSwingAmountBeforeImpact", |i: &mut Item, v: f32| i.swing_amount_before_impact = v);
    set(b, "setReplaceOnExtinguish", |i: &mut Item, v: Option<String>| i.replace_on_extinguish = v);
    get(b, "isDangerousUncooked", |i: &Item| i.dangerous_uncooked);
    set(b, "setDangerousUncooked", |i: &mut Item, v: bool| i.dangerous_uncooked = v);
    set(b, "setUseWhileEquipped", |i: &mut Item, v: bool| i.use_while_equipped = v);
    get(b, "isUseWhileUnequipped", |i: &Item| i.use_while_unequipped);
    set(b, "setUseWhileUnequipped", |i: &mut Item, v: bool| i.use_while_unequipped = v);
    get(b, "getLevelSkillTrained", |i: &Item| i.lvl_skill_trained);
    get(b, "getReplaceWhenUnequip", |i: &Item| i.replace_when_unequip.clone());
    set(b, "setDoubleClickRecipe", |i: &mut Item, v: Option<String>| i.double_click_recipe = v);
    get(b, "getReplaceOnDeplete", |i: &Item| i.replace_on_deplete.clone());
    get(b, "getWithoutDrainable", |i: &Item| i.without_drainable.clone());
    get(b, "getWorldStaticModel", |i: &Item| i.world_static_model.clone());
    get(b, "getAcceptItemFunction", |i: &Item| i.accept_item_function.clone());
    get(b, "getName", |i: &Item| i.name.clone());
    get(b, "isHidden", |i: &Item| i.hidden);
    get(b, "getCount", |i: &Item| i.count);
    get(b, "getFileName", |i: &Item| i.file_name.clone());
    get(b, "getFullName", |i: &Item| i.module_dot_type.clone());
}
