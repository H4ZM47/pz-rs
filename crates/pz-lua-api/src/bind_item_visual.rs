//! Item creation and item visuals for Lua: the `instanceItem` global
//! (`InventoryItemFactory.CreateItem`), `getAllDecalNamesForItem`, the
//! `InventoryItem` methods that reach the visual (`getVisual`,
//! `getClothingItem`, `allowRandomTint`, `synchWithVisual`,
//! `canBeEquipped`), and the classes they return: `ItemVisual`,
//! `ClothingItem`, and the `ImmutableColor` value type (its class is
//! registered in [`crate::bind_misc_classes`]). Character creation dresses its
//! avatar with these (`CharacterCreationMain`).
//!
//! Items come from the installed [`crate::bind_items::ItemLuaEnv`] (its
//! `Rand`, `OutfitRNG` and `ClothingItem` assets) and the shared
//! `ScriptManager`; without assets no item has a visual (as when the
//! clothing assets are not loaded).
//!
//! [`set_worn_items_from_item_visuals`] is `WornItems.setFromItemVisuals`
//! (the item half of `SurvivorDesc.dressInNamedOutfit`).
//!
//! An `ItemVisual` object stands for its item's visual: it reads and
//! writes `item.visual` (the original returns the same object until
//! `getVisual` drops it). The tint keeps no alpha (`ItemVisual.tint` is
//! RGB here; every tint the game makes is opaque).

use crate::bind_characters::{LuaItem, LuaSurvivorDesc};
use crate::bind_items::{
    item_arg, item_env, item_value, ItemRef, LuaInventoryItem, INVENTORY_ITEM_CLASS,
};
use crate::bind_java_util::JavaList;
use crate::bind_registry::{key_value, shared_script_manager, ITEM_BODY_LOCATION_CLASS};
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg, with_this};
use pz_inventory::item::ItemKind;
use pz_inventory::visual::ItemVisual;
use pz_inventory::{ClothingItem, ItemOwner};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::registry::{RegistryKey, ResourceLocation};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const ITEM_VISUAL_CLASS: &str = "zombie.core.skinnedmodel.visual.ItemVisual";
pub const CLOTHING_ITEM_CLASS: &str = "zombie.core.skinnedmodel.population.ClothingItem";
pub const ASSET_CLASS: &str = "zombie.asset.Asset";
pub const IMMUTABLE_COLOR_CLASS: &str = "zombie.core.ImmutableColor";

type VisualObjects = HashMap<usize, (Weak<RefCell<pz_inventory::InventoryItem>>, Value)>;

thread_local! {
    static CLOTHING: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
    static VISUALS: RefCell<VisualObjects> = RefCell::new(HashMap::new());
}

/// Item visuals and cloned clothing assets belong to the retired Lua
/// environment. Clear them before loading a new set of item assets.
pub(crate) fn reset_session_handles() {
    CLOTHING.with(|items| items.borrow_mut().clear());
    VISUALS.with(|visuals| visuals.borrow_mut().clear());
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

/// The `ItemBodyLocation` registry key of a registry id (`base:hat`).
pub fn body_location_key(id: &str) -> Option<Rc<RegistryKey>> {
    let sm = shared_script_manager();
    let l = ResourceLocation::of(id).ok()?;
    let r = sm.registries.get("item_body_location")?;
    let k = r.borrow().get(&l);
    k
}

/// The `ItemBodyLocation` object of a registry id (nil for none).
pub fn body_location_value(id: Option<&str>) -> Value {
    key_value(
        ITEM_BODY_LOCATION_CLASS,
        id.and_then(body_location_key).as_ref(),
    )
}

/// `canBeEquipped()`: the captured instance key for bags and radios, `null`
/// for other items.
pub fn can_be_equipped(item: &pz_inventory::InventoryItem) -> Option<Rc<RegistryKey>> {
    item.can_be_equipped()
}

// ---- ClothingItem ----

/// A `ClothingItem` asset (one object per asset).
pub struct ClothingItemObject(pub Rc<ClothingItem>);

impl LuaObject for ClothingItemObject {
    fn class_name(&self) -> &str {
        CLOTHING_ITEM_CLASS
    }
    fn to_lua_string(&self) -> String {
        format!(
            "ClothingItem{{ Name:{}, GUID:{}}}",
            self.0.name, self.0.guid
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[ASSET_CLASS]
    }
}

pub fn clothing_item_value(c: Option<&ClothingItem>) -> Value {
    let Some(c) = c else { return Value::Nil };
    CLOTHING.with(|m| {
        m.borrow_mut()
            .entry(c.name.clone())
            .or_insert_with(|| Value::Object(Rc::new(ClothingItemObject(Rc::new(c.clone())))))
            .clone()
    })
}

fn clothing_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<ClothingItem>> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<ClothingItemObject>() {
            Some(ci) => Ok(ci.0.clone()),
            None => Err(LuaError::runtime(format!(
                "Expected ClothingItem but got {}",
                o.class_name()
            ))),
        },
        _ => Err(npe()),
    }
}

fn strings(v: &[String]) -> Value {
    Value::Object(Rc::new(JavaList::new(
        v.iter().map(|s| Value::from(s.as_str())).collect(),
    )))
}

fn opt(s: Option<&str>) -> Value {
    s.map(Value::from).unwrap_or_default()
}

fn register_clothing_item(thread: &mut Thread) {
    expose_class(thread, ASSET_CLASS, None);
    let b = expose_class(thread, CLOTHING_ITEM_CLASS, Some(ASSET_CLASS));
    fn ci<R>(c: &NativeCall<'_>, f: impl FnOnce(&ClothingItem) -> R) -> LuaResult<R> {
        with_this::<ClothingItemObject, _>(c, |o| f(&o.0))
    }
    macro_rules! get {
        ($name:literal, |$i:ident| $body:expr) => {
            b.method($name, |c| {
                let v: Value = ci(c, |$i| $body)?;
                Ok(c.push(v))
            });
        };
    }
    get!("hasModel", |i| Value::from(i.has_model()));
    get!("isHat", |i| Value::from(i.is_hat()));
    get!("isMask", |i| Value::from(i.is_mask()));
    get!("getAllowRandomTint", |i| Value::from(i.allow_random_tint));
    get!("getAllowRandomHue", |i| Value::from(i.allow_random_hue));
    get!("getDecalGroup", |i| opt(i.decal_group.as_deref()));
    get!("getBaseTextures", |i| strings(&i.base_textures));
    get!("getTextureChoices", |i| strings(&i.texture_choices));
    get!("getSpawnWith", |i| strings(&i.spawn_with));
    get!("getMaleModel", |i| opt(i.male_model.as_deref()));
    get!("getFemaleModel", |i| opt(i.female_model.as_deref()));
    get!("getAltMaleModel", |i| opt(i.alt_male_model.as_deref()));
    get!("getAltFemaleModel", |i| opt(i.alt_female_model.as_deref()));
    get!("isReady", |_i| Value::from(true));
    get!("isEmpty", |_i| Value::from(false));
    get!("isFailure", |_i| Value::from(false));
    b.method("getModel", |c| {
        let female = crate::expose::bool_arg(c, 1);
        let v = ci(c, |i| {
            opt(if female {
                i.female_model.as_deref()
            } else {
                i.male_model.as_deref()
            })
        })?;
        Ok(c.push(v))
    });
    b.method("getAltModel", |c| {
        let female = crate::expose::bool_arg(c, 1);
        let v = ci(c, |i| {
            opt(if female {
                i.alt_female_model.as_deref()
            } else {
                i.alt_male_model.as_deref()
            })
        })?;
        Ok(c.push(v))
    });
    b.method("toString", |c| {
        let v = with_this::<ClothingItemObject, _>(c, |o| o.to_lua_string())?;
        Ok(c.push(v))
    });
}

// ---- ImmutableColor ----

/// A `zombie.core.ImmutableColor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImmutableColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LuaObject for ImmutableColor {
    fn class_name(&self) -> &str {
        IMMUTABLE_COLOR_CLASS
    }
    fn to_lua_string(&self) -> String {
        use pz_core::config::float_to_string as f;
        format!(
            "ImmutableColor ({},{},{},{})",
            f(self.r),
            f(self.g),
            f(self.b),
            f(self.a)
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ImmutableColor {
    /// `new ImmutableColor(r, g, b)`: clamped to `[0, 1]`, opaque.
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        ImmutableColor {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a: 1.0,
        }
    }
    /// `new ImmutableColor(r, g, b, a)`: capped at 1.
    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        ImmutableColor {
            r: r.min(1.0),
            g: g.min(1.0),
            b: b.min(1.0),
            a: a.min(1.0),
        }
    }
    pub fn value(self) -> Value {
        Value::Object(Rc::new(self))
    }
}

fn color_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ImmutableColor>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ImmutableColor>() {
            Some(k) => Ok(Some(*k)),
            None => Err(LuaError::runtime(format!(
                "Expected ImmutableColor but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected ImmutableColor but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

// ---- ItemVisual ----

/// An item's `ItemVisual`.
pub struct ItemVisualObject(pub ItemRef);

impl LuaObject for ItemVisualObject {
    fn class_name(&self) -> &str {
        ITEM_VISUAL_CLASS
    }
    fn to_lua_string(&self) -> String {
        format!("ItemVisual@{:x}", Rc::as_ptr(&self.0) as usize)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn visual_value(item: &ItemRef) -> Value {
    let key = Rc::as_ptr(item) as usize;
    VISUALS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((w, v)) = m.get(&key) {
            if w.upgrade().is_some_and(|i| Rc::ptr_eq(&i, item)) {
                return v.clone();
            }
        }
        m.retain(|_, (w, _)| w.strong_count() > 0);
        let v = Value::Object(Rc::new(ItemVisualObject(item.clone())));
        m.insert(key, (Rc::downgrade(item), v.clone()));
        v
    })
}

/// `InventoryItem.getVisual()` through the installed environment.
pub fn get_visual(item: &ItemRef) -> LuaResult<Value> {
    let e = item_env();
    let owner = e.owner(item);
    if e.context().assets().is_some() { crate::bind_items::ensure_clothing_mod_data(item)?; }
    let result = {
        let mut body = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item visual body").to_lua_error())?;
        body.try_get_visual(e.context().assets(), owner.as_ref(), e.visual_rng()).map(|value| value.is_some())
    };
    pz_inventory::shared::observe_item(item);
    let has = result.map_err(|error| error.to_lua_error())?;
    Ok(if has {
        visual_value(item)
    } else {
        Value::Nil
    })
}

fn clothing_of(item: &ItemRef) -> LuaResult<Option<ClothingItem>> {
    let e = item_env();
    let owner = e.owner(item);
    if e.context().assets().is_some() { crate::bind_items::ensure_clothing_mod_data(item)?; }
    let result = {
        let body = item.try_borrow().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item visual body").to_lua_error())?;
        body.try_get_clothing_item(e.context().assets(), owner.as_ref()).map(|value| value.cloned())
    };
    result.map_err(|error| error.to_lua_error())
}

/// `ItemVisual.getClothingItem()`: by the visual's alternate model name.
fn visual_clothing_item(item: &ItemRef) -> Option<ClothingItem> {
    let e = item_env();
    let a = e.context().assets()?;
    let i = item.borrow();
    let t = &i.template;
    let alt = i
        .visual
        .as_ref()
        .map(|v| v.alternate_model_name.clone())
        .unwrap_or_default();
    let name = if alt.eq_ignore_ascii_case("LeftHand") {
        t.replace_second_hand
            .as_ref()
            .and_then(|r| r.clothing_item_name.clone())
    } else if alt.eq_ignore_ascii_case("RightHand") {
        t.replace_primary_hand
            .as_ref()
            .and_then(|r| r.clothing_item_name.clone())
    } else if alt.eq_ignore_ascii_case("WithoutDrainable")
        && t.without_drainable_clothing_item.is_some()
    {
        t.without_drainable_clothing_item.clone()
    } else {
        t.clothing_item.clone()
    };
    a.clothing_items.get(name.as_deref()?).cloned()
}

fn vis<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut ItemVisual, &mut pz_inventory::InventoryItem) -> R,
) -> LuaResult<R> {
    let item = with_this::<ItemVisualObject, _>(c, |o| o.0.clone())?;
    let mut i = item.borrow_mut();
    let mut v = i.visual.take().ok_or_else(npe)?;
    let r = f(&mut v, &mut i);
    i.visual = Some(v);
    Ok(r)
}

fn register_item_visual(thread: &mut Thread) {
    let b = expose_class(thread, ITEM_VISUAL_CLASS, None);
    b.method("getItemType", |c| {
        let v = vis(c, |v, _| v.full_type.clone())?;
        Ok(c.push(v))
    });
    b.method("getClothingItemName", |c| {
        let v = vis(c, |v, _| v.clothing_item_name.clone())?;
        Ok(c.push(v))
    });
    b.method("getAlternateModelName", |c| {
        let v = vis(c, |v, _| v.alternate_model_name.clone())?;
        Ok(c.push(v))
    });
    b.method("getInventoryItem", |c| {
        let item = with_this::<ItemVisualObject, _>(c, |o| o.0.clone())?;
        Ok(c.push(item_value(&item)))
    });
    b.method("getClothingItem", |c| {
        let item = with_this::<ItemVisualObject, _>(c, |o| o.0.clone())?;
        Ok(c.push(clothing_item_value(visual_clothing_item(&item).as_ref())))
    });
    // `getTextureChoice()` / `getTextureChoice(clothingItem)` (the name,
    // picking an index when unset), and the same for the base texture.
    b.method("getTextureChoice", |c| {
        if c.nargs() > 1 {
            let ci = clothing_arg(c, 1)?;
            let rng = item_env();
            let v = vis(c, |v, _| {
                if ci.texture_choices.is_empty() {
                    v.texture_choice = -1;
                    return Value::Nil;
                }
                let n = ci.texture_choices.len();
                if v.texture_choice < 0 || v.texture_choice as usize >= n {
                    v.texture_choice = rng.visual_rng().next(n as i32) as i8;
                }
                Value::from(ci.texture_choices[v.texture_choice as usize].as_str())
            })?;
            return Ok(c.push(v));
        }
        let v = vis(c, |v, _| v.texture_choice as i32)?;
        Ok(c.push(v))
    });
    b.method("getBaseTexture", |c| {
        if c.nargs() > 1 {
            let ci = clothing_arg(c, 1)?;
            let rng = item_env();
            let v = vis(c, |v, _| {
                if ci.base_textures.is_empty() {
                    v.base_texture = -1;
                    return Value::Nil;
                }
                let n = ci.base_textures.len();
                if v.base_texture < 0 || v.base_texture as usize >= n {
                    v.base_texture = rng.visual_rng().next(n as i32) as i8;
                }
                Value::from(ci.base_textures[v.base_texture as usize].as_str())
            })?;
            return Ok(c.push(v));
        }
        let v = vis(c, |v, _| v.base_texture as i32)?;
        Ok(c.push(v))
    });
    b.method("setTextureChoice", |c| {
        let n = int_arg(c, 1);
        vis(c, |v, _| v.texture_choice = n as i8)?;
        Ok(0)
    });
    b.method("setBaseTexture", |c| {
        let n = int_arg(c, 1);
        vis(c, |v, _| v.base_texture = n as i8)?;
        Ok(0)
    });
    b.method("getTint", |c| {
        if c.nargs() > 1 {
            let ci = clothing_arg(c, 1)?;
            let rng = item_env();
            let v = vis(c, |v, i| {
                if i.custom_color {
                    // `new ImmutableColor(item.getR(), getG(), getB())`.
                    return ImmutableColor::rgb(i.col[0], i.col[1], i.col[2]);
                }
                let t = if ci.allow_random_tint {
                    *v.tint
                        .get_or_insert_with(|| rng.visual_rng().random_color())
                } else {
                    v.tint = Some([1.0, 1.0, 1.0]);
                    [1.0, 1.0, 1.0]
                };
                ImmutableColor::rgba(t[0], t[1], t[2], 1.0)
            })?;
            return Ok(c.push(v.value()));
        }
        let v = vis(c, |v, _| v.tint)?;
        Ok(c.push(
            v.map(|t| ImmutableColor::rgba(t[0], t[1], t[2], 1.0).value())
                .unwrap_or_default(),
        ))
    });
    b.method("setTint", |c| {
        let t = color_arg(c, 1)?;
        vis(c, |v, _| v.tint = t.map(|k| [k.r, k.g, k.b]))?;
        Ok(0)
    });
    b.method("getHue", |c| {
        if c.nargs() > 1 {
            let ci = clothing_arg(c, 1)?;
            let rng = item_env();
            let v = vis(c, |v, _| {
                if ci.allow_random_hue {
                    if v.hue == f32::INFINITY {
                        v.hue = rng.visual_rng().next(200) as f32 / 100.0 - 1.0;
                    }
                } else {
                    v.hue = 0.0;
                }
                v.hue
            })?;
            return Ok(c.push(v as f64));
        }
        let v = vis(c, |v, _| v.hue)?;
        Ok(c.push(v as f64))
    });
    b.method("setHue", |c| {
        let h = num_arg(c, 1) as f32;
        vis(c, |v, _| v.set_hue(h))?;
        Ok(0)
    });
    b.method("getDecal", |c| {
        let ci = clothing_arg(c, 1)?;
        let rng = item_env();
        let v = vis(c, |v, _| {
            match ci.decal_group.as_deref().filter(|g| !g.trim().is_empty()) {
                None => {
                    v.decal = None;
                    None
                }
                Some(g) => {
                    if v.decal.is_none() {
                        v.decal = rng.visual_rng().random_decal(g);
                    }
                    v.decal.clone()
                }
            }
        })?;
        Ok(c.push(opt(v.as_deref())))
    });
    b.method("setDecal", |c| {
        let d = str_arg(c, 1).map(|s| s.to_string());
        vis(c, |v, _| v.decal = d)?;
        Ok(0)
    });
    b.method("pickUninitializedValues", |c| {
        let ci = match c.arg(1) {
            Value::Nil => return Ok(0),
            _ => clothing_arg(c, 1)?,
        };
        let rng = item_env();
        vis(c, |v, i| {
            v.pick_uninitialized_values(&ci, i.custom_color, rng.visual_rng())
        })?;
        Ok(0)
    });
    type Part = fn(&ItemVisual, usize) -> f32;
    let parts: [(&str, Part); 6] = [
        ("getBlood", ItemVisual::get_blood),
        ("getDirt", ItemVisual::get_dirt),
        ("getHole", ItemVisual::get_hole),
        ("getBasicPatch", ItemVisual::get_basic_patch),
        ("getDenimPatch", ItemVisual::get_denim_patch),
        ("getLeatherPatch", ItemVisual::get_leather_patch),
    ];
    for (name, g) in parts {
        b.method(name, move |c| {
            let p = part_index(c, 1);
            let v = vis(c, |v, _| g(v, p))?;
            Ok(c.push(v as f64))
        });
    }
    b.method("getHolesNumber", |c| {
        let v = vis(c, |v, _| v.get_holes_number())?;
        Ok(c.push(v))
    });
    b.method("getBasicPatchesNumber", |c| {
        let v = vis(c, |v, _| v.get_basic_patches_number())?;
        Ok(c.push(v))
    });
    b.method("getTotalBlood", |c| {
        let v = vis(c, |v, _| v.get_total_blood())?;
        Ok(c.push(v as f64))
    });
    b.method("removeBlood", |c| {
        vis(c, |v, _| v.remove_blood())?;
        Ok(0)
    });
    b.method("removeDirt", |c| {
        vis(c, |v, _| v.remove_dirt())?;
        Ok(0)
    });
    b.method("toString", |c| {
        let v = with_this::<ItemVisualObject, _>(c, |o| o.to_lua_string())?;
        Ok(c.push(v))
    });
}

/// A `BloodBodyPartType` (enum object) or its index.
fn part_index(c: &NativeCall<'_>, i: usize) -> usize {
    if let Value::Object(o) = c.arg(i) {
        if let Some(e) = o.as_any().downcast_ref::<crate::expose::JavaEnum>() {
            return e.ordinal.max(0) as usize;
        }
    }
    int_arg(c, i).max(0) as usize
}

// ---- InventoryItem ----

fn this_item(c: &NativeCall<'_>) -> LuaResult<ItemRef> {
    item_arg(c, 0)
}

fn register_item_methods(thread: &mut Thread) {
    let b = expose_class(thread, INVENTORY_ITEM_CLASS, None);
    b.method("getVisual", |c| {
        let item = this_item(c)?;
        Ok(c.push(get_visual(&item)?))
    });
    b.method("getClothingItem", |c| {
        let item = this_item(c)?;
        Ok(c.push(clothing_item_value(clothing_of(&item)?.as_ref())))
    });
    b.method("allowRandomTint", |c| {
        let item = this_item(c)?;
        Ok(c.push(clothing_of(&item)?.is_some_and(|ci| ci.allow_random_tint)))
    });
    b.method("synchWithVisual", |c| {
        let item = this_item(c)?;
        let e = item_env();
        let owner = e.owner(&item);
        if e.context().assets().is_some() { crate::bind_items::ensure_clothing_mod_data(&item)?; }
        let result = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item visual body").to_lua_error())?
            .try_synch_with_visual(e.context().assets(), owner.as_ref(), e.visual_rng());
        pz_inventory::shared::observe_item(&item);
        result.map_err(|error| error.to_lua_error())?;
        Ok(0)
    });
    b.method("canBeEquipped", |c| {
        let item = this_item(c)?;
        let k = can_be_equipped(&item.borrow());
        Ok(c.push(key_value(ITEM_BODY_LOCATION_CLASS, k.as_ref())))
    });
    b.method("isCustomColor", |c| {
        let item = this_item(c)?;
        let v = item.borrow().custom_color;
        Ok(c.push(v))
    });
}

// ---- globals ----

/// `instanceItem(item)`: a script `Item` (its `moduleDotType`), an
/// `ItemKey` or any object by its `toString`, or a type string with an
/// optional `useDelta`.
fn instance_item(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let (name, use_delta) = match c.arg(0) {
        Value::Nil => return Err(npe()),
        Value::Object(o) => {
            let name = match o
                .as_any()
                .downcast_ref::<crate::bind_scripts::ScriptHandle>()
            {
                Some(h) => {
                    h.read(|i: &pz_scripts::objects::items::Item| i.module_dot_type.clone())?
                }
                None => o.to_lua_string(),
            };
            (name, 1.0)
        }
        _ => {
            let name = str_arg(c, 0).ok_or_else(npe)?.to_string();
            let d = if c.nargs() > 1 {
                num_arg(c, 1) as f32
            } else {
                1.0
            };
            (name, d)
        }
    };
    let item = crate::bind_items::try_create_item(&name, use_delta)?;
    if let Some(item) = &item {
        crate::bind_items::initialise_item(c.thread, item);
    }
    Ok(c.push(item.map(|i| item_value(&i)).unwrap_or_default()))
}

/// `getAllDecalNamesForItem(item)`: the decals of the item's clothing
/// item's decal group (an empty list without one).
fn all_decal_names(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut out = Vec::new();
    if let Value::Object(o) = c.arg(0) {
        if let Some(l) = o.as_any().downcast_ref::<LuaInventoryItem>() {
            let group = clothing_of(&l.item)?
                .and_then(|ci| ci.decal_group)
                .filter(|g| !g.trim().is_empty());
            if let Some(g) = group {
                let pop = crate::bind_appearance::shared_population();
                let pop = pop.borrow();
                if let Some(d) = pop.clothing_decals.as_ref() {
                    if let Some(grp) = d.find_group(&g) {
                        grp.get_decals(d, &mut out);
                    }
                }
            }
        }
    }
    Ok(c.push(strings(&out)))
}

/// `WornItems.setFromItemVisuals(itemVisuals)` on a descriptor: clears
/// the worn items, then for each visual creates its item type
/// (`CreateItem`), copies the visual in (`ItemVisual.copyFrom`, after
/// picking the source's unset values), runs `synchWithVisual` and wears it
/// at its body location (clothing) or `canBeEquipped()` location.
pub fn set_worn_items_from_item_visuals(
    thread: &mut Thread,
    desc: &Rc<RefCell<LuaSurvivorDesc>>,
    visuals: &[ItemVisual],
) -> Result<(), pz_scripts::ScriptError> {
    let retired = {
        let mut desc = desc.try_borrow_mut().map_err(|_| pz_scripts::ScriptError::new(
            "descriptor is borrowed while clearing worn items",
        ))?;
        desc.worn_items.take_items_for_retirement()
    };
    if !retired.is_empty() {
        let mut pins = pz_entity::RetiredComponents::new();
        pins.push_payload(Box::new(retired));
        pz_inventory::component_host::retire_components(pins);
    }
    let e = item_env();
    for other in visuals {
        let Some(item) = crate::bind_items::try_create_item(&other.full_type, 1.0)
            .map_err(|error| pz_scripts::ScriptError::new(format!("{error:?}")))? else {
            continue;
        };
        // The method may still be inside its caller's world/sprite scope.
        // Preserve the actual incoming item even if a later phase fails.
        let mut pins = pz_entity::RetiredComponents::new();
        pins.push_owner(pz_entity::EntityOwner::from_ref(&item));
        pz_inventory::component_host::retire_components(pins);
        // Initialization may reenter the same descriptor. Its worn items
        // have already been cleared; reacquire its current fields afterward.
        crate::bind_items::initialise_item(thread, &item);
        let owner = e.owner(&item);
        if e.context().assets().is_some() {
            crate::bind_items::ensure_clothing_mod_data(&item)
                .map_err(|error| pz_scripts::ScriptError::new(format!("{error:?}")))?;
        }
        let visual_result = (|| {
            let mut i = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item visual copy"))?;
            let assets = e.context().assets();
            if i.try_get_visual(assets, owner.as_ref(), e.visual_rng())?
                .is_some()
            {
                copy_visual_from(&mut i, other, assets, owner.as_ref(), e.as_ref());
                i.try_synch_with_visual(assets, owner.as_ref(), e.visual_rng())?;
            }
            Ok::<(), pz_inventory::moddata::ModDataError>(())
        })();
        pz_inventory::shared::observe_item(&item);
        visual_result.map_err(|error| pz_scripts::ScriptError::new(error.to_string()))?;
        let i = item.borrow();
        let location = if matches!(
            i.kind,
            ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
        ) {
            i.get_body_location().and_then(body_location_key)
        } else {
            can_be_equipped(&i)
        };
        drop(i);
        let prior = desc.try_borrow().map_err(|_| pz_scripts::ScriptError::new(
            "descriptor is borrowed while retaining worn items",
        ))?.worn_items.items().to_vec();
        if !prior.is_empty() {
            let mut pins = pz_entity::RetiredComponents::new();
            pins.push_payload(Box::new(prior));
            pz_inventory::component_host::retire_components(pins);
        }
        let result = {
            let mut desc = desc.try_borrow_mut().map_err(|_| pz_scripts::ScriptError::new(
                "descriptor is borrowed while setting a worn item",
            ))?;
            desc.worn_items.set_item(location, Some(LuaItem(item_value(&item))))
        };
        result?;
    }
    Ok(())
}

/// `ItemVisual.copyFrom(other)` into `item`'s visual.
fn copy_visual_from(
    item: &mut pz_inventory::InventoryItem,
    other: &ItemVisual,
    assets: Option<&pz_inventory::ItemAssets>,
    _owner: &dyn ItemOwner,
    e: &dyn crate::bind_items::ItemLuaEnv,
) {
    let mut src = other.clone();
    let ci = assets.and_then(|a| {
        let t = &item.template;
        let name = if src.alternate_model_name.eq_ignore_ascii_case("LeftHand") {
            t.replace_second_hand
                .as_ref()
                .and_then(|r| r.clothing_item_name.clone())
        } else if src.alternate_model_name.eq_ignore_ascii_case("RightHand") {
            t.replace_primary_hand
                .as_ref()
                .and_then(|r| r.clothing_item_name.clone())
        } else {
            t.clothing_item.clone()
        };
        a.clothing_items.get(name.as_deref()?)
    });
    if let Some(ci) = ci {
        // The source has no inventory item, so its tint is never custom.
        src.pick_uninitialized_values(ci, false, e.visual_rng());
    }
    if let Some(v) = item.visual.as_mut() {
        *v = src;
    }
}

pub fn register(thread: &mut Thread) {
    register_clothing_item(thread);
    register_item_visual(thread);
    register_item_methods(thread);
    global(thread, "instanceItem", instance_item);
    global(thread, "getAllDecalNamesForItem", all_decal_names);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_items::{install_item_env, GameItemEnv};
    use pz_inventory::{ClothingItems, ItemAssets};
    use pz_kahlua::TableRef;
    use pz_scripts::ScriptManager;

    const SCRIPTS: &str = "module Base {
        item Tshirt_Test {
            DisplayCategory = Clothing, ItemType = base:clothing, Icon = TshirtGeneric,
            BodyLocation = base:tshirt, ClothingItem = Tshirt_Test,
        }
        item Rock { ItemType = base:normal, Weight = 2, CanBeEquipped = base:back, }
        item Bag_Test { ItemType = base:container, CanBeEquipped = base:back, }
        item Radio_Test { ItemType = base:radio, CanBeEquipped = base:back, }
    }";

    fn scripts() -> Rc<ScriptManager> {
        // One directory per call: tests run in parallel.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("pz-bind-item-visual-{}-{n}", std::process::id()));
        let p = dir.join("media/scripts/test.txt");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, SCRIPTS).unwrap();
        let sm = ScriptManager::new();
        for (reg, id) in [
            ("item_type", "base:clothing"),
            ("item_type", "base:normal"),
            ("item_type", "base:container"),
            ("item_type", "base:radio"),
            ("item_body_location", "base:tshirt"),
            ("item_body_location", "base:back"),
            ("item_body_location", "base:replacement"),
        ] {
            sm.registries
                .get(reg)
                .unwrap()
                .borrow_mut()
                .register_id(id, true)
                .unwrap();
        }
        let mut st = sm.statics.items.borrow_mut();
        st.constants
            .insert("item_type", "CLOTHING", "base:clothing");
        st.constants.insert("item_type", "NORMAL", "base:normal");
        st.constants
            .insert("item_type", "CONTAINER", "base:container");
        st.constants.insert("item_type", "RADIO", "base:radio");
        drop(st);
        sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        Rc::new(sm)
    }

    fn assets() -> ItemAssets {
        let mut c = ClothingItem::new("Tshirt_Test");
        c.male_model = Some("Skinned/Clothes/Tshirt".into());
        c.female_model = Some("Skinned/Clothes/Tshirt".into());
        c.texture_choices = vec!["A".into(), "B".into(), "C".into()];
        c.allow_random_tint = true;
        let mut items = ClothingItems::default();
        items.insert("Tshirt_Test", "guid-1", Some(c));
        ItemAssets {
            clothing_items: items,
            ..Default::default()
        }
    }

    fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone())
            .expect("compile");
        t.pcall(&f, &[])
            .map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))
    }

    fn setup(seed: i64) -> Thread {
        let sm = scripts();
        crate::bind_registry::install_script_manager(sm.clone());
        let mut env = GameItemEnv::new(Some(assets()));
        env.rand = Some(pz_core::random::Rand::new(seed));
        install_item_env(Rc::new(env));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_registry::register(&mut t, sm);
        crate::bind_items::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        register(&mut t);
        t
    }

    #[test]
    fn session_reset_reloads_same_name_clothing_and_releases_old_visual_items() {
        reset_session_handles();
        let mut thread = setup(17);
        let mut first_asset = ClothingItem::new("SessionResetClothing");
        first_asset.male_model = Some("ModelA".into());
        let first = clothing_item_value(Some(&first_asset));
        thread.globals().set_str("firstClothing", first.clone());
        let item = crate::bind_items::create_item("Base.Tshirt_Test", 1.0).unwrap();
        let weak_item = Rc::downgrade(&item);
        let visual = get_visual(&item).unwrap();
        assert!(matches!(visual, Value::Object(_)));
        drop(visual);
        drop(item);
        assert!(
            weak_item.upgrade().is_some(),
            "fixture did not retain an item through the visual cache"
        );

        crate::session_reset::reset_session_bindings();
        assert!(
            weak_item.upgrade().is_none(),
            "session reset retained an old visual's item"
        );
        let mut second_asset = ClothingItem::new("SessionResetClothing");
        second_asset.male_model = Some("ModelB".into());
        let second = clothing_item_value(Some(&second_asset));
        assert!(
            !first.raw_equals(&second),
            "new assets reused an old clothing wrapper"
        );
        thread.globals().set_str("secondClothing", second);
        run(
            &mut thread,
            r#"
            assert(firstClothing ~= secondClothing)
            assert(firstClothing:getMaleModel() == 'ModelA')
            assert(secondClothing:getMaleModel() == 'ModelB')
        "#,
        )
        .unwrap();
    }

    #[test]
    fn factory_equipped_key_is_captured_before_protected_lua_create() {
        let mut thread = setup(42);
        let sm = shared_script_manager();
        let back = body_location_key("base:back").unwrap();
        let replacement = body_location_key("base:replacement").unwrap();
        thread.globals().set_str(
            "capturedKey",
            key_value(ITEM_BODY_LOCATION_CLASS, Some(&back)),
        );
        run(
            &mut thread,
            r#"
            creationCalls = 0
            function EquipmentCreate(item)
                assert(item:canBeEquipped() == capturedKey)
                creationCalls = creationCalls + 1
            end
        "#,
        )
        .unwrap();
        for ty in ["Base.Bag_Test", "Base.Radio_Test"] {
            let script = sm
                .find_item(ty, false)
                .unwrap()
                .downcast::<pz_scripts::objects::items::Item>()
                .unwrap();
            script.borrow_mut().lua_create = Some("EquipmentCreate".into());
            let item = crate::bind_items::create_item(ty, 1.0).unwrap();
            assert!(Rc::ptr_eq(&item.borrow().can_be_equipped().unwrap(), &back));
            // Native InstanceItem captures this key before initialiseItem.
            // Even the same script's later mutation cannot change the item.
            script.borrow_mut().can_be_equipped = Some(replacement.clone());
            crate::bind_items::initialise_item(&mut thread, &item);
            assert!(item.borrow().is_initialised);
            assert!(Rc::ptr_eq(&can_be_equipped(&item.borrow()).unwrap(), &back));
        }
        assert_eq!(
            run(&mut thread, "return creationCalls").unwrap()[0].as_num(),
            Some(2.0)
        );
        let base = crate::bind_items::create_item("Base.Rock", 1.0).unwrap();
        assert!(base
            .borrow()
            .template
            .script
            .as_ref()
            .unwrap()
            .0
            .borrow()
            .can_be_equipped
            .is_some());
        assert!(
            base.borrow().can_be_equipped().is_none(),
            "base InventoryItem must return null despite a script key"
        );
    }

    #[test]
    fn equipped_query_retains_key_after_same_name_script_manager_replacement() {
        let mut thread = setup(42);
        let old_back = body_location_key("base:back").unwrap();
        let bag = crate::bind_items::create_item("Base.Bag_Test", 1.0).unwrap();
        let radio = crate::bind_items::create_item("Base.Radio_Test", 1.0).unwrap();
        thread.globals().set_str("retainedBag", item_value(&bag));
        thread
            .globals()
            .set_str("retainedRadio", item_value(&radio));
        thread.globals().set_str(
            "oldKey",
            key_value(ITEM_BODY_LOCATION_CLASS, Some(&old_back)),
        );
        let replacement = scripts();
        crate::bind_registry::install_script_manager(replacement);
        let new_back = body_location_key("base:back").unwrap();
        assert!(!Rc::ptr_eq(&old_back, &new_back));
        run(
            &mut thread,
            r#"
            assert(retainedBag:canBeEquipped() == oldKey)
            assert(retainedRadio:canBeEquipped() == oldKey)
        "#,
        )
        .unwrap();
        assert!(Rc::ptr_eq(
            &can_be_equipped(&bag.borrow()).unwrap(),
            &old_back
        ));
        assert!(Rc::ptr_eq(
            &can_be_equipped(&radio.borrow()).unwrap(),
            &old_back
        ));
        let new_bag = crate::bind_items::create_item("Base.Bag_Test", 1.0).unwrap();
        assert!(Rc::ptr_eq(
            &new_bag.borrow().can_be_equipped().unwrap(),
            &new_back
        ));
    }

    #[test]
    fn instance_item_in_lua() {
        let mut t = setup(42);
        run(
            &mut t,
            r#"
            assert(instanceItem("Base.NoSuchItem") == nil)
            local rock = instanceItem("Rock")
            assert(rock:getFullType() == "Base.Rock" and rock:getVisual() == nil and rock:getClothingItem() == nil)
            assert(rock:canBeEquipped() == nil and rock:getBodyLocation() == nil)
            local it = instanceItem("Base.Tshirt_Test")
            assert(instanceof(it, "Clothing") and it:IsClothing())
            assert(it:getBodyLocation() == ItemBodyLocation.get(ResourceLocation.of("base:tshirt")))
            assert(it:isBodyLocation(it:getBodyLocation()))
            local v = it:getVisual()
            assert(v ~= nil and v == it:getVisual() and v:getInventoryItem() == it)
            assert(v:getItemType() == "Base.Tshirt_Test" and v:getClothingItemName() == "Tshirt_Test")
            local ci = it:getClothingItem()
            assert(ci == v:getClothingItem() and ci:hasModel() and ci:getAllowRandomTint())
            assert(ci:getTextureChoices():size() == 3 and ci:getBaseTextures():size() == 0)
            assert(it:allowRandomTint())
            local tc = v:getTextureChoice()
            assert(tc >= 0 and tc < 3 and v:getTextureChoice(ci) == ci:getTextureChoices():get(tc))
            v:setTextureChoice(2)
            assert(v:getTextureChoice() == 2)
            local tint = v:getTint(ci)
            assert(tint ~= nil and tint:getAlphaFloat() == 1)
            v:setTint(ImmutableColor.new(0.5, 0.25, 0.125, 1))
            assert(v:getTint(ci):getRedFloat() == 0.5 and v:getTint():getBlueFloat() == 0.125)
            assert(v:getDecal(ci) == nil)
            assert(getAllDecalNamesForItem(it):size() == 0)
            assert(instanceItem(it:getFullType()) ~= it)
            "#,
        )
        .unwrap();
    }

    #[test]
    fn seeded_creation_is_reproducible() {
        let draw = |seed| {
            let mut t = setup(seed);
            let r = run(
                &mut t,
                "local it = instanceItem('Base.Tshirt_Test') local v = it:getVisual() local c = v:getTint()
                 return it:getID(), v:getTextureChoice(), c:getRedFloat(), c:getGreenFloat(), c:getBlueFloat()",
            )
            .unwrap();
            r.iter()
                .map(|v| pz_kahlua::exec::raw_tonumber(v).unwrap())
                .collect::<Vec<f64>>()
        };
        assert_eq!(draw(7), draw(7));
        assert_ne!(draw(7), draw(8));
        // Rand draws in `InstanceItem` order: the colour (3), the
        // `OutfitRNG` seed, then the id.
        let r = pz_core::random::Rand::new(7);
        for _ in 0..3 {
            r.next(255);
        }
        let seed = r.next(i32::MAX);
        let id = r.next(2146250223) + 1233423;
        assert_eq!(draw(7)[0], id as f64);
        // `pickUninitializedValues`: the random tint, then the texture.
        pz_core::random::outfit_rng::set_seed(seed as i64);
        let tint = pz_core::random::outfit_rng::random_immutable_color(true);
        let tc = pz_core::random::outfit_rng::next(3);
        assert_eq!(draw(7)[1], tc as f64);
        assert_eq!(draw(7)[2..], tint.map(|c| c as f64));
    }

    #[test]
    fn dress_from_item_visuals() {
        let mut t = setup(3);
        let sm = shared_script_manager();
        let mut chars = pz_characters::Characters::default();
        let tshirt = body_location_key("base:tshirt");
        let group = chars.body_locations.get_group("Human").unwrap();
        group
            .borrow_mut()
            .get_or_create_location(tshirt.clone())
            .unwrap();
        let desc = Rc::new(RefCell::new(LuaSurvivorDesc::new(&mut chars, &sm).unwrap()));
        let prior = crate::bind_items::try_create_item("Base.Rock", 1.0).unwrap().unwrap();
        desc.borrow_mut().worn_items.set_item(tshirt.clone(), Some(LuaItem(item_value(&prior)))).unwrap();
        sm.get::<pz_scripts::objects::items::Item>(pz_scripts::ScriptType::Item, "Base.Tshirt_Test")
            .unwrap().borrow_mut().lua_create = Some("wornInitProbe".into());
        let created = Rc::new(RefCell::new(None::<ItemRef>));
        let (probe_desc, probe_created) = (desc.clone(), created.clone());
        t.globals().set_str("wornInitProbe", Value::Native(pz_kahlua::NativeFunction::new("wornInitProbe", move |call| {
            let desc = probe_desc.try_borrow_mut().expect("initialization must release descriptor guards");
            assert!(desc.worn_items.items().is_empty(), "native clear precedes the first factory callback");
            drop(desc);
            let Value::Object(object) = call.arg(0) else { panic!("actual initialization receiver") };
            let item = &object.as_any().downcast_ref::<LuaInventoryItem>().unwrap().item;
            assert!(item.borrow().is_initialised, "native flag precedes LuaCreate");
            item.borrow_mut().condition = 6;
            *probe_created.borrow_mut() = Some(item.clone());
            Ok(0)
        })));
        let mut v = ItemVisual::new("Base.Tshirt_Test");
        v.texture_choice = 1;
        v.tint = Some([0.25, 0.5, 0.75]);
        let visuals = [ItemVisual::new("Base.Gone"), v];
        set_worn_items_from_item_visuals(&mut t, &desc, &visuals).unwrap();
        let desc = desc.borrow();
        let worn = desc.worn_items.items();
        assert_eq!(worn.len(), 1);
        assert!(worn[0].location == tshirt);
        let Value::Object(o) = &worn[0].item.0 else {
            panic!()
        };
        let item = o
            .as_any()
            .downcast_ref::<LuaInventoryItem>()
            .unwrap()
            .item
            .borrow();
        assert_eq!(item.condition, 6, "callback mutation remains on the actual worn item");
        let worn_ref = &o.as_any().downcast_ref::<LuaInventoryItem>().unwrap().item;
        assert!(Rc::ptr_eq(created.borrow().as_ref().unwrap(), worn_ref));
        let vis = item.visual.as_ref().unwrap();
        assert_eq!((vis.texture_choice, vis.tint), (1, Some([0.25, 0.5, 0.75])));
        assert_eq!(item.col, [0.25, 0.5, 0.75, 1.0]);
    }
}
