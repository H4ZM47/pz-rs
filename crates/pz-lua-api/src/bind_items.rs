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
//! ([`install_item_env`]); without one they see the item's own container
//! and its parent ([`crate::bind_item_container::container_owner`]), the
//! shared scripts, no `ClothingItem` assets and zero globals
//! ([`GameItemEnv`]). `instanceItem` ([`create_item`]) creates
//! items through it; see [`crate::bind_item_visual`].
//!
//! Not exposed yet: methods returning or taking objects that have no Lua
//! binding here (`getScriptItem`, `getModData`, `getWorldItem`,
//! `getTags`; `getContainer` and `InventoryContainer.getInventory` are in
//! [`crate::bind_item_container`]), the registry-object
//! returns (`getBloodClothingType` a list of names, `hasTag` takes ids or
//! objects whose `tostring` is the id). `getVisual`, `getClothingItem`
//! and `canBeEquipped` live in [`crate::bind_item_visual`].

use crate::bind_java_util::JavaList;
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg};
use pz_inventory::api::Offline;
use pz_inventory::gameplay::{ConditionOutcome, ItemError, UseOutcome};
use pz_inventory::item::ItemKind;
use pz_inventory::visual::VisualRng;
use pz_inventory::{InventoryItem, ItemAssets, ItemClass, ItemContext, ItemEnv, ItemOwner};
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
    /// `Rand.Next(bound)` (`RandStandard`), for item creation.
    fn rand_next(&self, bound: i32) -> i32 {
        pz_core::random::next(bound)
    }
    /// `Rand.Next(min, max)` for floats.
    fn rand_float(&self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
    /// `OutfitRNG` and `ClothingDecals` for item visuals.
    fn visual_rng(&self) -> &dyn VisualRng {
        &OutfitVisualRng
    }
    /// The entity registries (attributes, fluids) items' ECS components
    /// resolve against: new items get their factory components
    /// (`GameEntityFactory.CreateInventoryItemEntity`) only with one.
    fn entity(&self) -> Option<&dyn pz_entity::EntityContext> {
        None
    }
    /// A moveable world sprite's properties (`IsoSpriteManager.namedMap`)
    /// for `Moveable.ReadFromWorldSprite` at creation; `None` without the
    /// sprite tables.
    fn world_sprite(&self, _name: &str) -> Option<pz_inventory::env::WorldSpriteInfo> {
        None
    }
}

/// The item environment outside a game (the main menu and character
/// creation): no holder, game state at its defaults, scripts from the
/// shared `ScriptManager` and the `ClothingItem` assets it was given.
pub struct GameItemEnv {
    pub assets: Option<ItemAssets>,
    /// A generator for `Rand` instead of `RandStandard` (tests, oracles).
    pub rand: Option<pz_core::random::Rand>,
    /// The entity registries ([`ItemLuaEnv::entity`]).
    pub entity: Option<Rc<pz_entity::BasicContext>>,
    /// `ZomboidGlobals.equippedOrWornEncumbranceMultiplier` once
    /// `ZomboidGlobals.Load` read it (the offline default otherwise).
    pub encumbrance_multiplier: Option<f64>,
}

impl GameItemEnv {
    pub fn new(assets: Option<ItemAssets>) -> Self {
        GameItemEnv {
            assets,
            rand: None,
            entity: None,
            encumbrance_multiplier: None,
        }
    }
}

/// The entity registries of the game (`AttributeType`s from the jar,
/// `Fluid`s from the jar and the loaded scripts), as the item factory
/// needs them for `FluidContainer` and other item components.
pub fn item_entity_context(
    jar: &std::path::Path,
    sm: &pz_scripts::ScriptManager,
) -> Result<pz_entity::BasicContext, String> {
    let jar = &pz_classfile::Jar::open(jar).map_err(|e| format!("{}: {e:?}", jar.display()))?;
    Ok(pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(jar)
            .map_err(|e| format!("attribute types: {e:?}"))?,
        fluids: Some(
            pz_entity::FluidRegistry::from_scripts(jar, sm)
                .map_err(|e| format!("fluids: {e:?}"))?,
        ),
        ..Default::default()
    })
}

impl ItemContext for GameItemEnv {
    fn world_age_hours(&self) -> f64 {
        Offline.world_age_hours()
    }
    fn multiplier(&self) -> f32 {
        Offline.multiplier()
    }
    fn rain_intensity(&self) -> f32 {
        Offline.rain_intensity()
    }
    fn is_raining(&self) -> bool {
        Offline.is_raining()
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        self.encumbrance_multiplier
            .unwrap_or_else(|| Offline.equipped_or_worn_encumbrance_multiplier())
    }
    fn basic_mask_defense(&self) -> f32 {
        Offline.basic_mask_defense()
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        Offline.improvised_gas_mask_defense()
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        Offline.standard_gas_mask_defense()
    }
    fn script(&self, name: &str) -> Option<Rc<pz_inventory::ItemTemplate>> {
        let sm = crate::bind_registry::shared_script_manager();
        let s = sm
            .find_item(name, true)?
            .downcast::<pz_scripts::objects::items::Item>()?;
        pz_inventory::ItemTemplate::from_script_ref(&sm, &s)
    }
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        create_item_with(self, self.entity.as_deref().map(|e| e as _), full_type, 1.0)
    }
    fn fluid_amount(&self, item: &InventoryItem) -> Option<f32> {
        match &self.entity {
            Some(e) => pz_inventory::gameplay::fluid_container_amount(item, e.as_ref(), 0),
            None => pz_inventory::gameplay::fluid_container_amount(
                item,
                &pz_entity::BasicContext::default(),
                0,
            ),
        }
    }
    fn assets(&self) -> Option<&ItemAssets> {
        self.assets.as_ref()
    }
}

impl ItemLuaEnv for GameItemEnv {
    fn context(&self) -> &dyn ItemContext {
        self
    }
    /// The item's holder through its container
    /// ([`crate::bind_item_container::container_owner`]).
    fn owner(&self, item: &ItemRef) -> Rc<dyn ItemOwner> {
        crate::bind_item_container::container_owner(item)
    }
    fn rand_next(&self, bound: i32) -> i32 {
        match &self.rand {
            Some(r) => r.next(bound),
            None => pz_core::random::next(bound),
        }
    }
    fn rand_float(&self, min: f32, max: f32) -> f32 {
        match &self.rand {
            Some(r) => r.next_float(min, max),
            None => pz_core::random::next_float(min, max),
        }
    }
    fn entity(&self) -> Option<&dyn pz_entity::EntityContext> {
        self.entity.as_deref().map(|e| e as _)
    }
}

/// `OutfitRNG` (the shared per-thread generator) and
/// `ClothingDecals.instance` from the shared population.
pub struct OutfitVisualRng;

impl VisualRng for OutfitVisualRng {
    fn next(&self, bound: i32) -> i32 {
        pz_core::random::outfit_rng::next(bound)
    }
    fn random_color(&self) -> [f32; 3] {
        // `randomImmutableColor()`: `SandboxOptions.noBlackClothes`
        // (default true; the menu has no game sandbox).
        pz_core::random::outfit_rng::random_immutable_color(true)
    }
    fn random_decal(&self, group: &str) -> Option<String> {
        let pop = crate::bind_appearance::shared_population();
        let pop = pop.borrow();
        pop.clothing_decals.as_ref()?.get_random_decal(group)
    }
    fn set_seed(&self, seed: i64) {
        pz_core::random::outfit_rng::set_seed(seed);
    }
    fn get_seed(&self) -> i64 {
        pz_core::random::outfit_rng::get_seed()
    }
}

/// What `InventoryItemFactory.CreateItem` reads, from an item environment.
struct CreateEnv<'a> {
    lua: Option<&'a dyn ItemLuaEnv>,
    ctx: &'a dyn ItemContext,
    sm: &'a pz_scripts::ScriptManager,
    /// `EntityCodec` over the environment's entity registries.
    codec: Option<pz_inventory::component::EntityCodec<'a>>,
}

impl ItemEnv for CreateEnv<'_> {
    fn components(&self) -> &dyn pz_inventory::component::ComponentCodec {
        match &self.codec {
            Some(c) => c,
            None => &pz_inventory::component::OpaqueComponents,
        }
    }
    fn world_age_hours(&self) -> f64 {
        self.ctx.world_age_hours()
    }
    fn rand_next(&self, bound: i32) -> i32 {
        match self.lua {
            Some(l) => l.rand_next(bound),
            None => pz_core::random::next(bound),
        }
    }
    fn rand_float(&self, min: f32, max: f32) -> f32 {
        match self.lua {
            Some(l) => l.rand_float(min, max),
            None => pz_core::random::next_float(min, max),
        }
    }
    fn assets(&self) -> Option<&ItemAssets> {
        self.ctx.assets()
    }
    fn visual_rng(&self) -> &dyn VisualRng {
        match self.lua {
            Some(l) => l.visual_rng(),
            None => &OutfitVisualRng,
        }
    }
    fn world_sprite(&self, name: &str) -> Option<pz_inventory::env::WorldSpriteInfo> {
        self.lua?.world_sprite(name)
    }
    fn script_actual_weight(&self, name: &str) -> Option<f32> {
        let s = self
            .sm
            .find_item(name, true)?
            .downcast::<pz_scripts::objects::items::Item>()?;
        let w = s.borrow().actual_weight;
        Some(w)
    }
}

/// The script item's `getDisplayName()` (translated at load, the full
/// name when it has none).
pub fn script_display_name(full_type: &str) -> Option<String> {
    let sm = crate::bind_registry::shared_script_manager();
    let s = sm
        .find_item(full_type, true)?
        .downcast::<pz_scripts::objects::items::Item>()?;
    let s = s.borrow();
    Some(
        translated_name(&s.module_dot_type)
            .or_else(|| s.display_name.clone().filter(|n| !n.is_empty()))
            .unwrap_or_else(|| s.module_dot_type.clone()),
    )
}

/// The script item's `vehicleType` (`getMechanicType()`), 0 without one.
pub fn script_vehicle_type(full_type: &str) -> i32 {
    let sm = crate::bind_registry::shared_script_manager();
    sm.find_item(full_type, true)
        .and_then(|s| s.downcast::<pz_scripts::objects::items::Item>())
        .map_or(0, |s| s.borrow().vehicle_type)
}

/// `Translator.getItemNameFromFullType` when the translator is loaded.
fn translated_name(full_type: &str) -> Option<String> {
    crate::bind_text::with_translator_mut(|t| t.get_item_name_from_full_type(full_type, |_| None))
        .filter(|n| n != full_type)
}

fn create_item_with(
    ctx: &dyn ItemContext,
    entity: Option<&dyn pz_entity::EntityContext>,
    item_type: &str,
    use_delta: f32,
) -> Option<InventoryItem> {
    let sm = crate::bind_registry::shared_script_manager();
    let env = CreateEnv {
        lua: None,
        ctx,
        sm: &sm,
        codec: entity.map(|ctx| pz_inventory::component::EntityCodec { ctx }),
    };
    pz_inventory::factory::create_item_by_type(
        &env,
        &sm,
        item_type,
        use_delta,
        true,
        &translated_name,
    )
}

/// `InventoryItemFactory.CreateItem(itemType, useDelta)` through the
/// installed environment (its `Rand`, `OutfitRNG` and assets) and the
/// shared `ScriptManager`: `None` for an unknown type.
pub fn create_item(item_type: &str, use_delta: f32) -> Option<ItemRef> {
    let e = env();
    let sm = crate::bind_registry::shared_script_manager();
    let ce = CreateEnv {
        lua: Some(e.as_ref()),
        ctx: e.context(),
        sm: &sm,
        codec: e
            .entity()
            .map(|ctx| pz_inventory::component::EntityCodec { ctx }),
    };
    let item = pz_inventory::factory::create_item_by_type(
        &ce,
        &sm,
        item_type,
        use_delta,
        true,
        &translated_name,
    )?;
    Some(pz_inventory::shared::new_item(item))
}

/// `InventoryItem.setKeyId`, including `Key`'s own field and -1 RNG.
pub(crate) fn set_key_id(item: &ItemRef, key_id: i32) {
    let e = env();
    let sm = crate::bind_registry::shared_script_manager();
    let ce = CreateEnv {
        lua: Some(e.as_ref()),
        ctx: e.context(),
        sm: &sm,
        codec: None,
    };
    item.borrow_mut().set_key_id(key_id, &ce);
}

/// Installs a [`GameItemEnv`] with the `ClothingItem` assets
/// (`ItemAssets::load(game_dir)`), so items made from Lua have visuals.
pub fn install_item_assets(assets: ItemAssets) {
    install_item_env(Rc::new(GameItemEnv::new(Some(assets))));
}

/// Installs a [`GameItemEnv`] with the `ClothingItem` assets and the entity
/// registries ([`item_entity_context`]), so items made from Lua have
/// visuals and their factory components (`FluidContainer`, ...).
pub fn install_item_runtime(assets: Option<ItemAssets>, entity: Option<pz_entity::BasicContext>) {
    let mut env = GameItemEnv::new(assets);
    env.entity = entity.map(Rc::new);
    install_item_env(Rc::new(env));
}

thread_local! {
    static ENV: RefCell<Rc<dyn ItemLuaEnv>> = RefCell::new(Rc::new(GameItemEnv::new(None)));
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

/// The item behind a Lua `InventoryItem` object.
pub fn item_arg_value(v: &Value) -> Option<ItemRef> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaInventoryItem>()
            .map(|l| l.item.clone()),
        _ => None,
    }
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

/// `InventoryItem.getTexture()`.
pub fn item_texture(item: &ItemRef) -> Option<pz_render::Texture> {
    use pz_inventory::naming::{ItemTexture, QUESTION_TEXTURE};
    let custom = item.borrow().custom_inventory_icon().map(str::to_string);
    if let Some(t) = custom.and_then(|n| crate::bind_render::get_shared_texture(&n)) {
        return Some(t);
    }
    let base_textures = {
        let i = item.borrow();
        let bag_or_clothing = matches!(
            i.kind,
            ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..) | ItemKind::Container(_)
        );
        if bag_or_clothing && i.visual.is_some() {
            drop(i);
            let e = env();
            let owner = e.owner(item);
            let i = item.borrow();
            i.get_clothing_item(e.context().assets(), owner.as_ref()).map(|c| c.base_textures.len())
        } else {
            None
        }
    };
    let t = item.borrow().texture(base_textures)?;
    match t {
        ItemTexture::Shared(n) => crate::bind_render::get_shared_texture(&n),
        ItemTexture::Try { name, fallback } => crate::bind_render::try_get_texture(&name)
            .or_else(|| fallback.then(|| crate::bind_render::get_shared_texture(QUESTION_TEXTURE)).flatten()),
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
    get!(b, "getCategory", |i| Value::from(i.get_category()));
    b.method("getItemType", |c| {
        let item = this(c)?;
        let key = crate::bind_registry::key_value(
            crate::bind_registry::ITEM_TYPE_CLASS, item.borrow().item_type.as_ref()
        );
        Ok(c.push(key))
    });
    for (name, set) in [("isItemType", false), ("setItemType", true)] {
        b.method(name, move |c| {
            let item = this(c)?;
            let key = match c.arg(1) {
                Value::Nil => None,
                Value::Object(o) => match o.as_any().downcast_ref::<crate::bind_registry::KeyObject>() {
                    Some(key) if key.class == crate::bind_registry::ITEM_TYPE_CLASS => Some(key.key.clone()),
                    _ => return Err(crate::bind_java_util::class_cast(&Value::Object(o), crate::bind_registry::ITEM_TYPE_CLASS)),
                },
                other => return Err(crate::bind_java_util::class_cast(&other, crate::bind_registry::ITEM_TYPE_CLASS)),
            };
            if set {
                item.borrow_mut().item_type = key;
                Ok(0)
            } else {
                let matches = match (&item.borrow().item_type, key) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Rc::ptr_eq(a, &b),
                    _ => false,
                };
                Ok(c.push(matches))
            }
        });
    }
    get!(b, "getModule", |i| Value::from(i.get_module()));
    // `getName` and `getDisplayName` with the class overrides and state
    // prefixes ([`crate::bind_item_name`]).
    get!(b, "getDisplayName", |i| Value::from(
        crate::bind_item_name::display_name(&i, env().as_ref())
    ));
    get!(b, "getName", |i| Value::from(
        crate::bind_item_name::item_name(&i, env().as_ref())
    ));
    get!(b, "getID", |i| Value::from(i.id));
    // `getTex()` / `getTexture()` / `getIcon()`: the `customInventoryIcon` mod-data
    // texture when it exists, else `texture`.
    for name in ["getTex", "getTexture", "getIcon"] {
        b.method(name, |c| {
            let item = this(c)?;
            Ok(c.push(crate::bind_render::texture_value(item_texture(&item))))
        });
    }
    // `nameAfterDescriptor(SurvivorDesc)` / `monogramAfterDescriptor`.
    for (name, mono) in [("nameAfterDescriptor", false), ("monogramAfterDescriptor", true)] {
        b.method(name, move |c| {
            let item = this(c)?;
            if c.arg(1).is_nil() {
                return Ok(0);
            }
            let d = crate::bind_characters::desc_arg(c, 1)?;
            let (fore, sur) = {
                let d = d.borrow();
                (d.forename.clone(), d.surname.clone())
            };
            let full = item.borrow().get_full_type();
            let display = script_display_name(&full).ok_or_else(|| LuaError::Runtime {
                message: String::new(),
                class: "java.lang.NullPointerException",
            })?;
            let display = crate::bind_text::translate(&display);
            let mut i = item.borrow_mut();
            if mono {
                i.monogram_after_descriptor(&display, fore.as_deref(), sur.as_deref())
                    .map_err(|class| LuaError::Runtime { message: String::new(), class })?;
            } else {
                i.name_after_descriptor(&display, fore.as_deref(), sur.as_deref());
            }
            Ok(0)
        });
    }
    b.method("setName", |c| {
        let item = this(c)?;
        let name = str_arg(c, 1).ok_or_else(|| LuaError::Runtime {
            message: String::new(),
            class: "java.lang.NullPointerException",
        })?;
        item.borrow_mut().set_name(&name);
        Ok(0)
    });

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
    get!(b, "getBloodLevel", |i| f(
        crate::bind_item_name::blood_level(&i, env().context().assets())
    ));
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
    b.method("setKeyId", |c| {
        let item = this(c)?;
        set_key_id(&item, int_arg(c, 1));
        Ok(0)
    });
    get!(b, "getModelIndex", |i| Value::from(i.model_index));
    get!(b, "getCurrentAmmoCount", |i| Value::from(
        i.current_ammo_count
    ));
    get!(b, "getMaxAmmo", |i| {
        let sm = crate::bind_registry::shared_script_manager();
        Value::from(
            pz_characters::character::items::script_item(&sm, &i)
                .map_or(0, |s| s.borrow().max_ammo),
        )
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
    get!(b, "getBodyLocation", |i| {
        crate::bind_item_visual::body_location_value(i.get_body_location())
    });
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
    // `getUseDelta` and `getCurrentUsesFloat` dispatch on the item's
    // class like the Java overrides (`Clothing`, `DrainableComboItem`,
    // `Food`, `Radio`, `WeaponPart`).
    get!(b, "getUseDelta", |i| f(i.get_use_delta()));
    get!(b, "getCurrentUsesFloat", |i| f(i.get_current_uses_float()));
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
    // `HandWeapon.setActivated` also switches its active light part.
    with!(b, "setActivated", |i, c| {
        i.set_activated(bool_arg(c, 1));
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
    envd!(b, "isInPlayerInventory", |r, e, _ctx, o, _c| Ok(Value::from(
        o.is_in_player_inventory(&r.borrow())
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
fn firearm(
    i: &InventoryItem,
    g: fn(&pz_characters::character::firearm::FirearmStats) -> Value,
) -> Value {
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
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<crate::bind_character_sim::CharacterObject>()
            .map(|o| o.0.clone()),
        _ => None,
    };
    let chr = chr.ok_or_else(|| LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    })?;
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
    get!(b, "isSpentRoundChambered", |i| Value::from(
        i.is_spent_round_chambered()
    ));
    get!(b, "getSpentRoundCount", |i| Value::from(
        i.get_spent_round_count()
    ));
    get!(b, "getAmmoPerShoot", |_i| Value::from(1));
    get!(b, "getJamGunChance", |i| firearm(&i, |w| f(
        w.jam_gun_chance
    )));
    get!(b, "isRackAfterShoot", |i| firearm(&i, |w| Value::from(
        w.rack_after_shoot
    )));
    get!(b, "isManuallyRemoveSpentRounds", |i| firearm(&i, |w| {
        Value::from(w.manually_remove_spent_rounds)
    }));
    get!(b, "getAimingTime", |i| firearm(&i, |w| Value::from(
        w.aiming_time
    )));
    get!(b, "getRecoilDelay", |i| firearm(&i, |w| Value::from(
        w.recoil_delay
    )));
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
        let v = with_shooter(c, |p, sm, chars, ctx| {
            p.check_jam_item(sm, chars, ctx, &mut r.borrow_mut(), racking)
        })?;
        Ok(c.push(v))
    });
    b.method("checkUnJam", |c| {
        let r = this(c)?;
        let v = with_shooter(c, |p, sm, chars, ctx| {
            p.check_unjam_item(sm, chars, ctx, &mut r.borrow_mut())
        })?;
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
    get!(b, "isFullUses", |i| Value::from(i.uses >= i.get_max_uses()));
    envd!(b, "updateWeight", |r, e, ctx, _o, _c| {
        r.borrow_mut().update_weight(Some(ctx));
        Ok(Value::Nil)
    });
    register_set_used_delta(&b);
}

/// `DrainableComboItem.setUsedDelta` and `WeaponPart.setUsedDelta`:
/// `setCurrentUsesFloat(delta)` (`Clothing.setUsedDelta` is the gas mask
/// filter in the mod data instead).
fn register_set_used_delta(b: &crate::expose::ClassBuilder) {
    envd!(b, "setUsedDelta", |r, e, ctx, _o, c| {
        r.borrow_mut()
            .set_current_uses_float(num_arg(c, 1) as f32, Some(ctx));
        Ok(Value::Nil)
    });
}

fn register_weapon_part(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.WeaponPart",
        Some(INVENTORY_ITEM_CLASS),
    );
    register_set_used_delta(&b);
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
    envd!(b, "getInventoryWeight", |r, e, ctx, o, _c| {
        // InventoryContainer.getInventoryWeight: the bag's items, equipped
        // weights when the bag is (fake-)equipped.
        let (equipped, inner) = {
            let i = r.borrow();
            (
                i.is_equipped(o.as_ref()) || i.is_fake_equipped(o.as_ref()),
                pz_inventory::shared::inventory_of(&i),
            )
        };
        let mut total = 0.0f32;
        if let Some(inner) = inner {
            let list = inner.borrow().items.clone();
            for it in &list {
                let io = e.owner(it);
                let i = it.borrow();
                total += if equipped {
                    i.get_equipped_weight(ctx, io.as_ref())
                } else {
                    i.get_unequipped_weight(ctx, io.as_ref())
                };
            }
        }
        Ok(f(total))
    });
}

/// Registers the item classes and `WeaponType`.
/// The `InventoryItem` methods the vehicle part Lua (`Vehicles.lua`, the
/// mechanics UI) calls on part items: capacities, the part stats from the
/// item script, `setConditionMax` and `setCurrentUsesFloat`.
fn register_vehicle_part_methods(thread: &mut Thread) {
    let b = expose_class(thread, INVENTORY_ITEM_CLASS, Some(GAME_ENTITY_CLASS));
    get!(b, "getMaxCapacity", |i| Value::from(i.max_capacity));
    get!(b, "getItemCapacity", |i| f(i.item_capacity));
    get!(b, "getConditionLowerNormal", |i| f(i
        .vehicle_part
        .condition_lower_normal));
    get!(b, "getConditionLowerOffroad", |i| f(i
        .vehicle_part
        .condition_lower_offroad));
    get!(b, "getBrakeForce", |i| f(i.vehicle_part.brake_force));
    get!(b, "getWheelFriction", |i| f(i.vehicle_part.wheel_friction));
    get!(b, "getChanceToSpawnDamaged", |i| Value::from(
        i.vehicle_part.chance_to_spawn_damaged
    ));
    with!(b, "setConditionMax", |i, c| {
        i.condition_max = int_arg(c, 1);
        Ok(Value::Nil)
    });
    with!(b, "setMaxCapacity", |i, c| {
        i.max_capacity = int_arg(c, 1);
        Ok(Value::Nil)
    });
    with!(b, "setItemCapacity", |i, c| {
        i.item_capacity = num_arg(c, 1) as f32;
        Ok(Value::Nil)
    });
    // `InventoryItem.setCurrentUsesFloat` (over `InventoryItem.useDelta`),
    // `WeaponPart`'s (its own `useDelta`) and `DrainableComboItem`'s
    // (which also runs the full `updateWeight`).
    envd!(b, "setCurrentUsesFloat", |r, e, ctx, _o, c| {
        r.borrow_mut()
            .set_current_uses_float(num_arg(c, 1) as f32, Some(ctx));
        Ok(Value::Nil)
    });
}

pub fn register(thread: &mut Thread) {
    register_inventory_item(thread);
    // `instanceItem(String)` / `instanceItem(String, float useDelta)`:
    // `InventoryItemFactory.CreateItem` (useDelta 1 by default).
    // TODO(port): the `Item` / `ItemKey` overloads.
    crate::expose::global(thread, "instanceItem", |c| {
        let Some(t) = str_arg(c, 0) else {
            return Err(LuaError::runtime(format!(
                "TODO(port): instanceItem({})",
                pz_kahlua::stdlib::java_class_name(&c.arg(0))
            )));
        };
        let delta = if c.nargs() > 1 {
            num_arg(c, 1) as f32
        } else {
            1.0
        };
        let v = create_item(&t, delta).map_or(Value::Nil, |i| item_value(&i));
        Ok(c.push(v))
    });
    crate::item_tooltip::register(thread);
    register_vehicle_part_methods(thread);
    register_clothing(thread);
    register_food(thread);
    register_weapon(thread);
    register_drainable(thread);
    register_container(thread);
    register_weapon_part(thread);
    for c in [
        "zombie.inventory.types.ComboItem",
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

    /// A Lua thread with the item bindings (and `Radio.getDeviceData`).
    fn item_thread() -> Thread {
        let mut thread = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        crate::bind_globals::register(&mut thread);
        register(&mut thread);
        crate::bind_vehicles::register(&mut thread);
        thread
    }

    fn make(class: ItemClass, name: &str, edit: impl FnOnce(&mut ItemTemplate)) -> ItemRef {
        let mut t = ItemTemplate::new(class, "Base", name);
        t.display_name = name.into();
        edit(&mut t);
        Rc::new(RefCell::new(InventoryItem::create(Rc::new(t), 1)))
    }

    #[test]
    fn inventory_classification_uses_live_instance_type_and_category_override() {
        use crate::bind_registry::{key_value, ITEM_TAG_CLASS, ITEM_TYPE_CLASS};
        use pz_scripts::registry::{RegistryKey, ResourceLocation};
        let key = |name: &str| Rc::new(RegistryKey {
            location: ResourceLocation::of(name).unwrap(), name: name.into(), item_key: None,
        });
        let container = key("test:container");
        let other = key("test:normal");
        // Equal spelling does not make two separately allocated Java
        // registry objects identical to the itemType field.
        let duplicate = key("test:container");
        let mut thread = item_thread();
        for (name, value) in [
            ("containerType", key_value(ITEM_TYPE_CLASS, Some(&container))),
            ("otherType", key_value(ITEM_TYPE_CLASS, Some(&other))),
            ("duplicateType", key_value(ITEM_TYPE_CLASS, Some(&duplicate))),
            ("wrongKind", key_value(ITEM_TAG_CLASS, Some(&key("test:tag")))),
        ] {
            thread.globals().set_str(name, value);
        }
        let item = make(ItemClass::Container, "AuthoredBag", |t| t.item_type = Some(container.clone()));
        thread.globals().set_str("item", item_value(&item));
        let values = run(&mut thread, r#"
            return item:getCategory() == "Container",
                item:getItemType() == containerType,
                item:isItemType(containerType), not item:isItemType(otherType),
                not item:isItemType(duplicateType), not item:isItemType(nil)
        "#);
        assert_eq!(values.len(), 6);
        assert!(values.iter().all(|v| matches!(v, Value::Bool(true))));
        let values = run(&mut thread, r#"
            item:setItemType(otherType)
            return item:getItemType() == otherType, item:isItemType(otherType),
                not item:isItemType(containerType), item:getCategory() == "Container"
        "#);
        assert!(values.iter().all(|v| matches!(v, Value::Bool(true))));
        item.borrow_mut().main_category = Some("Custom category".into());
        assert_eq!(run(&mut thread, "return item:getCategory()")[0].as_str(), Some("Custom category"));
        item.borrow_mut().main_category = Some(String::new());
        assert_eq!(run(&mut thread, "return item:getCategory()")[0].as_str(), Some(""));
        item.borrow_mut().main_category = None;
        assert_eq!(run(&mut thread, "return item:getCategory()")[0].as_str(), Some("Container"));
        run(&mut thread, "item:setItemType(nil)");
        let nil = run(&mut thread, "return item:getItemType()");
        assert_eq!(nil.len(), 1);
        assert!(nil[0].is_nil());
        assert!(matches!(run(&mut thread, "return item:isItemType(nil)")[0], Value::Bool(true)));
        let rejected = run(&mut thread, r#"
            local first = pcall(function() item:setItemType(wrongKind) end)
            local second, message = pcall(function() item:isItemType('test:normal') end)
            return first, second, message
        "#);
        assert_eq!(rejected.len(), 3);
        assert!(matches!(rejected[0], Value::Bool(false)));
        assert!(matches!(rejected[1], Value::Bool(false)));
        assert!(pz_kahlua::exec::java_value_of(&rejected[2]).contains("ClassCastException"));
        assert!(item.borrow().item_type.is_none(), "invalid type changed the field");
        let animal = make(ItemClass::Animal, "AuthoredAnimal", |_| {});
        animal.borrow_mut().main_category = Some("Custom category".into());
        thread.globals().set_str("animal", item_value(&animal));
        assert_eq!(run(&mut thread, "return animal:getCategory()")[0].as_str(), Some("Animal"));
    }

    fn num(v: &Value) -> f64 {
        match v {
            Value::Num(n) => *n,
            other => panic!("not a number: {}", pz_kahlua::exec::java_value_of(other)),
        }
    }

    /// `DrainableComboItem.setUsedDelta(d)` is `setCurrentUsesFloat(d)`
    /// (uses, not `Clothing`'s mod-data filter level), and `isFullUses`
    /// compares the uses with `getMaxUses()`.
    #[test]
    fn battery_set_used_delta_sets_uses() {
        let mut t = item_thread();
        let battery = make(ItemClass::Drainable, "Battery", |t| t.use_delta = 0.1);
        battery.borrow_mut().uses = 10;
        t.globals().set_str("item", item_value(&battery));
        let r = run(
            &mut t,
            "local full = item:isFullUses()\n\
             item:setUsedDelta(0.37)\n\
             return full, item:getCurrentUses(), item:getCurrentUsesFloat(), item:isFullUses(),\n\
             item:getUseDelta(), item.getUsedDelta == nil, item.getDeviceData == nil",
        );
        assert!(matches!(r[0], Value::Bool(true)));
        assert_eq!(num(&r[1]), 4.0);
        assert_eq!(num(&r[2]) as f32, 4.0 * 0.1f32);
        assert!(matches!(r[3], Value::Bool(false)));
        assert_eq!(num(&r[4]) as f32, 0.1);
        assert!(matches!(r[5], Value::Bool(true)));
        assert!(matches!(r[6], Value::Bool(true)));
        assert!(battery.borrow().table.is_none(), "no usedDelta mod data");
    }

    /// A gas can's `setCurrentUsesFloat` / `setUsedDelta` run the
    /// drainable `updateWeight` (`WeightEmpty`).
    #[test]
    fn gas_can_set_current_uses_float_updates_weight() {
        let mut t = item_thread();
        let can = make(ItemClass::Drainable, "PetrolCan", |t| {
            t.use_delta = 0.125;
            t.actual_weight = 5.0;
            t.weight_empty = 1.0;
        });
        t.globals().set_str("item", item_value(&can));
        let r = run(
            &mut t,
            "item:setCurrentUsesFloat(0.5)\n\
             local a, w = item:getCurrentUses(), item:getActualWeight()\n\
             item:setUsedDelta(2)\n\
             return a, w, item:getCurrentUses(), item:getActualWeight(), item:isCustomWeight()",
        );
        assert_eq!(num(&r[0]), 4.0);
        assert_eq!(num(&r[1]) as f32, 3.0);
        assert_eq!(num(&r[2]), 8.0);
        assert_eq!(num(&r[3]) as f32, 5.0);
        assert!(matches!(r[4], Value::Bool(true)));
    }

    /// `WeaponPart` has its own `useDelta` for its uses methods, but
    /// `getUseDelta` is `InventoryItem`'s (0.03125).
    #[test]
    fn weapon_part_uses_its_own_use_delta() {
        let mut t = item_thread();
        let part = make(ItemClass::WeaponPart, "GunLight", |t| t.use_delta = 0.25);
        part.borrow_mut().uses = 4;
        t.globals().set_str("item", item_value(&part));
        let r = run(
            &mut t,
            "local before = item:getCurrentUsesFloat()\n\
             item:setUsedDelta(0.5)\n\
             return before, item:getCurrentUses(), item:getCurrentUsesFloat(), item:getMaxUses(),\n\
             item:getUseDelta(), item.getUsedDelta == nil",
        );
        assert_eq!(num(&r[0]), 1.0);
        assert_eq!(num(&r[1]), 2.0);
        assert_eq!(num(&r[2]), 0.5);
        assert_eq!(num(&r[3]), 4.0);
        assert_eq!(num(&r[4]) as f32, 0.03125);
        assert!(matches!(r[5], Value::Bool(true)));
    }

    /// `Clothing.setUsedDelta` keeps the filter level in the mod data
    /// (clamped at 0) and leaves the uses; `getUseDelta` is the script's.
    #[test]
    fn clothing_set_used_delta_is_mod_data() {
        let mut t = item_thread();
        let mask = make(ItemClass::Clothing, "Hat_GasMask", |t| t.use_delta = 0.2);
        let uses = mask.borrow().uses;
        t.globals().set_str("item", item_value(&mask));
        let r = run(
            &mut t,
            "item:setUsedDelta(0.75)\n\
             local a = item:getUsedDelta()\n\
             item:setUsedDelta(-2)\n\
             return a, item:getUsedDelta(), item:getCurrentUses(), item:getUseDelta()",
        );
        assert_eq!(num(&r[0]), 0.75);
        assert_eq!(num(&r[1]), 0.0);
        assert_eq!(num(&r[2]), uses as f64);
        assert_eq!(num(&r[3]) as f32, 0.2);
    }

    /// A plain item has no `setUsedDelta` or `getDeviceData`, and
    /// `InventoryItem.setCurrentUsesFloat` divides by
    /// `InventoryItem.useDelta`, not the script's.
    #[test]
    fn combo_item_uses_the_base_use_delta() {
        let mut t = item_thread();
        let item = make(ItemClass::Normal, "Tire", |t| t.use_delta = 0.5);
        t.globals().set_str("item", item_value(&item));
        let r = run(
            &mut t,
            "item:setCurrentUsesFloat(0.5)\n\
             return item.setUsedDelta == nil, item.getDeviceData == nil, item:getCurrentUses(),\n\
             item:getUseDelta(), item:getCurrentUsesFloat()",
        );
        assert!(matches!(r[0], Value::Bool(true)));
        assert!(matches!(r[1], Value::Bool(true)));
        assert_eq!(num(&r[2]), 16.0);
        assert_eq!(num(&r[3]) as f32, 0.03125);
        assert_eq!(num(&r[4]), 16.0);
    }

    /// `Radio.getCurrentUsesFloat` is the device power, and only a radio
    /// has `getDeviceData`.
    #[test]
    fn radio_uses_float_is_device_power() {
        let mut t = item_thread();
        let radio = make(ItemClass::Radio, "RadioRed", |_| {});
        if let ItemKind::Radio(_, d) = &mut radio.borrow_mut().kind {
            d.power_delta = 0.625;
        }
        t.globals().set_str("item", item_value(&radio));
        let r = run(
            &mut t,
            "return item:getCurrentUsesFloat(), item:getDeviceData() ~= nil",
        );
        assert_eq!(num(&r[0]), 0.625);
        assert!(matches!(r[1], Value::Bool(true)));
    }

    /// `getName` dispatches per class: the broken prefix for a plain item,
    /// food state for food; `getDisplayName` stays the name field.
    #[test]
    fn get_name_dispatches_per_class() {
        let mut t = item_thread();
        let plain = make(ItemClass::Normal, "Plank", |_| {});
        let food = make(ItemClass::Food, "Apple", |t| t.days_fresh = 5);
        t.globals().set_str("plain", item_value(&plain));
        t.globals().set_str("food", item_value(&food));
        let r = run(
            &mut t,
            "local a = plain:getName()\n\
             plain:setCondition(0)\n\
             return a, plain:getName(), plain:getDisplayName(), food:getName(), food:getDisplayName()",
        );
        let s = |v: &Value| pz_kahlua::exec::java_value_of(v);
        assert_eq!(s(&r[0]), "Plank");
        // Without a translator `getText` gives the key.
        assert_eq!(s(&r[1]), "IGUI_ClothingNaming");
        assert_eq!(s(&r[2]), "Plank");
        assert_eq!(s(&r[3]), "IGUI_FoodNaming");
        assert_eq!(s(&r[4]), "Apple");
    }

    /// `HandWeapon.setActivated` switches its light part too.
    #[test]
    fn weapon_set_activated_switches_its_light() {
        let mut t = item_thread();
        let gun = make(ItemClass::Weapon, "Pistol", |_| {});
        let mut light = InventoryItem::create(
            Rc::new({
                let mut t = ItemTemplate::new(ItemClass::WeaponPart, "Base", "GunLight");
                t.torch_cone = true;
                t
            }),
            1,
        );
        light.torch_cone = true;
        if let ItemKind::Weapon(w) = &mut gun.borrow_mut().kind {
            w.attachments.push(light);
        }
        t.globals().set_str("item", item_value(&gun));
        run(&mut t, "item:setActivated(true)");
        let g = gun.borrow();
        let ItemKind::Weapon(w) = &g.kind else {
            unreachable!()
        };
        assert!(g.activated && w.attachments[0].activated);
    }

    /// `Food.getBoredomChange` / `getUnhappyChange` add the state
    /// penalties one at a time, so a `-0.0` change stays `-0.0`.
    #[test]
    fn food_negative_zero_changes_stay_negative() {
        let mut t = item_thread();
        let food = make(ItemClass::Food, "Gum", |_| {});
        if let ItemKind::Food(f) = &mut food.borrow_mut().kind {
            f.boredom_change = -0.0;
            f.unhappy_change = -0.0;
        }
        t.globals().set_str("item", item_value(&food));
        let r = run(
            &mut t,
            "return item:getBoredomChange(), item:getUnhappyChange()",
        );
        assert!(num(&r[0]) == 0.0 && num(&r[0]).is_sign_negative());
        assert!(num(&r[1]) == 0.0 && num(&r[1]).is_sign_negative());
    }
}
