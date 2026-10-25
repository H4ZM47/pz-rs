//! `zombie.inventory.InventoryItem` and its subclasses exposed to Lua, over
//! [`pz_inventory`] items, with the original class names, superclass
//! chains (`InventoryItem < GameEntity`, `Radio < Moveable`,
//! `AlarmClockClothing < Clothing`) and the methods `pz-inventory`
//! supports. `WeaponType` is exposed as an enum.
//!
//! An item is shared as an [`ItemRef`]; [`item_value`] gives one Lua
//! object per item while that object lives, so Lua `==` is Java identity.
//! Methods that read the holder or the game state (weights, `isEquipped`,
//! wetness, `getVisual`, `Use`) go through the installed [`ItemLuaEnv`]
//! ([`install_item_env`]); without one they see an item in a container
//! without a parent, no scripts and zero globals.
//!
//! Not exposed yet: methods returning or taking objects that have no Lua
//! binding here (`getScriptItem`, `getModData`, `getContainer`,
//! `getWorldItem`, `getVisual`'s `ItemVisual`, `getInventory`,
//! `getTags`), the registry-object returns (`getBodyLocation` returns the
//! registry id string, `getBloodClothingType` a list of names, `hasTag`
//! takes ids or objects whose `tostring` is the id).

use crate::bind_java_util::JavaList;
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg};
use pz_inventory::api::Offline;
use pz_inventory::context::NoOwner;
use pz_inventory::gameplay::{ConditionOutcome, ItemError, UseOutcome};
use pz_inventory::item::ItemKind;
use pz_inventory::{InventoryItem, ItemClass, ItemContext, ItemOwner};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const GAME_ENTITY_CLASS: &str = "zombie.entity.GameEntity";
pub const INVENTORY_ITEM_CLASS: &str = "zombie.inventory.InventoryItem";
pub const WEAPON_TYPE_CLASS: &str = "zombie.inventory.types.WeaponType";
const WEAPON_TYPES: [&str; 10] = [
    "UNARMED",
    "TWO_HANDED",
    "ONE_HANDED",
    "HEAVY",
    "KNIFE",
    "SPEAR",
    "HANDGUN",
    "FIREARM",
    "THROWING",
    "CHAINSAW",
];

/// A shared item (the original's object identity).
pub type ItemRef = Rc<RefCell<InventoryItem>>;

/// What an item's side effects did, for the installed environment.
pub enum ItemOutcome {
    Use(Box<UseOutcome>),
    Condition(ConditionOutcome),
}

/// The holder and game state the item methods read from Lua; installed by
/// the character simulation.
pub trait ItemLuaEnv {
    fn context(&self) -> &dyn ItemContext;
    /// The item's holder (its container's parent).
    fn owner(&self, item: &ItemRef) -> Rc<dyn ItemOwner>;
    /// Called after `Use`, `setCondition` and wetness updates with what
    /// the container or character must do (add or remove items, play
    /// sounds, call `OnBreak`).
    fn outcome(&self, _item: &ItemRef, _o: ItemOutcome) {}
}

struct DefaultEnv;
impl ItemLuaEnv for DefaultEnv {
    fn context(&self) -> &dyn ItemContext {
        &Offline
    }
    fn owner(&self, _item: &ItemRef) -> Rc<dyn ItemOwner> {
        Rc::new(NoOwner)
    }
}

thread_local! {
    static ENV: RefCell<Rc<dyn ItemLuaEnv>> = RefCell::new(Rc::new(DefaultEnv));
    static OBJECTS: RefCell<HashMap<usize, Weak<dyn LuaObject>>> = RefCell::new(HashMap::new());
}

/// Installs the environment the item methods use.
pub fn install_item_env(env: Rc<dyn ItemLuaEnv>) {
    ENV.with(|e| *e.borrow_mut() = env);
}

fn env() -> Rc<dyn ItemLuaEnv> {
    ENV.with(|e| e.borrow().clone())
}

/// The installed environment (for bindings outside this module that use
/// items, such as `BodyDamage:JustTookPill`).
pub fn item_env() -> Rc<dyn ItemLuaEnv> {
    env()
}

/// The Java class of an item.
pub fn class_of(item: &InventoryItem) -> &'static str {
    match item.template.class {
        ItemClass::Normal => "zombie.inventory.types.ComboItem",
        ItemClass::Key => "zombie.inventory.types.Key",
        ItemClass::WeaponPart => "zombie.inventory.types.WeaponPart",
        ItemClass::Container => "zombie.inventory.types.InventoryContainer",
        ItemClass::Food => "zombie.inventory.types.Food",
        ItemClass::Literature => "zombie.inventory.types.Literature",
        ItemClass::AlarmClock => "zombie.inventory.types.AlarmClock",
        ItemClass::AlarmClockClothing => "zombie.inventory.types.AlarmClockClothing",
        ItemClass::Weapon => "zombie.inventory.types.HandWeapon",
        ItemClass::Clothing => "zombie.inventory.types.Clothing",
        ItemClass::Drainable => "zombie.inventory.types.DrainableComboItem",
        ItemClass::Radio => "zombie.inventory.types.Radio",
        ItemClass::Moveable => "zombie.inventory.types.Moveable",
        ItemClass::Map => "zombie.inventory.types.MapItem",
        ItemClass::Animal => "zombie.inventory.types.AnimalInventoryItem",
    }
}

/// An item's Lua object.
pub struct LuaInventoryItem {
    pub item: ItemRef,
    class: &'static str,
}

impl LuaObject for LuaInventoryItem {
    fn class_name(&self) -> &str {
        self.class
    }
    fn to_lua_string(&self) -> String {
        // `getFullType() + ":" + super.toString()`.
        let i = self.item.borrow();
        format!(
            "{}:{}@{:x}",
            i.get_full_type(),
            self.class,
            Rc::as_ptr(&self.item) as usize
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        const ITEM: &[&str] = &[INVENTORY_ITEM_CLASS, GAME_ENTITY_CLASS];
        const RADIO: &[&str] = &[
            "zombie.inventory.types.Moveable",
            INVENTORY_ITEM_CLASS,
            GAME_ENTITY_CLASS,
        ];
        const ALARM: &[&str] = &[
            "zombie.inventory.types.Clothing",
            INVENTORY_ITEM_CLASS,
            GAME_ENTITY_CLASS,
        ];
        match self.class {
            "zombie.inventory.types.Radio" => RADIO,
            "zombie.inventory.types.AlarmClockClothing" => ALARM,
            _ => ITEM,
        }
    }
}

/// The Lua object of `item` (the same object while it is alive).
pub fn item_value(item: &ItemRef) -> Value {
    let key = Rc::as_ptr(item) as *const () as usize;
    OBJECTS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(o) = m.get(&key).and_then(Weak::upgrade) {
            return Value::Object(o);
        }
        let class = class_of(&item.borrow());
        let o: Rc<dyn LuaObject> = Rc::new(LuaInventoryItem {
            item: item.clone(),
            class,
        });
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(key, Rc::downgrade(&o));
        Value::Object(o)
    })
}

/// The item behind argument `i`.
pub fn item_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<ItemRef> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<LuaInventoryItem>() {
            Some(l) => Ok(l.item.clone()),
            None => Err(LuaError::runtime(format!(
                "Expected an InventoryItem but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected an InventoryItem but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn this(c: &NativeCall<'_>) -> LuaResult<ItemRef> {
    item_arg(c, 0)
}

fn java_error(e: ItemError) -> LuaError {
    match e {
        ItemError::NullPointer(what) => LuaError::Runtime {
            message: format!("{what} is null"),
            class: "java.lang.NullPointerException",
        },
    }
}

fn opt_str(s: Option<&str>) -> Value {
    s.map(Value::from).unwrap_or_default()
}

fn f(v: f32) -> Value {
    Value::Num(v as f64)
}

/// An id from a string or an object whose `toString` is the id.
fn id_arg(c: &NativeCall<'_>, i: usize) -> Option<String> {
    match c.arg(i) {
        Value::Object(o) => Some(o.to_lua_string()),
        _ => str_arg(c, i).map(|s| s.to_string()),
    }
}

/// A `BloodBodyPartType` (enum object) or its index.
fn part_arg(c: &NativeCall<'_>, i: usize) -> usize {
    if let Value::Object(o) = c.arg(i) {
        if let Some(e) = o.as_any().downcast_ref::<crate::expose::JavaEnum>() {
            return e.ordinal.max(0) as usize;
        }
    }
    int_arg(c, i).max(0) as usize
}

/// Read-only method: `get!(b, "name", |item| value)`.
macro_rules! get {
    ($b:expr, $name:literal, |$i:ident| $body:expr) => {
        $b.method($name, |c| {
            let r = this(c)?;
            let $i = r.borrow();
            let v: Value = $body;
            drop($i);
            Ok(c.push(v))
        });
    };
}

/// Method with the call and the item mutably: `with!(b, "name", |item, c| value)`.
macro_rules! with {
    ($b:expr, $name:literal, |$i:ident, $c:ident| $body:expr) => {
        $b.method($name, |$c| {
            let r = this($c)?;
            let v: LuaResult<Value> = {
                #[allow(unused_mut)]
                let mut $i = r.borrow_mut();
                $body
            };
            let v = v?;
            if matches!(v, Value::Nil) {
                Ok(0)
            } else {
                Ok($c.push(v))
            }
        });
    };
}

/// Method with the environment: `envd!(b, "name", |ref, env, ctx, owner, c| value)`.
macro_rules! envd {
    ($b:expr, $name:literal, |$r:ident, $e:ident, $ctx:ident, $o:ident, $c:ident| $body:expr) => {
        $b.method($name, |$c| {
            let $r = this($c)?;
            let $e = env();
            let $ctx = $e.context();
            let $o = $e.owner(&$r);
            let v: LuaResult<Value> = $body;
            let v = v?;
            if matches!(v, Value::Nil) {
                Ok(0)
            } else {
                Ok($c.push(v))
            }
        });
    };
}

fn register_inventory_item(thread: &mut Thread) {
    expose_class(thread, GAME_ENTITY_CLASS, None);
    let b = expose_class(thread, INVENTORY_ITEM_CLASS, Some(GAME_ENTITY_CLASS));
    get!(b, "getType", |i| Value::from(i.get_type()));
    get!(b, "getFullType", |i| Value::from(i.get_full_type()));
    get!(b, "getModule", |i| Value::from(i.get_module()));
    get!(b, "getDisplayName", |i| Value::from(i.get_name()));
    get!(b, "getName", |i| Value::from(i.get_name()));
    get!(b, "getID", |i| Value::from(i.id));
    get!(b, "getCondition", |i| Value::from(i.condition));
    get!(b, "getConditionMax", |i| Value::from(i.condition_max));
    get!(b, "getCurrentCondition", |i| f(i.get_current_condition()));
    get!(b, "isBroken", |i| Value::from(i.is_broken()));
    get!(b, "getHaveBeenRepaired", |i| Value::from(
        i.have_been_repaired
    ));
    get!(b, "getCurrentUses", |i| Value::from(i.get_current_uses()));
    get!(b, "getMaxUses", |i| Value::from(i.get_max_uses()));
    get!(b, "isAlcoholic", |i| Value::from(i.is_alcoholic()));
    get!(b, "getAge", |i| f(i.get_age()));
    get!(b, "getOffAge", |i| Value::from(i.get_off_age()));
    get!(b, "getOffAgeMax", |i| Value::from(i.get_off_age_max()));
    get!(b, "getStressChange", |i| f(i.get_stress_change()));
    get!(b, "getBoredomChange", |i| f(i.get_boredom_change()));
    get!(b, "getUnhappyChange", |i| f(i.get_unhappy_change()));
    get!(b, "getFatigueChange", |i| f(i.get_fatigue_change()));
    get!(b, "getReduceInfectionPower", |i| f(
        i.get_reduce_infection_power()
    ));
    get!(b, "isCooked", |i| Value::from(i.is_cooked()));
    get!(b, "isBurnt", |i| Value::from(i.is_burnt()));
    get!(b, "getWetness", |i| f(i.get_wetness()));
    get!(b, "getBloodLevel", |i| f(match &i.kind {
        ItemKind::Weapon(w) => w.blood_level,
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => c.blood_level,
        _ => 0.0,
    }));
    get!(b, "isCustomWeight", |i| Value::from(i.custom_weight));
    get!(b, "isCustomName", |i| Value::from(i.custom_name));
    get!(b, "isFavorite", |i| Value::from(i.favorite));
    get!(b, "isActivated", |i| Value::from(i.activated));
    get!(b, "isInfected", |i| Value::from(i.zombie_infected));
    get!(b, "getColorRed", |i| f(i.color_red));
    get!(b, "getColorGreen", |i| f(i.color_green));
    get!(b, "getColorBlue", |i| f(i.color_blue));
    get!(b, "getAttachedSlot", |i| Value::from(i.attached_slot));
    get!(b, "getKeyId", |i| Value::from(match &i.kind {
        ItemKind::Key { key_id, .. } => *key_id,
        _ => i.key_id,
    }));
    get!(b, "getModelIndex", |i| Value::from(i.model_index));
    get!(b, "getCurrentAmmoCount", |i| Value::from(
        i.current_ammo_count
    ));
    get!(b, "getMaxAmmo", |i| {
        let sm = crate::bind_registry::shared_script_manager();
        Value::from(pz_characters::character::items::script_item(&sm, &i).map_or(0, |s| s.borrow().max_ammo))
    });
    b.method("setCurrentAmmoCount", |c| {
        let n = int_arg(c, 1);
        this(c)?.borrow_mut().set_current_ammo_count(n);
        Ok(0)
    });
    get!(b, "getWetCooldown", |i| f(i.wet_cooldown));
    get!(b, "getCount", |i| Value::from(i.template.count));
    get!(b, "getDisplayCategory", |i| opt_str(
        i.template.display_category.as_deref()
    ));
    get!(b, "getReplaceOnUse", |i| opt_str(i.get_replace_on_use()));
    get!(b, "getReplaceOnUseFullType", |i| i
        .get_replace_on_use_full_type()
        .map(Value::from)
        .unwrap_or_default());
    get!(b, "getBodyLocation", |i| opt_str(i.get_body_location()));
    get!(
        b,
        "getBloodClothingType",
        |i| match i.get_blood_clothing_type() {
            Some(n) => Value::Object(Rc::new(JavaList::new(
                n.iter().map(|s| Value::from(s.as_str())).collect()
            ))),
            None => Value::Nil,
        }
    );
    get!(b, "isEquippedNoSprint", |i| Value::from(
        i.is_equipped_no_sprint()
    ));
    get!(b, "isProtectFromRainWhileEquipped", |i| Value::from(
        i.is_protect_from_rain_while_equipped()
    ));
    get!(b, "getClothingItemName", |i| opt_str(
        i.get_clothing_item_name()
    ));
    get!(b, "getWithoutDrainable", |i| opt_str(
        i.template.without_drainable.as_deref()
    ));
    get!(b, "getSwingAnim", |i| opt_str(i.get_swing_anim()));
    get!(b, "isTwoHandWeapon", |i| Value::from(
        i.is_two_hand_weapon()
    ));
    get!(b, "isDisappearOnUse", |i| Value::from(
        i.is_disappear_on_use()
    ));
    get!(b, "isKeepOnDeplete", |i| Value::from(
        i.is_keep_on_deplete()
    ));
    get!(b, "isKeyRing", |i| Value::from(i.is_key_ring()));
    get!(b, "getBreakSound", |i| opt_str(
        i.template.break_sound.as_deref()
    ));
    get!(b, "getDamagedSound", |i| opt_str(
        i.template.damaged_sound.as_deref()
    ));
    get!(b, "getUseDelta", |i| f(i.template.use_delta));
    get!(b, "getCurrentUsesFloat", |i| f(match i.template.class {
        ItemClass::Drainable => i.get_current_uses_float(),
        _ => i.get_current_uses() as f32 / i.get_max_uses() as f32,
    }));
    get!(b, "IsFood", |i| Value::from(matches!(
        i.kind,
        ItemKind::Food(_)
    )));
    get!(b, "IsWeapon", |i| Value::from(matches!(
        i.kind,
        ItemKind::Weapon(_)
    )));
    get!(b, "IsDrainable", |i| Value::from(
        i.template.class == ItemClass::Drainable
    ));
    get!(b, "IsClothing", |i| Value::from(matches!(
        i.kind,
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
    )));
    get!(b, "IsInventoryContainer", |i| Value::from(matches!(
        i.kind,
        ItemKind::Container(_)
    )));
    get!(b, "IsLiterature", |i| Value::from(matches!(
        i.kind,
        ItemKind::Literature(_)
    )));
    get!(b, "IsMap", |i| Value::from(matches!(
        i.kind,
        ItemKind::Map { .. }
    )));
    b.method("hasTag", |c| {
        let r = this(c)?;
        let i = r.borrow();
        let mut has = false;
        for a in 1..c.nargs() {
            if id_arg(c, a).is_some_and(|t| i.has_tag(&t)) {
                has = true;
            }
        }
        drop(i);
        Ok(c.push(has))
    });
    with!(b, "isBodyLocation", |i, c| Ok(Value::from(
        id_arg(c, 1).is_some_and(|l| i.is_body_location(&l))
    )));
    with!(b, "setActivated", |i, c| {
        i.activated = bool_arg(c, 1);
        Ok(Value::Nil)
    });
    with!(b, "setFavorite", |i, c| {
        i.favorite = bool_arg(c, 1);
        Ok(Value::Nil)
    });
    with!(b, "setActualWeight", |i, c| {
        i.actual_weight = (num_arg(c, 1) as f32).max(0.0);
        Ok(Value::Nil)
    });
    with!(b, "setWeight", |i, c| {
        i.weight = (num_arg(c, 1) as f32).max(0.0);
        Ok(Value::Nil)
    });
    with!(b, "setCustomWeight", |i, c| {
        i.custom_weight = bool_arg(c, 1);
        Ok(Value::Nil)
    });
    envd!(b, "setCurrentUses", |r, e, ctx, _o, c| {
        r.borrow_mut()
            .set_current_uses_with(int_arg(c, 1), Some(ctx));
        Ok(Value::Nil)
    });
    envd!(b, "getActualWeight", |r, e, ctx, _o, _c| Ok(f(r
        .borrow()
        .get_actual_weight_with(ctx))));
    envd!(b, "getWeight", |r, e, ctx, _o, _c| Ok(f(r
        .borrow()
        .get_weight_with(ctx))));
    envd!(b, "getContentsWeight", |r, e, ctx, o, _c| Ok(f(r
        .borrow()
        .get_contents_weight_with(ctx, o.as_ref()))));
    envd!(b, "getUnequippedWeight", |r, e, ctx, o, _c| Ok(f(r
        .borrow()
        .get_unequipped_weight(ctx, o.as_ref()))));
    envd!(b, "getEquippedWeight", |r, e, ctx, o, _c| Ok(f(r
        .borrow()
        .get_equipped_weight(ctx, o.as_ref()))));
    envd!(b, "getHotbarEquippedWeight", |r, e, ctx, o, _c| Ok(f(r
        .borrow()
        .get_hotbar_equipped_weight(ctx, o.as_ref()))));
    envd!(b, "isEquipped", |r, e, _ctx, o, _c| Ok(Value::from(
        r.borrow().is_equipped(o.as_ref())
    )));
    envd!(b, "isFakeEquipped", |r, e, _ctx, o, _c| Ok(Value::from(
        r.borrow().is_fake_equipped(o.as_ref())
    )));
    envd!(b, "isWorn", |r, e, _ctx, o, _c| {
        let i = r.borrow();
        let clothing = matches!(
            i.kind,
            ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
        );
        Ok(Value::from(clothing && o.is_worn(&i)))
    });
    envd!(b, "getAlternateModelName", |r, e, ctx, o, _c| Ok(opt_str(
        r.borrow()
            .get_alternate_model_name(ctx.assets(), o.as_ref())
    )));
    envd!(b, "setCondition", |r, e, ctx, o, c| {
        let cond = int_arg(c, 1);
        let out = if c.nargs() > 2 {
            r.borrow_mut()
                .set_condition_sound(cond, bool_arg(c, 2), o.as_ref(), ctx)
        } else {
            r.borrow_mut().set_condition(cond, o.as_ref(), ctx)
        };
        e.outcome(&r, ItemOutcome::Condition(out));
        Ok(Value::Nil)
    });
    envd!(b, "setConditionNoSound", |r, e, ctx, o, c| {
        let out = r
            .borrow_mut()
            .set_condition_no_sound(int_arg(c, 1), o.as_ref(), ctx);
        e.outcome(&r, ItemOutcome::Condition(out));
        Ok(Value::Nil)
    });
    let use_ = |name: &'static str| {
        move |c: &mut NativeCall<'_>| -> LuaResult<usize> {
            let r = this(c)?;
            let e = env();
            let (crafting, in_container, sync) = match name {
                "Use" => (bool_arg(c, 1), bool_arg(c, 2), bool_arg(c, 3)),
                _ => (false, false, false),
            };
            let out = r.borrow_mut().use_with(
                crafting,
                in_container,
                sync,
                e.owner(&r).as_ref(),
                e.context(),
            );
            e.outcome(&r, ItemOutcome::Use(Box::new(out)));
            Ok(0)
        }
    };
    b.method("Use", use_("Use"));
    b.method("UseAndSync", use_("UseAndSync"));
    b.method("UseItem", use_("UseItem"));
}

fn register_clothing(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.Clothing",
        Some(INVENTORY_ITEM_CLASS),
    );
    get!(b, "getScratchDefense", |i| f(i.get_scratch_defense()));
    get!(b, "getBiteDefense", |i| f(i.get_bite_defense()));
    get!(b, "getBulletDefense", |i| f(i.get_bullet_defense()));
    get!(b, "getInsulation", |i| f(i.get_insulation()));
    get!(b, "getWindresistance", |i| f(i.get_windresistance()));
    get!(b, "getWaterResistance", |i| f(i.get_water_resistance()));
    get!(b, "getWeightWet", |i| f(i.get_weight_wet()));
    get!(b, "getNeckProtectionModifier", |i| f(i
        .template
        .neck_protection_modifier));
    get!(b, "getDirtiness", |i| f(i.get_dirtyness()));
    get!(b, "getBloodlevel", |i| f(i.get_bloodlevel()));
    get!(b, "isRemoveOnBroken", |i| Value::from(
        i.template.remove_on_broken
    ));
    get!(b, "getCanHaveHoles", |i| Value::from(
        i.template.can_have_holes
    ));
    get!(b, "isCosmetic", |i| Value::from(i.template.cosmetic));
    get!(b, "hasFilter", |i| Value::from(i.has_filter()));
    get!(b, "hasTank", |i| Value::from(i.has_tank()));
    get!(b, "getFilterType", |i| opt_str(i.get_filter_type()));
    get!(b, "getTankType", |i| opt_str(i.get_tank_type()));
    get!(b, "getHolesNumber", |i| Value::from(
        i.visual.as_ref().map_or(0, |v| v.get_holes_number())
    ));
    get!(b, "getPatchesNumber", |i| Value::from(match &i.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) =>
            c.patches.as_ref().map_or(0, |p| p.len() as i32),
        _ => 0,
    }));
    with!(b, "setWetness", |i, c| {
        i.set_wetness(num_arg(c, 1) as f32);
        Ok(Value::Nil)
    });
    with!(b, "setFilterType", |i, c| {
        i.set_filter_type(&str_arg(c, 1).unwrap_or_else(|| Rc::from("")));
        Ok(Value::Nil)
    });
    with!(b, "setTankType", |i, c| {
        i.set_tank_type(&str_arg(c, 1).unwrap_or_else(|| Rc::from("")));
        Ok(Value::Nil)
    });
    with!(b, "getUsedDelta", |i, _c| Ok(f(i.get_used_delta())));
    with!(b, "setUsedDelta", |i, c| {
        i.set_used_delta(num_arg(c, 1) as f32);
        Ok(Value::Nil)
    });
    envd!(b, "getCorpseSicknessDefense", |r, e, ctx, _o, _c| Ok(f(r
        .borrow()
        .get_corpse_sickness_defense_with(ctx))));
    envd!(b, "getDefForPart", |r, e, ctx, _o, c| {
        let Some(a) = ctx.assets() else {
            return Err(java_error(ItemError::NullPointer("getVisual()")));
        };
        let d = r
            .borrow()
            .get_def_for_part(part_arg(c, 1), bool_arg(c, 2), bool_arg(c, 3), a)
            .map_err(java_error)?;
        Ok(f(d))
    });
    envd!(b, "getCoveredParts", |r, e, ctx, _o, _c| {
        let parts = ctx.assets().and_then(|a| r.borrow().get_covered_parts(a));
        Ok(Value::Object(Rc::new(JavaList::new(
            parts
                .unwrap_or_default()
                .into_iter()
                .map(|p| Value::from(p as i32))
                .collect(),
        ))))
    });
    envd!(b, "updateWetness", |r, e, ctx, o, c| {
        let out = r
            .borrow_mut()
            .update_wetness_with(bool_arg(c, 1), o.as_ref(), ctx);
        e.outcome(&r, ItemOutcome::Condition(out));
        Ok(Value::Nil)
    });
    envd!(b, "flushWetness", |r, e, ctx, o, _c| {
        let out = r.borrow_mut().flush_wetness(o.as_ref(), ctx);
        e.outcome(&r, ItemOutcome::Condition(out));
        Ok(Value::Nil)
    });
    envd!(b, "drainGasMask", |r, e, ctx, _o, c| {
        let rate = if c.nargs() > 1 {
            num_arg(c, 1) as f32
        } else {
            1.0
        };
        r.borrow_mut()
            .drain_gas_mask(rate, ctx)
            .map_err(java_error)?;
        Ok(Value::Nil)
    });
    envd!(b, "drainSCBA", |r, e, ctx, _o, _c| {
        r.borrow_mut().drain_scba(ctx).map_err(java_error)?;
        Ok(Value::Nil)
    });
    let a = expose_class(
        thread,
        "zombie.inventory.types.AlarmClockClothing",
        Some("zombie.inventory.types.Clothing"),
    );
    let _ = a;
}

fn food(i: &InventoryItem, g: fn(&pz_inventory::item::FoodData) -> f32) -> Value {
    match &i.kind {
        ItemKind::Food(x) => f(g(x)),
        _ => f(0.0),
    }
}

/// A weapon's script values (`FirearmStats`), `nil` for other items.
fn firearm(i: &InventoryItem, g: fn(&pz_characters::character::firearm::FirearmStats) -> Value) -> Value {
    let sm = crate::bind_registry::shared_script_manager();
    pz_characters::character::firearm::FirearmStats::of(&sm, i).map_or(Value::Nil, |f| g(&f))
}

/// Runs `f` with the character argument 1 (`checkJam(player, ...)`),
/// the scripts, the character statics and the simulation context.
fn with_shooter<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(
        &crate::bind_character_sim::LuaGameCharacter,
        &pz_scripts::ScriptManager,
        &pz_characters::Characters,
        &dyn pz_characters::SimContext,
    ) -> R,
) -> LuaResult<R> {
    let chr = match c.arg(1) {
        Value::Object(o) => o.as_any().downcast_ref::<crate::bind_character_sim::CharacterObject>().map(|o| o.0.clone()),
        _ => None,
    };
    let chr = chr.ok_or_else(|| LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" })?;
    let sm = crate::bind_registry::shared_script_manager();
    let chars = crate::bind_characters::shared_characters();
    let ctx = crate::bind_character_sim::current_sim_context();
    let p = chr.borrow();
    let chars = chars.borrow();
    Ok(f(&p, &sm, &chars, &*ctx))
}

fn weapon(i: &InventoryItem, g: fn(&pz_inventory::item::WeaponData) -> Value) -> Value {
    match &i.kind {
        ItemKind::Weapon(x) => g(x),
        _ => Value::Nil,
    }
}

fn register_food(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.Food",
        Some(INVENTORY_ITEM_CLASS),
    );
    get!(b, "getHungerChange", |i| f(i.get_hunger_change()));
    get!(b, "getHungChange", |i| f(i.get_hung_change()));
    get!(b, "getBaseHunger", |i| f(i.get_base_hunger()));
    get!(b, "getPoisonPower", |i| Value::from(i.get_poison_power()));
    get!(b, "isTainted", |i| Value::from(i.is_tainted()));
    get!(b, "isbDangerousUncooked", |i| Value::from(
        i.is_b_dangerous_uncooked()
    ));
    get!(b, "getOnEat", |i| opt_str(i.get_on_eat()));
    get!(b, "getThirstChange", |i| food(&i, |x| x.thirst_change));
    get!(b, "getCalories", |i| food(&i, |x| x.calories));
    get!(b, "getProteins", |i| food(&i, |x| x.proteins));
    get!(b, "getLipids", |i| food(&i, |x| x.lipids));
    get!(b, "getCarbohydrates", |i| food(&i, |x| x.carbohydrates));
    get!(b, "getHeat", |i| food(&i, |x| x.heat));
    get!(b, "getPainReduction", |i| food(&i, |x| x.pain_reduction));
    get!(b, "getEndChange", |i| food(&i, |x| x.end_change));
    get!(b, "getFluReduction", |i| Value::from(match &i.kind {
        ItemKind::Food(x) => x.flu_reduction,
        _ => 0,
    }));
    get!(b, "isFrozen", |i| Value::from(
        matches!(&i.kind, ItemKind::Food(x) if x.frozen)
    ));
    get!(b, "isPoison", |i| Value::from(
        matches!(&i.kind, ItemKind::Food(x) if x.poison)
    ));
}

fn register_weapon(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.HandWeapon",
        Some(INVENTORY_ITEM_CLASS),
    );
    get!(b, "getMinDamage", |i| f(i.get_min_damage()));
    get!(b, "getMaxDamage", |i| f(i.get_max_damage()));
    get!(b, "isRanged", |i| Value::from(i.is_ranged()));
    get!(b, "isAimedFirearm", |i| Value::from(i.is_aimed_firearm()));
    get!(b, "haveChamber", |i| Value::from(i.template.have_chamber));
    get!(b, "getMagazineType", |i| opt_str(
        i.template.magazine_type.as_deref()
    ));
    get!(b, "isRoundChambered", |i| weapon(&i, |x| Value::from(
        x.round_chambered
    )));
    get!(b, "isContainsClip", |i| weapon(&i, |x| Value::from(
        x.contains_clip
    )));
    get!(b, "isJammed", |i| weapon(&i, |x| Value::from(x.is_jammed)));
    get!(b, "getClipSize", |i| weapon(&i, |x| Value::from(
        x.clip_size
    )));
    get!(b, "getMaxRange", |i| weapon(&i, |x| f(x.max_range)));
    get!(b, "getFireMode", |i| weapon(&i, |x| opt_str(
        x.fire_mode.as_deref()
    )));
    // The firearm state Lua's reload, rack and shoot actions use.
    get!(b, "isSpentRoundChambered", |i| Value::from(i.is_spent_round_chambered()));
    get!(b, "getSpentRoundCount", |i| Value::from(i.get_spent_round_count()));
    get!(b, "getAmmoPerShoot", |_i| Value::from(1));
    get!(b, "getJamGunChance", |i| firearm(&i, |w| f(w.jam_gun_chance)));
    get!(b, "isRackAfterShoot", |i| firearm(&i, |w| Value::from(w.rack_after_shoot)));
    get!(b, "isManuallyRemoveSpentRounds", |i| firearm(&i, |w| Value::from(w.manually_remove_spent_rounds)));
    get!(b, "getAimingTime", |i| firearm(&i, |w| Value::from(w.aiming_time)));
    get!(b, "getRecoilDelay", |i| firearm(&i, |w| Value::from(w.recoil_delay)));
    b.method("setRoundChambered", |c| {
        let r = this(c)?;
        let v = bool_arg(c, 1);
        let have = r.borrow().template.have_chamber;
        r.borrow_mut().set_round_chambered(v, have);
        Ok(0)
    });
    b.method("setSpentRoundChambered", |c| {
        let v = bool_arg(c, 1);
        this(c)?.borrow_mut().set_spent_round_chambered(v);
        Ok(0)
    });
    b.method("setSpentRoundCount", |c| {
        let r = this(c)?;
        let n = int_arg(c, 1);
        let max = firearm(&r.borrow(), |w| Value::from(w.max_ammo));
        let max = match max {
            Value::Num(m) => m as i32,
            _ => 0,
        };
        r.borrow_mut().set_spent_round_count(n, max);
        Ok(0)
    });
    b.method("setJammed", |c| {
        let v = bool_arg(c, 1);
        this(c)?.borrow_mut().set_jammed(v);
        Ok(0)
    });
    b.method("setContainsClip", |c| {
        let v = bool_arg(c, 1);
        this(c)?.borrow_mut().set_contains_clip(v);
        Ok(0)
    });
    b.method("checkJam", |c| {
        let r = this(c)?;
        let racking = bool_arg(c, 2);
        let v = with_shooter(c, |p, sm, chars, ctx| p.check_jam_item(sm, chars, ctx, &mut r.borrow_mut(), racking))?;
        Ok(c.push(v))
    });
    b.method("checkUnJam", |c| {
        let r = this(c)?;
        let v = with_shooter(c, |p, sm, chars, ctx| p.check_unjam_item(sm, chars, ctx, &mut r.borrow_mut()))?;
        Ok(c.push(v))
    });
    b.method("isOfWeaponCategory", |c| {
        let r = this(c)?;
        let id = id_arg(c, 1);
        let has = id.is_some_and(|id| {
            r.borrow()
                .get_weapon_categories()
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&id))
        });
        Ok(c.push(has))
    });
}

fn register_drainable(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.DrainableComboItem",
        Some(INVENTORY_ITEM_CLASS),
    );
    get!(b, "getReplaceOnDeplete", |i| opt_str(
        i.get_replace_on_deplete()
    ));
    get!(b, "getReplaceOnDepleteFullType", |i| i
        .get_replace_on_deplete_full_type()
        .map(Value::from)
        .unwrap_or_default());
    get!(b, "getWeightEmpty", |i| f(i.template.weight_empty));
    get!(b, "getOnEat", |i| opt_str(i.get_on_eat()));
    get!(b, "isEmptyUses", |i| Value::from(
        i.get_current_uses_float() <= 0.0
    ));
    get!(b, "isFullUses", |i| Value::from(
        i.get_current_uses_float() >= 1.0
    ));
    envd!(b, "updateWeight", |r, e, ctx, _o, _c| {
        r.borrow_mut().update_weight(Some(ctx));
        Ok(Value::Nil)
    });
}

fn register_container(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.InventoryContainer",
        Some(INVENTORY_ITEM_CLASS),
    );
    get!(b, "getWeightReduction", |i| Value::from(match &i.kind {
        ItemKind::Container(c) => c.weight_reduction,
        _ => 0,
    }));
    get!(b, "getCapacity", |i| Value::from(match &i.kind {
        ItemKind::Container(c) => c.container.capacity,
        _ => 0,
    }));
    get!(b, "isEmpty", |i| Value::from(match &i.kind {
        ItemKind::Container(c) => c.container.items.is_empty(),
        _ => true,
    }));
    envd!(b, "getInventoryWeight", |r, e, ctx, o, _c| {
        // InventoryContainer.getInventoryWeight.
        let i = r.borrow();
        let equipped = i.is_equipped(o.as_ref()) || i.is_fake_equipped(o.as_ref());
        let inner = NoOwner;
        let total = match &i.kind {
            ItemKind::Container(c) => {
                c.container
                    .items
                    .iter()
                    .filter_map(|s| s.as_item())
                    .fold(0.0f32, |a, it| {
                        a + if equipped {
                            it.get_equipped_weight(ctx, &inner)
                        } else {
                            it.get_unequipped_weight(ctx, &inner)
                        }
                    })
            }
            _ => 0.0,
        };
        Ok(f(total))
    });
}

/// Registers the item classes and `WeaponType`.
pub fn register(thread: &mut Thread) {
    register_inventory_item(thread);
    register_clothing(thread);
    register_food(thread);
    register_weapon(thread);
    register_drainable(thread);
    register_container(thread);
    for c in [
        "zombie.inventory.types.ComboItem",
        "zombie.inventory.types.WeaponPart",
        "zombie.inventory.types.AnimalInventoryItem",
        "zombie.inventory.types.Literature",
        "zombie.inventory.types.Moveable",
        "zombie.inventory.types.MapItem",
        "zombie.inventory.types.AlarmClock",
        "zombie.inventory.types.Key",
        "zombie.inventory.types.KeyRing",
    ] {
        expose_class(thread, c, Some(INVENTORY_ITEM_CLASS));
    }
    expose_class(
        thread,
        "zombie.inventory.types.Radio",
        Some("zombie.inventory.types.Moveable"),
    );
    let wt = expose_enum(thread, WEAPON_TYPE_CLASS, &WEAPON_TYPES);
    wt.static_fn("getWeaponType", |c| {
        // getWeaponType(HandWeapon).
        let v = match c.arg(0) {
            Value::Object(o) => match o.as_any().downcast_ref::<LuaInventoryItem>() {
                Some(l) => l.item.borrow().get_weapon_type(),
                None => None,
            },
            _ => None,
        };
        Ok(c.push(
            v.map(|n| enum_value(WEAPON_TYPE_CLASS, n))
                .unwrap_or_default(),
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_inventory::ItemTemplate;

    fn run(t: &mut Thread, src: &str) -> Vec<Value> {
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone())
            .expect("compile");
        t.pcall(&f, &[])
            .map_err(|(m, _)| pz_kahlua::exec::java_value_of(&m))
            .expect("run")
    }

    #[test]
    fn items_in_lua() {
        let mut thread = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_globals::register(&mut thread);
        register(&mut thread);
        let mut t = ItemTemplate::new(ItemClass::Clothing, "Base", "Tshirt");
        t.scratch_defense = 20.0;
        t.display_name = "T-shirt".into();
        t.actual_weight = 0.5;
        t.tags = vec!["base:spiked".into()];
        let item: ItemRef = Rc::new(RefCell::new(InventoryItem::create(Rc::new(t), 1)));
        thread.globals().set_str("item", item_value(&item));
        thread.globals().set_str("again", item_value(&item));
        let r = run(
            &mut thread,
            "item:setCondition(0)\n\
             return item:getFullType(), item:getScratchDefense(), item:getActualWeight(),\n\
             item:hasTag('base:spiked'), item == again, instanceof(item, 'InventoryItem'),\n\
             item:isBroken(), item:getCurrentCondition()",
        );
        assert_eq!(pz_kahlua::exec::java_value_of(&r[0]), "Base.Tshirt");
        assert!(matches!(r[1], Value::Num(n) if n == 0.0));
        assert!(matches!(r[2], Value::Num(n) if n == 0.5));
        assert!(matches!(r[3], Value::Bool(true)));
        assert!(matches!(r[4], Value::Bool(true)));
        assert!(matches!(r[5], Value::Bool(true)));
        assert!(matches!(r[6], Value::Bool(true)));
        assert_eq!(item.borrow().condition, 0);
    }
}
