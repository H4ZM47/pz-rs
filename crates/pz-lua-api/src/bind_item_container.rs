//! `zombie.inventory.ItemContainer` exposed to Lua over
//! [`pz_inventory::shared`] containers.
//!
//! A container is shared as a [`ContainerRef`]; [`container_value`] gives
//! one Lua object per container while that object lives (Lua `==` is Java
//! identity), and its items are the item objects of
//! [`crate::bind_items::item_value`]. `getItems()` is a live
//! `ArrayList` of them.
//!
//! The parent (`getParent`, an `IsoObject`) is the Lua object the owner
//! gives with [`set_parent`]. What the original reads from the parent or
//! from a character argument (`getInventoryWeight`, `getMaxWeight`,
//! `hasTrait`, `getInventory`, `removeFromHands`, ...) is called on those
//! Lua objects by name, so it is whatever their bindings do. Calls whose
//! result the method uses fail when the binding lacks the method; side
//! effects (`removeFromHands`, `checkClothing`) are skipped then.
//!
//! A bag's inventory (`InventoryContainer.getInventory()`) is the bag
//! item's shared container ([`shared::inventory_of`]), so the `*Recurse`
//! methods and `doInv`/`insideInv` arguments walk real containers and
//! return the items inside bags.
//!
//! Not ported: vehicle part containers (`vehiclePart` is never set, so
//! the vehicle methods answer as for a container outside a vehicle), the
//! world (`DirtySlice`, `flagForHotSave`),
//! networking (offline: `requestSync`, `requestServerItemsForContainer`,
//! `SynchSpawn` and `removeItemOnServer` do nothing), and
//! `Food.updateAge` / `Radio.deviceData` in `OnAddedToContainer`.
//!
//! The `FluidContainer` queries (`FindWaterSource`, `getSoapList`, the
//! `*FluidContainer*`/`*FluidSources` methods, ...) read the items'
//! `pz_entity` `FluidContainer` component, decoded against the item
//! environment's entity registries ([`crate::bind_items::ItemLuaEnv::entity`]).
//!
//! Not bound: the overloads taking Java `Predicate`/`Comparator` objects
//! (`getAll`, `getFirst`, `getSome`, `getCount`, `getBest`, `getBestType`
//! and their `Recurse` forms, `getUsesRecurse`), `ItemType` arguments (`HasType`,
//! `Find`/`Remove(ItemType)`), item behaviour pz-inventory lacks
//! (`FindAndReturnStack`, `getBestWeapon`, `getTotalWeaponScore`,
//! `isEmptyOrUnwanted`, `getAllFoodsForAnimals`, `getItems4Admin`,
//! `getAllItems`, `reset`, `getAnimalInventoryItem`, `hasRecipe`,
//! `getRecipeItem`), the world (`getWorldPosition`, `isShop`), `save`/
//! `load` (use [`SharedContainer::to_container`]/`from_container`) and
//! `getClass`/`notifyAll`.

use crate::bind_items::{item_env, item_value, ItemRef, LuaInventoryItem};
use crate::bind_java_util::{list_value, JavaList};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg};
use pz_core::debug::DebugType;
use pz_inventory::context::{ItemOwner, ItemPlace};
use pz_inventory::item::ItemKind;
use pz_inventory::shared::{self, container_of, set_container, ContainerRef, SharedContainer};
use pz_inventory::{InventoryItem, ItemClass};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const ITEM_CONTAINER_CLASS: &str = "zombie.inventory.ItemContainer";
const ISO_GAME_CHARACTER: &str = "zombie.characters.IsoGameCharacter";
const ISO_PLAYER: &str = "zombie.characters.IsoPlayer";
const ISO_DEAD_BODY: &str = "zombie.iso.objects.IsoDeadBody";
const ISO_MANNEQUIN: &str = "zombie.iso.objects.IsoMannequin";
const ISO_FEEDING_TROUGH: &str = "zombie.iso.objects.IsoFeedingTrough";
const ISO_STOVE: &str = "zombie.iso.objects.IsoStove";
const ISO_BARBECUE: &str = "zombie.iso.objects.IsoBarbecue";
const ISO_FIREPLACE: &str = "zombie.iso.objects.IsoFireplace";
const ISO_THUMPABLE: &str = "zombie.iso.objects.IsoThumpable";
const ISO_WORLD_INVENTORY_OBJECT: &str = "zombie.iso.objects.IsoWorldInventoryObject";

/// A Lua value stored in [`SharedContainer::parent`] /
/// [`SharedContainer::source_grid`].
pub struct LuaRef(pub Value);

/// A container's Lua object.
pub struct LuaItemContainer {
    pub container: ContainerRef,
    /// `getItems()` (the same list object each call).
    items_list: RefCell<Option<Value>>,
}

impl LuaObject for LuaItemContainer {
    fn class_name(&self) -> &str {
        ITEM_CONTAINER_CLASS
    }
    fn to_lua_string(&self) -> String {
        // "ItemContainer:[type:" + getType() + ", parent:" + parent + "]".
        let c = self.container.borrow();
        let parent = java_string(&parent_of(&c));
        format!("ItemContainer:[type:{}, parent:{parent}]", c.kind)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static OBJECTS: RefCell<HashMap<usize, Weak<dyn LuaObject>>> = RefCell::new(HashMap::new());
}

/// The Lua object of `c` (the same object while it is alive). This is
/// what `IsoGameCharacter.getInventory()` returns for a character whose
/// inventory is `c`.
pub fn container_value(c: &ContainerRef) -> Value {
    let key = Rc::as_ptr(c) as *const () as usize;
    OBJECTS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(o) = m.get(&key).and_then(Weak::upgrade) {
            if let Some(l) = o.as_any().downcast_ref::<LuaItemContainer>() {
                if Rc::ptr_eq(&l.container, c) {
                    return Value::Object(o);
                }
            }
        }
        let o: Rc<dyn LuaObject> = Rc::new(LuaItemContainer {
            container: c.clone(),
            items_list: RefCell::new(None),
        });
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(key, Rc::downgrade(&o));
        Value::Object(o)
    })
}

/// The container of `v`, if it is a container object.
pub fn container_of_value(v: &Value) -> Option<ContainerRef> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaItemContainer>()
            .map(|l| l.container.clone()),
        _ => None,
    }
}

/// The container behind argument `i`.
pub fn container_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<ContainerRef> {
    let v = c.arg(i);
    container_of_value(&v).ok_or_else(|| {
        LuaError::runtime(format!(
            "Expected an ItemContainer but got {}",
            pz_kahlua::stdlib::java_class_name(&v)
        ))
    })
}

/// `setParent(parent)` from the engine: the parent's Lua object (`nil`
/// for none).
pub fn set_parent(c: &ContainerRef, parent: Value) {
    c.borrow_mut().parent = (!parent.is_nil()).then(|| Rc::new(LuaRef(parent)) as Rc<dyn Any>);
}

/// A parent that is a character (or another object answering for the
/// items it holds): its Lua object and the [`ItemOwner`] that answers
/// `chr.isEquipped(item)`, `getWornItems().contains(item)`, the hand items
/// and the item's place for the items directly in the container.
pub struct LuaParent {
    pub value: Value,
    pub owner: Rc<dyn ItemOwner>,
}

/// [`set_parent`] for a parent that answers for its items: what the
/// characters binding calls for a character's inventory, so items' owner
/// queries ([`container_owner`]) reach the character.
pub fn set_parent_with_owner(c: &ContainerRef, parent: Value, owner: Rc<dyn ItemOwner>) {
    c.borrow_mut().parent = if parent.is_nil() {
        None
    } else {
        Some(Rc::new(LuaParent {
            value: parent,
            owner,
        }) as Rc<dyn Any>)
    };
}

/// `getParent()`.
pub fn parent_of(c: &SharedContainer) -> Value {
    lua_ref(&c.parent)
}

fn lua_ref(r: &Option<Rc<dyn Any>>) -> Value {
    let Some(p) = r.as_ref() else {
        return Value::Nil;
    };
    if let Some(l) = p.downcast_ref::<LuaRef>() {
        return l.0.clone();
    }
    p.downcast_ref::<LuaParent>()
        .map_or(Value::Nil, |l| l.value.clone())
}

pub(crate) fn source_grid(c: &ContainerRef) -> Value {
    lua_ref(&c.borrow().source_grid)
}

/// The [`ItemOwner`] of the container's parent, when it was set with
/// [`set_parent_with_owner`].
pub fn parent_owner(c: &SharedContainer) -> Option<Rc<dyn ItemOwner>> {
    c.parent
        .as_ref()?
        .downcast_ref::<LuaParent>()
        .map(|l| l.owner.clone())
}

/// The holder of a shared item as the original reaches it through
/// `item.container`: the container, its parent (and the parent's
/// [`ItemOwner`]), and the outermost container's parent.
pub struct ContainerOwner {
    container: Option<ContainerRef>,
    outermost: Option<ContainerRef>,
    /// The direct container's parent is an `IsoGameCharacter`.
    character_parent: bool,
    /// The direct container's parent has a parent value at all.
    has_parent: bool,
    parent: Option<Rc<dyn ItemOwner>>,
    /// The outermost container's parent is an `IsoPlayer`.
    player_outermost: bool,
}

/// The [`ItemOwner`] of `item` from its current container (what
/// [`crate::bind_items::GameItemEnv`] answers for `owner`, and what an
/// [`crate::bind_items::ItemLuaEnv`] of the characters thread can use).
/// Equipped/worn/hand questions go to the parent's owner given with
/// [`set_parent_with_owner`] and only when the item is directly in that
/// container (`isEquipped`/`isWorn` read `container.parent`); without one
/// they answer as for a container without a character.
pub fn container_owner(item: &ItemRef) -> Rc<dyn ItemOwner> {
    let container = container_of(item);
    let outermost = shared::outermost_container_of(item);
    let (character_parent, has_parent, parent) = match &container {
        Some(c) => {
            let s = c.borrow();
            let v = parent_of(&s);
            (
                instance_of(&v, ISO_GAME_CHARACTER),
                !v.is_nil(),
                parent_owner(&s),
            )
        }
        None => (false, false, None),
    };
    let player_outermost = outermost
        .as_ref()
        .is_some_and(|o| instance_of(&parent_of(&o.borrow()), ISO_PLAYER));
    Rc::new(ContainerOwner {
        container,
        outermost,
        character_parent,
        has_parent,
        parent,
        player_outermost,
    })
}

impl ContainerOwner {
    fn character(&self) -> Option<&dyn ItemOwner> {
        if self.character_parent {
            self.parent.as_deref()
        } else {
            None
        }
    }
}

impl ItemOwner for ContainerOwner {
    fn is_equipped(&self, item: &InventoryItem) -> bool {
        self.character().is_some_and(|c| c.is_equipped(item))
    }
    fn is_primary_hand_item(&self, item: &InventoryItem) -> bool {
        self.character()
            .is_some_and(|c| c.is_primary_hand_item(item))
    }
    fn is_secondary_hand_item(&self, item: &InventoryItem) -> bool {
        self.character()
            .is_some_and(|c| c.is_secondary_hand_item(item))
    }
    fn is_worn(&self, item: &InventoryItem) -> bool {
        self.character().is_some_and(|c| c.is_worn(item))
    }
    fn is_in_player_inventory(&self, _item: &InventoryItem) -> bool {
        self.player_outermost
    }
    fn is_in_outermost_container(&self, _item: &InventoryItem) -> bool {
        match (&self.container, &self.outermost) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    fn has_container(&self, _item: &InventoryItem) -> bool {
        self.container.is_some()
    }
    fn place(&self, item: &InventoryItem) -> ItemPlace {
        match (&self.container, &self.parent) {
            (None, _) => ItemPlace::Nowhere,
            (Some(_), Some(p)) => p.place(item),
            (Some(_), None) if !self.has_parent => ItemPlace::NoParent,
            (Some(_), None) => ItemPlace::OtherParent,
        }
    }
    fn world_item_fluid_amount(&self, item: &InventoryItem) -> Option<f32> {
        self.parent
            .as_ref()
            .and_then(|p| p.world_item_fluid_amount(item))
    }
    fn container_temperature(&self) -> f32 {
        self.parent
            .as_ref()
            .map_or(1.0, |p| p.container_temperature())
    }
}

/// `item.getContainer()` as a Lua value.
pub fn item_container_value(item: &ItemRef) -> Value {
    container_of(item).map_or(Value::Nil, |c| container_value(&c))
}

// ---- helpers ----------------------------------------------------------------

fn this(c: &NativeCall<'_>) -> LuaResult<ContainerRef> {
    container_arg(c, 0)
}

fn items(c: &ContainerRef) -> Vec<ItemRef> {
    c.borrow().items.clone()
}

fn opt_item(i: Option<ItemRef>) -> Value {
    i.map_or(Value::Nil, |i| item_value(&i))
}

fn item_list(v: Vec<ItemRef>) -> Value {
    list_value(v.iter().map(item_value).collect())
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: "null".into(),
        class: "java.lang.NullPointerException",
    }
}

fn class_cast(v: &Value, to: &str) -> LuaError {
    LuaError::Runtime {
        message: format!(
            "class {} cannot be cast to class {to}",
            pz_kahlua::stdlib::java_class_name(v)
        ),
        class: "java.lang.ClassCastException",
    }
}

/// `String.valueOf(o)`.
fn java_string(v: &Value) -> String {
    match v {
        Value::Nil => "null".into(),
        Value::Object(o) => o.to_lua_string(),
        Value::Str(s) => s.to_string(),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

/// An item argument (`null` is `None`).
fn opt_item_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ItemRef>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<LuaInventoryItem>() {
            Some(l) => Ok(Some(l.item.clone())),
            None => Err(class_cast(
                &Value::Object(o.clone()),
                "zombie.inventory.InventoryItem",
            )),
        },
        other => Err(class_cast(&other, "zombie.inventory.InventoryItem")),
    }
}

fn is_item(v: &Value) -> bool {
    matches!(v, Value::Object(o) if o.as_any().is::<LuaInventoryItem>())
}

/// A `String` argument; numbers convert as Kahlua converts them, objects
/// (`ItemKey`, `ItemTag`) give their `toString()`.
fn opt_str_arg(c: &NativeCall<'_>, i: usize) -> Option<String> {
    match c.arg(i) {
        Value::Str(s) => Some(s.to_string()),
        Value::Num(n) => Some(pz_kahlua::number::number_to_string(n)),
        Value::Object(o) => Some(o.to_lua_string()),
        _ => None,
    }
}

/// A `String` the method dereferences (`null` throws).
fn str_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<String> {
    opt_str_arg(c, i).ok_or_else(npe)
}

/// A `LuaClosure` (`Objects.requireNonNull`).
fn fn_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Value> {
    match c.arg(i) {
        Value::Nil => Err(npe()),
        v => Ok(v),
    }
}

/// An `ArrayList` result argument.
fn list_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<Rc<dyn LuaObject>>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) if o.as_any().is::<JavaList>() => Ok(Some(o)),
        other => Err(class_cast(&other, "java.util.ArrayList")),
    }
}

fn as_list(o: &Rc<dyn LuaObject>) -> &JavaList {
    o.as_any().downcast_ref::<JavaList>().unwrap()
}

/// `obj instanceof class` for a Lua object.
fn instance_of(v: &Value, class: &str) -> bool {
    match v {
        Value::Object(o) => o.class_name() == class || o.super_classes().contains(&class),
        _ => false,
    }
}

/// `obj.name(args)` through the object's Lua binding.
fn call(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "ItemContainer: {}.{name} is not bound",
            pz_kahlua::stdlib::java_class_name(obj)
        )));
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

/// `obj.name(args)`, or `default` (the `IsoObject` implementation's
/// answer) when the binding has no such method.
fn call_or(
    t: &mut Thread,
    obj: &Value,
    name: &str,
    args: &[Value],
    default: Value,
) -> LuaResult<Value> {
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Ok(default);
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

/// `LuaManager.getFunctionObject(name)`: a global function, or a dotted
/// path through global tables (`nil` with an error log otherwise).
pub(crate) fn function_object(t: &Thread, name: &str) -> Value {
    if name.is_empty() {
        return Value::Nil;
    }
    let mut table = t.globals().clone();
    let parts: Vec<&str> = name.split('.').collect();
    for p in &parts[..parts.len() - 1] {
        match table.rawget(&Value::str(p)) {
            Value::Table(tb) => table = tb,
            _ => {
                DebugType::General.error(None, &format!("no such function \"{name}\""));
                return Value::Nil;
            }
        }
    }
    match table.rawget(&Value::str(parts[parts.len() - 1])) {
        f @ (Value::Closure(_) | Value::Native(_)) => f,
        _ => {
            DebugType::General.error(None, &format!("no such function \"{name}\""));
            Value::Nil
        }
    }
}

/// A side-effect call, skipped when the binding has no such method.
fn call_if_bound(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<()> {
    let f = t.table_get(obj, &Value::str(name))?;
    if !f.is_nil() {
        let mut all = vec![obj.clone()];
        all.extend_from_slice(args);
        t.call1(&f, &all)?;
    }
    Ok(())
}

fn num(v: &Value) -> f64 {
    pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0)
}

/// A static field of an exposed class (`CharacterTrait.ORGANIZED`).
fn static_field(t: &Thread, class: &str, field: &str) -> Value {
    match t.globals().get_str(class) {
        Value::Table(tb) => tb.get_str(field),
        _ => Value::Nil,
    }
}

/// A global function's result (`getGameTime()`).
fn global_call(t: &mut Thread, name: &str) -> LuaResult<Value> {
    let f = t.globals().get_str(name);
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "ItemContainer: {name} is not bound"
        )));
    }
    t.call1(&f, &[])
}

/// `ItemContainer.floatingPointCorrection(val)`.
pub fn floating_point_correction(val: f32) -> f32 {
    let tmp = val * 100.0f32;
    let r = if tmp - (tmp as i32) as f32 >= 0.5f32 {
        tmp + 1.0f32
    } else {
        tmp
    };
    (r as i32) as f32 / 100.0f32
}

/// `InventoryItem.getCategory()` (`mainCategory` is never set).
fn category(i: &InventoryItem) -> &'static str {
    match i.template.class {
        ItemClass::Food => "Food",
        ItemClass::Weapon => "Weapon",
        ItemClass::Clothing => "Clothing",
        ItemClass::AlarmClock | ItemClass::AlarmClockClothing => "AlarmClock",
        ItemClass::Container => "Container",
        ItemClass::Literature => "Literature",
        ItemClass::Key => "Key",
        ItemClass::WeaponPart => "WeaponPart",
        ItemClass::Animal => "Animal",
        _ => "Item",
    }
}

/// `compareType(type1, type2)`: `type1` may list variants with `/`.
fn compare_type(type1: &str, type2: &str) -> bool {
    if type1.contains('/') {
        let Some(p) = type1.find(type2) else {
            return false;
        };
        let b = type1.as_bytes();
        let before = if p > 0 { b[p - 1] } else { 0 };
        let end = p + type2.len();
        let after = if end < b.len() { b[end] } else { 0 };
        return before == 0 && after == b'/'
            || before == b'/' && after == 0
            || before == b'/' && after == b'/';
    }
    type1 == type2
}

/// `compareType(type, item)`.
fn compare_type_item(ty: &str, i: &InventoryItem) -> bool {
    if !ty.contains('.') {
        return compare_type(ty, i.get_type());
    }
    compare_type(ty, &i.get_full_type()) || compare_type(ty, i.get_type())
}

fn is_drainable(i: &InventoryItem) -> bool {
    i.template.class == ItemClass::Drainable
}

/// `instanceof Drainable` (`DrainableComboItem`, `WeaponPart`).
fn is_drainable_interface(i: &InventoryItem) -> bool {
    matches!(
        i.template.class,
        ItemClass::Drainable | ItemClass::WeaponPart
    )
}

/// `item instanceof InventoryContainer ? item.getInventory() : null`.
fn bag(i: &ItemRef) -> Option<ContainerRef> {
    shared::inventory_of(&i.borrow())
}

/// `protectedCallBoolean(f, args) == Boolean.TRUE` (errors are false).
fn call_bool(t: &mut Thread, f: &Value, args: &[Value]) -> bool {
    matches!(t.pcall(f, args).as_deref(), Ok([Value::Bool(true), ..]))
}

/// `EvalComparator.compare`: `Double.compare(result, 0)`, 0 unless the
/// call returns a number.
fn call_compare(t: &mut Thread, f: &Value, args: &[Value]) -> i32 {
    match t.pcall(f, args).as_deref() {
        Ok([Value::Num(v), ..]) => {
            let v = *v;
            if v.is_nan() || v > 0.0 {
                1
            } else if v < 0.0 || (v == 0.0 && v.is_sign_negative()) {
                -1
            } else {
                0
            }
        }
        _ => 0,
    }
}

// ---- predicates -------------------------------------------------------------

/// The original's `Predicate<InventoryItem>` classes.
enum Pred {
    Type(String),
    Tag(String),
    Category(String),
    Eval(Value),
    EvalArg(Value, Value),
    TagEval(String, Value),
    TagEvalArg(String, Value, Value),
    TypeEval(String, Value),
    TypeEvalArg(String, Value, Value),
}

impl Pred {
    /// The part that needs no Lua call.
    fn plain(&self, i: &InventoryItem) -> bool {
        match self {
            Pred::Type(t) | Pred::TypeEval(t, _) | Pred::TypeEvalArg(t, ..) => {
                compare_type_item(t, i)
            }
            Pred::Tag(t) | Pred::TagEval(t, _) | Pred::TagEvalArg(t, ..) => i.has_tag(t),
            Pred::Category(cat) => category(i) == cat,
            Pred::Eval(_) | Pred::EvalArg(..) => true,
        }
    }

    fn function(&self) -> Option<(&Value, Option<&Value>)> {
        match self {
            Pred::Eval(f) | Pred::TagEval(_, f) | Pred::TypeEval(_, f) => Some((f, None)),
            Pred::EvalArg(f, a) | Pred::TagEvalArg(_, f, a) | Pred::TypeEvalArg(_, f, a) => {
                Some((f, Some(a)))
            }
            _ => None,
        }
    }

    fn test(&self, t: &mut Thread, r: &ItemRef) -> bool {
        if !self.plain(&r.borrow()) {
            return false;
        }
        match self.function() {
            None => true,
            Some((f, None)) => call_bool(t, f, &[item_value(r)]),
            Some((f, Some(a))) => call_bool(t, f, &[item_value(r), a.clone()]),
        }
    }
}

/// `getSome`/`getSomeRecurse`/`getAll`/`getAllRecurse`/`getFirst*`: the
/// matches in the original's order (this level, then each bag's
/// inventory). `limit` is `count` against `base + out.len()` (the result
/// list's size).
struct Walk<'a, 'b> {
    t: &'a mut Thread,
    pred: &'b Pred,
    limit: Option<(usize, i32)>,
    out: Vec<ItemRef>,
}

impl Walk<'_, '_> {
    fn full(&self) -> bool {
        self.limit
            .is_some_and(|(base, count)| (base + self.out.len()) as i64 >= count as i64)
    }

    fn walk(&mut self, c: &ContainerRef, recurse: bool) -> bool {
        let mut bags = Vec::new();
        for r in items(c) {
            if self.pred.test(self.t, &r) {
                self.out.push(r.clone());
                if self.full() {
                    return true;
                }
            }
            if recurse {
                if let Some(b) = bag(&r) {
                    bags.push(b);
                }
            }
        }
        for b in bags {
            if self.walk(&b, true) {
                return true;
            }
        }
        false
    }
}

fn collect(
    t: &mut Thread,
    c: &ContainerRef,
    pred: &Pred,
    recurse: bool,
    limit: Option<(usize, i32)>,
) -> LuaResult<Vec<ItemRef>> {
    let mut w = Walk {
        t,
        pred,
        limit,
        out: Vec::new(),
    };
    w.walk(c, recurse);
    Ok(w.out)
}

fn first(
    t: &mut Thread,
    c: &ContainerRef,
    pred: &Pred,
    recurse: bool,
) -> LuaResult<Option<ItemRef>> {
    Ok(collect(t, c, pred, recurse, Some((0, 1)))?
        .into_iter()
        .next())
}

/// The original's `Comparator<InventoryItem>` classes.
enum Cmp {
    Condition,
    Eval(Value),
    EvalArg(Value, Value),
}

fn compare(t: &mut Thread, cmp: &Cmp, a: &ItemRef, b: &ItemRef) -> LuaResult<i32> {
    match cmp {
        Cmp::Condition => Ok(a.borrow().condition - b.borrow().condition),
        Cmp::Eval(f) | Cmp::EvalArg(f, _) => {
            let mut args = vec![item_value(a), item_value(b)];
            if let Cmp::EvalArg(_, x) = cmp {
                args.push(x.clone());
            }
            Ok(call_compare(t, f, &args))
        }
    }
}

/// `getBest`/`getBestRecurse`: `getBestOf(getAll(predicate))`.
fn best(
    t: &mut Thread,
    c: &ContainerRef,
    pred: &Pred,
    cmp: &Cmp,
    recurse: bool,
) -> LuaResult<Option<ItemRef>> {
    let mut all = collect(t, c, pred, recurse, None)?.into_iter();
    let Some(mut b) = all.next() else {
        return Ok(None);
    };
    for i in all {
        if compare(t, cmp, &i, &b)? > 0 {
            b = i;
        }
    }
    Ok(Some(b))
}

/// `getBestCondition`: no item when the best is broken.
fn best_condition(
    t: &mut Thread,
    c: &ContainerRef,
    pred: &Pred,
    recurse: bool,
) -> LuaResult<Option<ItemRef>> {
    match best(t, c, pred, &Cmp::Condition, recurse)? {
        Some(b) if b.borrow().condition <= 0 => Ok(None),
        b => Ok(b),
    }
}

/// `getUses(items)`: drainables count their uses, other items one.
fn uses(f: &[ItemRef]) -> i32 {
    f.iter()
        .map(|f| {
            let i = f.borrow();
            if is_drainable(&i) {
                i.get_current_uses()
            } else {
                1
            }
        })
        .sum()
}

// ---- the parent -------------------------------------------------------------

fn parent(c: &ContainerRef) -> Value {
    parent_of(&c.borrow())
}

/// `getCharacter()`: the parent, or the character of the bag's container.
fn character(c: &ContainerRef) -> Value {
    let p = parent(c);
    if instance_of(&p, ISO_GAME_CHARACTER) {
        return p;
    }
    if let Some(bag) = containing_item(c) {
        if let Some(outer) = container_of(&bag) {
            return character(&outer);
        }
    }
    Value::Nil
}

fn containing_item(c: &ContainerRef) -> Option<ItemRef> {
    shared::containing_item(c)
}

/// `getCapacity()`.
fn capacity(c: &ContainerRef) -> i32 {
    let cap = c.borrow().capacity;
    if containing_item(c).is_some() {
        cap.min(50)
    } else {
        cap.min(100)
    }
}

/// `getContentsWeight()`.
fn contents_weight(c: &ContainerRef) -> f32 {
    let e = item_env();
    let ctx = e.context();
    let mut total = 0.0f32;
    for r in items(c) {
        let o = e.owner(&r);
        total += r.borrow().get_unequipped_weight(ctx, o.as_ref());
    }
    total
}

/// `getCapacityWeight()`.
fn capacity_weight(t: &mut Thread, c: &ContainerRef) -> LuaResult<f32> {
    let p = parent(c);
    if instance_of(&p, ISO_PLAYER) {
        let debug = crate::bind_core::shared_core().borrow().debug;
        if debug && call(t, &p, "isGhostMode", &[])?.truthy() {
            return Ok(0.0);
        }
        if call(t, &p, "isUnlimitedCarry", &[])?.truthy() {
            return Ok(0.0);
        }
    }
    if instance_of(&p, ISO_GAME_CHARACTER) || instance_of(&p, ISO_DEAD_BODY) {
        return Ok(num(&call(t, &p, "getInventoryWeight", &[])?) as f32);
    }
    Ok(contents_weight(c))
}

/// `getMaxWeight()`.
fn max_weight(t: &mut Thread, c: &ContainerRef) -> LuaResult<f32> {
    let p = parent(c);
    if instance_of(&p, ISO_GAME_CHARACTER) {
        return Ok(num(&call(t, &p, "getMaxWeight", &[])?) as f32);
    }
    Ok(capacity(c) as f32)
}

fn has_trait(t: &mut Thread, chr: &Value, name: &str) -> LuaResult<bool> {
    let tr = static_field(t, "CharacterTrait", name);
    Ok(call(t, chr, "hasTrait", &[tr])?.truthy())
}

/// `getEffectiveCapacity(chr)`.
fn effective_capacity(t: &mut Thread, c: &ContainerRef, chr: &Value) -> LuaResult<i32> {
    let cap = capacity(c);
    let p = parent(c);
    let floor = c.borrow().kind == "floor";
    if !(chr.is_nil()
        || instance_of(&p, ISO_GAME_CHARACTER)
        || instance_of(&p, ISO_DEAD_BODY)
        || floor)
    {
        if has_trait(t, chr, "ORGANIZED")? {
            return Ok((cap as f32 * 1.3f32).max((cap + 1) as f32) as i32);
        }
        if has_trait(t, chr, "DISORGANIZED")? {
            return Ok((cap as f32 * 0.7f32).max(1.0) as i32);
        }
    }
    Ok(cap)
}

/// `isInCharacterInventory(chr)`.
fn in_character_inventory(t: &mut Thread, c: &ContainerRef, chr: &Value) -> LuaResult<bool> {
    if chr.is_nil() {
        return Err(npe());
    }
    let inv = call(t, chr, "getInventory", &[])?;
    if container_of_value(&inv).is_some_and(|i| Rc::ptr_eq(&i, c)) {
        return Ok(true);
    }
    if let Some(bag) = containing_item(c) {
        if let Some(i) = container_of_value(&inv) {
            if contains_in(t, &i, &bag, true)? {
                return Ok(true);
            }
        }
        if let Some(outer) = container_of(&bag) {
            return in_character_inventory(t, &outer, chr);
        }
    }
    Ok(false)
}

/// `contains(item, doInv)`: `findItem` by identity.
fn contains_in(_t: &mut Thread, c: &ContainerRef, item: &ItemRef, do_inv: bool) -> LuaResult<bool> {
    Ok(find_ref(c, &|r| Rc::ptr_eq(r, item), do_inv).is_some())
}

/// `isItemAllowed(item)`.
fn is_item_allowed(t: &mut Thread, c: &ContainerRef, item: Option<&ItemRef>) -> LuaResult<bool> {
    let Some(item) = item else {
        return Ok(false);
    };
    let (kind, only, accept) = {
        let s = c.borrow();
        (
            s.kind.clone(),
            s.only_accept_category.clone(),
            s.accept_item_function.clone(),
        )
    };
    let p = parent(c);
    {
        let i = item.borrow();
        if i.template.class == ItemClass::Animal && kind != "floor" {
            return Ok(false);
        }
        if i.get_type().contains("Corpse") && instance_of(&p, ISO_DEAD_BODY) {
            return Ok(false);
        }
        if let Some(cat) = &only {
            if !cat.eq_ignore_ascii_case(category(&i)) {
                return Ok(false);
            }
        }
    }
    if let Some(name) = accept {
        let f = function_object(t, &name);
        if !f.is_nil() && !call_bool(t, &f, &[container_value(c), item_value(item)]) {
            return Ok(false);
        }
    }
    if !p.is_nil()
        && !call_or(
            t,
            &p,
            "isItemAllowedInContainer",
            &[container_value(c), item_value(item)],
            Value::Bool(true),
        )?
        .truthy()
    {
        return Ok(false);
    }
    {
        let i = item.borrow();
        let clothing = matches!(
            i.kind,
            ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
        );
        if kind == "clothingrack" && !clothing && !i.has_tag("base:fitsclothingrack") {
            return Ok(false);
        }
    }
    if !p.is_nil() {
        let props = call_or(t, &p, "getProperties", &[], Value::Nil)?;
        if !props.is_nil() {
            let custom = static_field(t, "IsoPropertyType", "CUSTOM_NAME");
            let name = call(t, &props, "get", &[custom])?;
            if java_string(&name) == "Toaster"
                && !name.is_nil()
                && !item.borrow().has_tag("base:fitstoaster")
            {
                return Ok(false);
            }
            let group = static_field(t, "IsoPropertyType", "GROUP_NAME");
            let g = call(t, &props, "get", &[group])?;
            if !g.is_nil() {
                let g = java_string(&g);
                if (g == "Coffee" || g == "Espresso") && !item.borrow().has_tag("base:coffeemaker")
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// `hasRoomFor(chr, weightVal, weightAddedToFloor)` without vehicle seats
/// or a bag's world item (neither is shared yet).
fn has_room_for_weight(
    t: &mut Thread,
    c: &ContainerRef,
    chr: &Value,
    w: f32,
    _floor: f32,
) -> LuaResult<bool> {
    // `inventoryContainer.getMaxItemSize()`: the bag's script limit.
    if let Some(b) = containing_item(c) {
        let max = b.borrow().template.max_item_size;
        if max > 0.0 && w > max {
            return Ok(false);
        }
    }
    let cw = capacity_weight(t, c)?;
    Ok(floating_point_correction(cw) + w <= effective_capacity(t, c, chr)? as f32)
}

/// `isPowered()`.
pub(crate) fn is_powered(t: &mut Thread, c: &ContainerRef) -> LuaResult<bool> {
    let p = parent(c);
    if p.is_nil() {
        return Ok(false);
    }
    if num(&call_or(t, &p, "getObjectIndex", &[], Value::Num(-1.0))?) == -1.0 {
        return Ok(false);
    }
    Ok(call(t, &p, "checkObjectPowered", &[])?.truthy())
}

/// `isFridge()`.
pub(crate) fn is_fridge(t: &mut Thread, c: &ContainerRef) -> LuaResult<bool> {
    let (fridge, freezer, none) = {
        let s = c.borrow();
        (s.is_fridge_type(), s.is_freezer(), s.kind == "none")
    };
    if none || freezer {
        return Ok(false);
    }
    if fridge {
        return Ok(true);
    }
    let p = parent(c);
    if p.is_nil() {
        return Ok(false);
    }
    let props = call_or(t, &p, "getProperties", &[], Value::Nil)?;
    if props.is_nil() {
        return Ok(false);
    }
    let k = static_field(t, "IsoPropertyType", "IS_FRIDGE");
    Ok(call(t, &props, "has", &[k])?.truthy())
}

fn is_stove(c: &SharedContainer) -> bool {
    matches!(c.kind.as_str(), "stove" | "toaster" | "coffeemaker")
}

/// `isFridgeOrFreezerWarming()`.
fn is_fridge_or_freezer_warming(t: &mut Thread, c: &ContainerRef) -> LuaResult<bool> {
    if !is_fridge(t, c)? && !c.borrow().is_freezer() {
        return Ok(false);
    }
    let gt = global_call(t, "getGameTime")?;
    let days = num(&call(t, &gt, "getWorldAgeDaysSinceBegin", &[])?) as i32;
    let so = global_call(t, "getSandboxOptions")?;
    let shut = num(&call(t, &so, "getElecShutModifier", &[])?) as i32;
    Ok(days == shut && (num(&call(t, &gt, "getTimeOfDay", &[])?) as f32) < 13.0)
}

/// `getTemperature()`.
pub(crate) fn temperature(t: &mut Thread, c: &ContainerRef) -> LuaResult<f32> {
    let custom = c.borrow().custom_temperature;
    if custom != 0.0 {
        return Ok(custom);
    }
    let p = parent(c);
    if is_powered(t, c)? && (is_fridge(t, c)? || c.borrow().is_freezer()) {
        return Ok(0.2);
    }
    let stove = {
        let s = c.borrow();
        is_stove(&s) || s.kind == "microwave"
    };
    if is_powered(t, c)? && stove && instance_of(&p, ISO_STOVE) {
        return Ok(num(&call(t, &p, "getCurrentTemperature", &[])?) as f32);
    }
    if instance_of(&p, ISO_BARBECUE) || instance_of(&p, ISO_FIREPLACE) {
        return Ok(num(&call(t, &p, "getTemperature", &[])?) as f32);
    }
    if is_fridge_or_freezer_warming(t, c)? {
        let gt = global_call(t, "getGameTime")?;
        let tod = num(&call(t, &gt, "getTimeOfDay", &[])?) as f32;
        // GameTime.Lerp(0.2, 1.0, delta).
        let delta = ((tod - 7.0f32) / 6.0f32).clamp(0.0, 1.0);
        return Ok(0.2f32 + (1.0f32 - 0.2f32) * delta);
    }
    Ok(1.0)
}

// ---- adding and removing ----------------------------------------------------

/// The `drawDirty`/`dirty` flags the add methods set.
fn mark_added(c: &ContainerRef, dirty_with_character: bool) {
    let p = parent(c);
    let mut s = c.borrow_mut();
    s.draw_dirty = true;
    if !p.is_nil() && (dirty_with_character || !instance_of(&p, ISO_GAME_CHARACTER)) {
        s.dirty = true;
    }
}

/// `AddItem(InventoryItem)`.
pub fn add_item(
    t: &mut Thread,
    c: &ContainerRef,
    item: Option<ItemRef>,
) -> LuaResult<Option<ItemRef>> {
    let Some(item) = item else {
        return Ok(None);
    };
    let id = item.borrow().id;
    if let Some(same) = items(c).into_iter().find(|i| i.borrow().id == id) {
        DebugType::General.error(None, "Error, container already has id");
        return Ok(Some(same));
    }
    mark_added(c, true);
    if let Some(old) = container_of(&item) {
        remove(t, &old, &item)?;
    }
    set_container(&item, Some(c));
    c.borrow_mut().items.push(item.clone());
    let p = parent(c);
    if instance_of(&p, ISO_FEEDING_TROUGH) {
        call_if_bound(t, &p, "onFoodAdded", &[])?;
    }
    Ok(Some(item))
}

/// `AddItem(String type)`.
pub fn add_item_type(t: &mut Thread, c: &ContainerRef, ty: &str) -> LuaResult<Option<ItemRef>> {
    mark_added(c, false);
    let sm = crate::bind_registry::shared_script_manager();
    let Some(script) = sm
        .find_item(ty, true)
        .and_then(|s| s.downcast::<pz_scripts::objects::items::Item>())
    else {
        DebugType::General.println(&format!("ERROR: ItemContainer.AddItem: can't find {ty}"));
        return Ok(None);
    };
    if script.borrow().obsolete {
        return Ok(None);
    }
    let Some(item) = crate::bind_items::try_create_item(ty, 1.0)? else {
        return Ok(None);
    };
    crate::bind_items::initialise_item(t, &item);
    set_container(&item, Some(c));
    c.borrow_mut().items.push(item.clone());
    let food = matches!(item.borrow().kind, ItemKind::Food(_));
    if food {
        let temp = temperature(t, c)?;
        if let ItemKind::Food(f) = &mut item.borrow_mut().kind {
            f.heat = temp;
        }
    }
    // STUB: `setItemHeat` of cookable fluid containers is not ported.
    Ok(Some(item))
}

/// `AddItem(String type, float useDelta)` (the `synchSpawn` variant is
/// the same offline).
pub fn add_item_uses(t: &mut Thread, c: &ContainerRef, ty: &str, use_delta: f32) -> LuaResult<bool> {
    mark_added(c, false);
    let Some(item) = crate::bind_items::try_create_item(ty, 1.0)? else {
        return Ok(false);
    };
    crate::bind_items::initialise_item(t, &item);
    if is_drainable_interface(&item.borrow()) {
        let e = item_env();
        let n = (item.borrow().get_max_uses() as f32 * use_delta) as i32;
        item.borrow_mut()
            .set_current_uses_with(n, Some(e.context()));
    }
    set_container(&item, Some(c));
    c.borrow_mut().items.push(item);
    Ok(true)
}

/// The parent's `checkClothing(item)` (dead bodies, mannequins) and
/// `onRemoveFood()` (feeding troughs) after a removal.
fn after_remove(t: &mut Thread, c: &ContainerRef, item: Value) -> LuaResult<()> {
    let p = parent(c);
    if instance_of(&p, ISO_DEAD_BODY) || instance_of(&p, ISO_MANNEQUIN) {
        call_if_bound(t, &p, "checkClothing", &[item])?;
    } else if instance_of(&p, ISO_FEEDING_TROUGH) {
        call_if_bound(t, &p, "onRemoveFood", &[])?;
    }
    Ok(())
}

/// `Remove(InventoryItem)`.
pub fn remove(t: &mut Thread, c: &ContainerRef, item: &ItemRef) -> LuaResult<()> {
    let chr = character(c);
    if !chr.is_nil() {
        call_if_bound(t, &chr, "removeFromHands", &[item_value(item)])?;
    }
    if shared::remove_item(c, item) {
        after_remove(t, c, item_value(item))?;
    }
    Ok(())
}

/// `DoRemoveItem(item)`.
fn do_remove_item(t: &mut Thread, c: &ContainerRef, item: &ItemRef) -> LuaResult<()> {
    let p = parent(c);
    {
        let mut s = c.borrow_mut();
        s.draw_dirty = true;
        if !p.is_nil() {
            s.dirty = true;
        }
        if let Some(i) = s.index_of(item) {
            s.items.remove(i);
        }
    }
    set_container(item, None);
    after_remove(t, c, item_value(item))
}

/// `RemoveOneOf(type, insideInv)`: the first item of the type loses a
/// use or leaves; with `insideInv`, then the bags' inventories (the bag
/// is returned).
fn remove_one_of(c: &ContainerRef, ty: &str, inside: bool) -> Option<ItemRef> {
    let p = parent(c);
    {
        let mut m = c.borrow_mut();
        m.draw_dirty = true;
        if !p.is_nil() && !instance_of(&p, ISO_GAME_CHARACTER) {
            m.dirty = true;
        }
    }
    let e = item_env();
    for r in items(c) {
        let hit = {
            let i = r.borrow();
            i.get_full_type() == ty || i.get_type() == ty
        };
        if !hit {
            continue;
        }
        let uses = r.borrow().get_current_uses();
        if uses > 1 {
            r.borrow_mut()
                .set_current_uses_with(uses - 1, Some(e.context()));
        } else {
            set_container(&r, None);
            let mut m = c.borrow_mut();
            if let Some(i) = m.index_of(&r) {
                m.items.remove(i);
            }
        }
        c.borrow_mut().dirty = true;
        return Some(r);
    }
    if inside {
        for r in items(c) {
            if bag(&r).is_some_and(|b| remove_one_of(&b, ty, inside).is_some()) {
                return Some(r);
            }
        }
    }
    None
}

/// `removeItemWithIDRecurse(id)`.
fn remove_with_id_recurse(t: &mut Thread, c: &ContainerRef, id: i32) -> LuaResult<bool> {
    for r in items(c) {
        if r.borrow().id == id {
            remove(t, c, &r)?;
            return Ok(true);
        }
        if let Some(b) = bag(&r) {
            if remove_with_id_recurse(t, &b, id)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// `getNumberOfItem(findItem, includeReplaceOnDeplete, insideInv)`.
fn number_of_item(c: &ContainerRef, find: &str, rod: bool, inside: bool) -> i32 {
    let mut result = 0;
    for r in items(c) {
        let i = r.borrow();
        if i.get_full_type() == find || i.get_type() == find {
            result += 1;
            continue;
        }
        if inside {
            if let Some(b) = shared::inventory_of(&i) {
                drop(i);
                result += number_of_item(&b, find, false, false);
                continue;
            }
        }
        if !rod || !is_drainable(&i) {
            continue;
        }
        let (Some(r), Some(rf)) = (
            i.get_replace_on_deplete(),
            i.get_replace_on_deplete_full_type(),
        ) else {
            continue;
        };
        if rf == find || r == find {
            result += 1;
        }
    }
    result
}

/// `getItemCountFromTypeRecurse(type)`.
fn count_full_type_recurse(c: &ContainerRef, ty: Option<&str>) -> i32 {
    let mut n = 0;
    for r in items(c) {
        if ty == Some(r.borrow().get_full_type().as_str()) {
            n += 1;
        }
        if let Some(b) = bag(&r) {
            n += count_full_type_recurse(&b, ty);
        }
    }
    n
}

/// `haveThisKeyId(keyId)`: a key, or one on a key ring (a key ring that
/// is not a bag throws as the original's cast does).
fn have_key_id(c: &ContainerRef, key_id: i32) -> LuaResult<Option<ItemRef>> {
    for r in items(c) {
        let ring = {
            let i = r.borrow();
            if let ItemKind::Key { key_id: k, .. } = &i.kind {
                if *k == key_id {
                    drop(i);
                    return Ok(Some(r));
                }
                continue;
            }
            i.get_type() == "KeyRing" || i.has_tag("base:keyring")
        };
        if ring {
            let b = bag(&r).ok_or_else(|| {
                class_cast(&item_value(&r), "zombie.inventory.types.InventoryContainer")
            })?;
            if let Some(k) = have_key_id(&b, key_id)? {
                return Ok(Some(k));
            }
        }
    }
    Ok(None)
}

/// `getItemWithIDRecursiv(id)`.
fn item_with_id_recursive(c: &ContainerRef, id: i64) -> Option<ItemRef> {
    for r in items(c) {
        if r.borrow().id as i64 == id {
            return Some(r);
        }
        if let Some(b) = bag(&r) {
            if b.borrow().items.is_empty() {
                continue;
            }
            if let Some(f) = item_with_id_recursive(&b, id) {
                return Some(f);
            }
        }
    }
    None
}

/// `containsRecursive(item)`.
fn contains_recursive(c: &ContainerRef, item: &ItemRef) -> bool {
    items(c)
        .iter()
        .any(|r| Rc::ptr_eq(r, item) || bag(r).is_some_and(|b| contains_recursive(&b, item)))
}

/// `findItem(type, doInv, ignoreBroken)`.
fn find_item(
    c: &ContainerRef,
    ty: &str,
    do_inv: bool,
    ignore_broken: bool,
) -> LuaResult<Option<ItemRef>> {
    if ty.contains('/') {
        for v in ty.split('/') {
            if let Some(f) = find_item(c, v.trim(), do_inv, ignore_broken)? {
                return Ok(Some(f));
            }
        }
        return Ok(None);
    }
    if ty.contains("Type:") {
        let class: fn(&InventoryItem) -> bool = if ty.contains("Food") {
            |i| matches!(i.kind, ItemKind::Food(_))
        } else if ty.contains("Weapon") {
            if ignore_broken {
                |i| matches!(i.kind, ItemKind::Weapon(_))
            } else {
                |i| matches!(i.kind, ItemKind::Weapon(_)) && !i.is_broken()
            }
        } else if ty.contains("AlarmClock") {
            // "AlarmClockClothing" contains "AlarmClock": the original
            // never reaches its own branch.
            |i| i.template.class == ItemClass::AlarmClock
        } else {
            return Ok(None);
        };
        return Ok(find_by(c, &|i| class(i), do_inv));
    }
    let ty = ty.trim().to_string();
    Ok(find_by(
        c,
        &|i| compare_type_item(&ty, i) && (ignore_broken || !i.is_broken()),
        do_inv,
    ))
}

/// `findItem(predicate, doInv)`: this level, then each bag once.
fn find_by(
    c: &ContainerRef,
    pred: &dyn Fn(&InventoryItem) -> bool,
    do_inv: bool,
) -> Option<ItemRef> {
    find_ref(c, &|r| pred(&r.borrow()), do_inv)
}

/// [`find_by`] with a test of the item object.
fn find_ref(c: &ContainerRef, pred: &dyn Fn(&ItemRef) -> bool, do_inv: bool) -> Option<ItemRef> {
    let mut bags: Vec<(ItemRef, ContainerRef)> = Vec::new();
    for r in items(c) {
        if pred(&r) {
            return Some(r);
        }
        if do_inv && !bags.iter().any(|(b, _)| Rc::ptr_eq(b, &r)) {
            if let Some(b) = bag(&r) {
                bags.push((r, b));
            }
        }
    }
    bags.iter().find_map(|(_, b)| find_ref(b, pred, do_inv))
}

/// `getItemFromType(type, chr, notEquipped, ignoreBroken, includeInv)` /
/// `getItemFromTag`.
fn item_from(
    t: &mut Thread,
    c: &ContainerRef,
    test: &dyn Fn(&InventoryItem) -> bool,
    chr: &Value,
    not_equipped: bool,
    ignore_broken: bool,
    include_inv: bool,
) -> LuaResult<Option<ItemRef>> {
    let mut bags: Vec<(ItemRef, ContainerRef)> = Vec::new();
    for r in items(c) {
        if test(&r.borrow()) {
            if not_equipped
                && !chr.is_nil()
                && call(t, chr, "isEquippedClothing", &[item_value(&r)])?.truthy()
            {
                continue;
            }
            if !ignore_broken || !r.borrow().is_broken() {
                return Ok(Some(r));
            }
            continue;
        }
        if include_inv && !bags.iter().any(|(b, _)| Rc::ptr_eq(b, &r)) {
            if let Some(b) = bag(&r) {
                bags.push((r, b));
            }
        }
    }
    for (_, b) in bags {
        if let Some(f) = item_from(t, &b, test, chr, not_equipped, ignore_broken, include_inv)? {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

// ---- fluids ---------------------------------------------------------------------

/// Stored component-data presence for the legacy read-only presentation
/// helpers below. Encoded carriers are not exposed as live Lua components;
/// the actual getFluidContainer binding uses get_fluid_ref and reports them.
fn has_fluid_container(i: &InventoryItem) -> bool {
    !matches!(i.components.runtime().presence(pz_entity::ComponentType::FluidContainer), pz_entity::EntryPresence::Absent)
}

/// The owner phase of FluidContainer.recalculateCaches. Resolve the actual
/// outermost parent, then dirty the DIRECT container in native write order.
/// Every parent/class operation runs after the container borrow ends.
pub(crate) fn on_fluid_update(item: &ItemRef) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    let borrowed = |operation| RuntimeHostError::Borrowed(operation);
    let Some(mut outer) = shared::container_of(item) else { return Ok(()); };
    if outer.try_borrow().map_err(|_| borrowed("fluid outermost container"))?.kind == "floor" {
        return Ok(());
    }
    loop {
        let containing = outer.try_borrow().map_err(|_| borrowed("fluid containing item"))?
            .containing_item.as_ref().and_then(Weak::upgrade);
        let Some(next) = containing.and_then(|bag| shared::container_of(&bag)) else { break; };
        if next.try_borrow().map_err(|_| borrowed("fluid outer container kind"))?.kind == "floor" { break; }
        outer = next;
    }
    let parent = {
        let container = outer.try_borrow().map_err(|_| borrowed("fluid outermost parent"))?;
        match container.parent.as_ref() {
            None => Value::Nil,
            Some(parent) => {
                if let Some(parent) = parent.downcast_ref::<LuaRef>() { parent.0.clone() }
                else if let Some(parent) = parent.downcast_ref::<LuaParent>() { parent.value.clone() }
                else { return Err(RuntimeHostError::Unsupported {
                    operation: "actual fluid ItemContainer parent type", kind: Some(pz_entity::ComponentType::FluidContainer),
                }); }
            }
        }
    };
    if instance_of(&parent, ISO_PLAYER) {
        // The original reads item.getContainer again after the predicate.
        if let Some(direct) = shared::container_of(item) {
            let mut direct = direct.try_borrow_mut().map_err(|_| borrowed("fluid direct container flags"))?;
            direct.draw_dirty = true;
            direct.dirty = true;
        }
    }
    // Current InventoryItem subclasses inherit the exact return-only
    // GameEntity.onFluidContainerUpdate, authenticated across the installed JAR.
    Ok(())
}

/// `item.getFluidContainer()` decoded against the environment's entity
/// registries.
fn fluid_container(i: &InventoryItem) -> Option<pz_entity::FluidContainer> {
    let e = item_env();
    match e.entity() {
        Some(ctx) => i.get_fluid_container(ctx),
        None => i.get_fluid_container(&pz_entity::BasicContext::default()),
    }
}

/// `FluidInstance.getFluid().getFluidTypeString()` (`None`: no fluid).
fn fluid_name(f: &pz_entity::FluidInstance) -> Option<String> {
    use pz_entity::fluids::FluidRef;
    use pz_entity::Lookup;
    let e = item_env();
    let default = pz_entity::BasicContext::default();
    let ctx = e.entity().unwrap_or(&default);
    match f.fluid.as_ref()? {
        FluidRef::Type(id) => match ctx.fluid_by_id(*id) {
            Lookup::Found(info) => Some(info.name),
            Lookup::Missing => None,
            Lookup::Unknown => {
                pz_entity::fluids::fluid_type_from_id(*id).map(|t| t.name().to_string())
            }
        },
        FluidRef::Modded(s) => match ctx.fluid_by_name(s) {
            Lookup::Found(info) => Some(info.name),
            Lookup::Missing => None,
            Lookup::Unknown => Some(s.clone()),
        },
    }
}

/// `FluidContainer.getPrimaryFluid().getFluidTypeString()`: `None` when
/// empty (or the instance has no fluid).
fn primary_fluid(fc: &pz_entity::FluidContainer) -> Option<String> {
    if fc.is_empty() || fc.fluids.is_empty() {
        return None;
    }
    let mut primary = &fc.fluids[0];
    for test in &fc.fluids {
        if test.amount > primary.amount {
            primary = test;
        }
    }
    fluid_name(primary)
}

/// `FluidContainer.isWaterSource()`.
fn fc_is_water_source(fc: &pz_entity::FluidContainer) -> bool {
    !fc.is_empty()
        && matches!(
            primary_fluid(fc).as_deref(),
            Some("Water" | "TaintedWater" | "CarbonatedWater")
        )
}

/// `InventoryItem.isWaterSource()` (STUB: `isMultiTileMoveable` is false,
/// moveable sprite grids are not in pz-inventory).
fn is_water_source(i: &InventoryItem) -> bool {
    has_fluid_container(i) && fluid_container(i).is_some_and(|fc| fc_is_water_source(&fc))
}

/// `FluidContainer.getRatioForFluid(fluid)`: the instance's share of the
/// container's fluids (`percentage` as `recalculateCaches` sets it).
fn ratio_for_fluid(fc: &pz_entity::FluidContainer, name: &str) -> f32 {
    let total = fc.amount();
    for f in &fc.fluids {
        if fluid_name(f).as_deref() == Some(name) {
            return if total > 0.0 { f.amount / total } else { 0.0 };
        }
    }
    0.0
}

thread_local! {
    /// `ItemContainer.tempList` (a static list the original returns from
    /// `getAllWaterFillables` and `getAvailableFluidContainer`).
    static TEMP_LIST: Value = list_value(Vec::new());
}

/// Refills the static `tempList` and returns it.
fn temp_list(items: Vec<ItemRef>) -> Value {
    TEMP_LIST.with(|v| {
        if let Value::Object(o) = v {
            let l = as_list(o);
            *l.items.borrow_mut() = items.iter().map(item_value).collect();
        }
        v.clone()
    })
}

fn primary_is(i: &InventoryItem, names: &[&str]) -> bool {
    has_fluid_container(i)
        && fluid_container(i).is_some_and(|fc| {
            !fc.is_empty() && primary_fluid(&fc).is_some_and(|p| names.contains(&p.as_str()))
        })
}

/// `getAllWaterFluidSources(includeTainted)`.
fn water_fluid_sources(c: &ContainerRef, tainted: bool) -> Vec<ItemRef> {
    let names: &[&str] = if tainted {
        &["Water", "CarbonatedWater", "TaintedWater"]
    } else {
        &["Water", "CarbonatedWater"]
    };
    items(c)
        .into_iter()
        .filter(|r| primary_is(&r.borrow(), names))
        .collect()
}

/// `getAvailableFluidContainer(type)`: empty fluid containers, and those
/// whose primary fluid is `type` and not full (an empty one of that
/// fluid is listed once per rule, as in the original).
fn available_fluid_containers(c: &ContainerRef, ty: Option<&str>) -> Vec<ItemRef> {
    let mut out = Vec::new();
    for r in items(c) {
        let i = r.borrow();
        if !has_fluid_container(&i) {
            continue;
        }
        let Some(fc) = fluid_container(&i) else {
            continue;
        };
        if fc.is_empty() {
            out.push(r.clone());
        }
        let Some(ty) = ty.filter(|t| !t.is_empty()) else {
            continue;
        };
        let Some(p) = primary_fluid(&fc) else {
            continue;
        };
        if p.eq_ignore_ascii_case(ty) && !fc.is_full() {
            out.push(r.clone());
        }
    }
    out
}

/// `getSoapList(result, includeLiquidSoap)`: equipped bags add their own.
fn soap_list(c: &ContainerRef, list: &JavaList, liquid: bool, need: f64) -> LuaResult<()> {
    let e = item_env();
    for r in items(c) {
        let inner = {
            let i = r.borrow();
            if i.get_full_type() == "Base.Soap2" {
                list.push(item_value(&r))?;
            } else if liquid {
                if let Some(fc) = fluid_container(&i) {
                    if ratio_for_fluid(&fc, "CleaningLiquid") >= 0.499f32
                        && fc.get_amount() as f64 >= need
                    {
                        list.push(item_value(&r))?;
                    }
                }
            }
            match shared::inventory_of(&i) {
                Some(b) if i.is_equipped(e.owner(&r).as_ref()) => Some(b),
                _ => None,
            }
        };
        if let Some(b) = inner {
            soap_list(&b, list, liquid, need)?;
        }
    }
    Ok(())
}

fn register_fluids(b: &crate::expose::ClassBuilder) {
    b.method("getWaterContainerCount", |c| {
        let s = this(c)?;
        let n = items(&s)
            .iter()
            .filter(|r| has_fluid_container(&r.borrow()))
            .count();
        Ok(c.push(n as f64))
    });
    b.method("FindWaterSource", |c| {
        let s = this(c)?;
        let r = items(&s).into_iter().find(|r| {
            let i = r.borrow();
            is_water_source(&i) && !(is_drainable_interface(&i) && i.get_current_uses() <= 0)
        });
        Ok(c.push(opt_item(r)))
    });
    b.method("FindAndReturnWaterItem", |c| {
        let s = this(c)?;
        let uses = int_arg(c, 1);
        let r = items(&s).into_iter().find(|r| {
            let i = r.borrow();
            is_drainable(&i) && is_water_source(&i) && i.get_current_uses() >= uses
        });
        Ok(c.push(opt_item(r)))
    });
    b.method("getAllWaterFillables", |c| {
        let s = this(c)?;
        let v = items(&s)
            .into_iter()
            .filter(|r| {
                let i = r.borrow();
                has_fluid_container(&i) && fluid_container(&i).is_some_and(|fc| fc.is_empty())
            })
            .collect();
        Ok(c.push(temp_list(v)))
    });
    b.method("getAllWaterFluidSources", |c| {
        let s = this(c)?;
        let v = water_fluid_sources(&s, bool_arg(c, 1));
        Ok(c.push(item_list(v)))
    });
    b.method("getFirstWaterFluidSources", |c| {
        let s = this(c)?;
        let v = water_fluid_sources(&s, bool_arg(c, 1));
        if c.nargs() > 2 && bool_arg(c, 2) {
            if let Some(r) = v
                .iter()
                .find(|r| primary_is(&r.borrow(), &["TaintedWater"]))
            {
                return Ok(c.push(item_value(r)));
            }
        }
        Ok(c.push(opt_item(v.into_iter().next())))
    });
    let cleaning = |c: &ContainerRef| -> Vec<ItemRef> {
        items(c)
            .into_iter()
            .filter(|r| primary_is(&r.borrow(), &["Bleach", "CleaningLiquid"]))
            .collect()
    };
    b.method("getAllCleaningFluidSources", move |c| {
        let s = this(c)?;
        Ok(c.push(item_list(cleaning(&s))))
    });
    b.method("getFirstCleaningFluidSources", move |c| {
        let s = this(c)?;
        Ok(c.push(opt_item(cleaning(&s).into_iter().next())))
    });
    b.method("getFirstFluidContainer", |c| {
        let s = this(c)?;
        let ty = opt_str_arg(c, 1).filter(|t| !t.is_empty());
        for r in items(&s) {
            let i = r.borrow();
            if !has_fluid_container(&i) {
                continue;
            }
            let Some(fc) = fluid_container(&i) else {
                continue;
            };
            if fc.is_empty() {
                drop(i);
                return Ok(c.push(item_value(&r)));
            }
            let (Some(ty), Some(p)) = (ty.as_deref(), primary_fluid(&fc)) else {
                continue;
            };
            if p.eq_ignore_ascii_case(ty) {
                drop(i);
                return Ok(c.push(item_value(&r)));
            }
        }
        Ok(c.push(Value::Nil))
    });
    b.method("getAvailableFluidContainer", |c| {
        let s = this(c)?;
        let v = available_fluid_containers(&s, opt_str_arg(c, 1).as_deref());
        Ok(c.push(temp_list(v)))
    });
    b.method("getFirstAvailableFluidContainer", |c| {
        let s = this(c)?;
        let v = available_fluid_containers(&s, opt_str_arg(c, 1).as_deref());
        temp_list(v.clone());
        Ok(c.push(opt_item(v.into_iter().next())))
    });
    b.method("getAvailableFluidContainersCapacity", |c| {
        let s = this(c)?;
        let v = available_fluid_containers(&s, opt_str_arg(c, 1).as_deref());
        temp_list(v.clone());
        let mut volume = 0.0f32;
        for r in &v {
            if let Some(fc) = fluid_container(&r.borrow()) {
                volume += fc.get_free_capacity();
            }
        }
        Ok(c.push(volume as f64))
    });
    b.method("getSoapList", |c| {
        let s = this(c)?;
        let liquid = bool_arg(c, 2);
        let list = match list_arg(c, 1)? {
            Some(l) => l,
            None => match list_value(Vec::new()) {
                Value::Object(o) => o,
                _ => unreachable!(),
            },
        };
        // ZomboidGlobals.cleanStainCleaningFluidAmount (loaded from the
        // Lua table of the same name).
        let need = match c.thread.globals().get_str("ZomboidGlobals") {
            Value::Table(t) => num(&t.get_str("CleanStainCleaningFluidAmount")),
            _ => 0.0,
        };
        soap_list(&s, as_list(&list), liquid, need)?;
        Ok(c.push(Value::Object(list)))
    });
}

// ---- registration -------------------------------------------------------------

fn push_opt(c: &mut NativeCall<'_>, v: LuaResult<Option<ItemRef>>) -> LuaResult<usize> {
    let v = opt_item(v?);
    Ok(c.push(v))
}

/// Builds a predicate from the arguments from 1; returns it and the index
/// of the next argument.
type MkPred = fn(&NativeCall<'_>) -> LuaResult<(Pred, usize)>;

fn p_type(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::Type(str_arg(c, 1)?), 2))
}
fn p_tag(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::Tag(str_arg(c, 1)?), 2))
}
fn p_category(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::Category(str_arg(c, 1)?), 2))
}
fn p_eval(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::Eval(fn_arg(c, 1)?), 2))
}
fn p_eval_arg(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::EvalArg(fn_arg(c, 1)?, c.arg(2)), 3))
}
fn p_tag_eval(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::TagEval(str_arg(c, 1)?, fn_arg(c, 2)?), 3))
}
fn p_tag_eval_arg(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::TagEvalArg(str_arg(c, 1)?, fn_arg(c, 2)?, c.arg(3)), 4))
}
fn p_type_eval(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((Pred::TypeEval(str_arg(c, 1)?, fn_arg(c, 2)?), 3))
}
fn p_type_eval_arg(c: &NativeCall<'_>) -> LuaResult<(Pred, usize)> {
    Ok((
        Pred::TypeEvalArg(str_arg(c, 1)?, fn_arg(c, 2)?, c.arg(3)),
        4,
    ))
}

/// `getFirst*` / `contains*`.
fn reg_first(
    b: &crate::expose::ClassBuilder,
    name: &'static str,
    mk: MkPred,
    recurse: bool,
    contains: bool,
) {
    b.method(name, move |c| {
        let s = this(c)?;
        let (p, _) = mk(c)?;
        let f = first(c.thread, &s, &p, recurse)?;
        if contains {
            return Ok(c.push(f.is_some()));
        }
        push_opt(c, Ok(f))
    });
}

/// `getCount*`.
fn reg_count(b: &crate::expose::ClassBuilder, name: &'static str, mk: MkPred, recurse: bool) {
    b.method(name, move |c| {
        let s = this(c)?;
        let (p, _) = mk(c)?;
        let n = collect(c.thread, &s, &p, recurse, None)?.len();
        Ok(c.push(n as f64))
    });
}

/// Fills the result list (argument `at`, a new list when absent and
/// `optional`) with `f`'s items.
fn fill_result(
    c: &mut NativeCall<'_>,
    at: usize,
    optional: bool,
    null_ok: bool,
    f: impl FnOnce(&mut Thread, usize) -> LuaResult<Vec<ItemRef>>,
) -> LuaResult<usize> {
    let (list, value) = if c.nargs() > at {
        match list_arg(c, at)? {
            Some(o) => (Some(o.clone()), Value::Object(o)),
            None if null_ok => (None, Value::Nil),
            None => return Err(npe()),
        }
    } else if optional {
        (None, Value::Nil)
    } else {
        return Err(npe());
    };
    let base = list.as_ref().map_or(0, |l| as_list(l).len());
    let found = f(c.thread, base)?;
    let vals: Vec<Value> = found.iter().map(item_value).collect();
    match list {
        Some(l) => {
            for v in vals {
                as_list(&l).push(v)?;
            }
            Ok(c.push(value))
        }
        None => Ok(c.push(list_value(vals))),
    }
}

/// `getAll*` (`result` optional for the overloads that create one).
fn reg_all(
    b: &crate::expose::ClassBuilder,
    name: &'static str,
    mk: MkPred,
    recurse: bool,
    optional: bool,
) {
    b.method(name, move |c| {
        let s = this(c)?;
        let (p, at) = mk(c)?;
        let null_ok = name == "getAllTagEval";
        fill_result(c, at, optional, null_ok, |t, _| {
            collect(t, &s, &p, recurse, None)
        })
    });
}

/// `getSome*` (`count`, then an optional `result`).
fn reg_some(
    b: &crate::expose::ClassBuilder,
    name: &'static str,
    mk: MkPred,
    recurse: bool,
    optional: bool,
) {
    b.method(name, move |c| {
        let s = this(c)?;
        let (p, at) = mk(c)?;
        let count = int_arg(c, at);
        fill_result(c, at + 1, optional, false, |t, base| {
            collect(t, &s, &p, recurse, Some((base, count)))
        })
    });
}

fn cmp_eval(c: &NativeCall<'_>, at: usize, arg: Option<usize>) -> LuaResult<Cmp> {
    let f = fn_arg(c, at)?;
    Ok(match arg {
        Some(a) => Cmp::EvalArg(f, c.arg(a)),
        None => Cmp::Eval(f),
    })
}

fn register_queries(b: &crate::expose::ClassBuilder) {
    // getFirst / contains.
    for (name, mk, recurse) in [
        ("getFirstType", p_type as MkPred, false),
        ("getFirstTypeRecurse", p_type, true),
        ("getFirstTypeEval", p_type_eval, true),
        ("getFirstTypeEvalRecurse", p_type_eval, true),
        ("getFirstTypeEvalArgRecurse", p_type_eval_arg, true),
        ("getFirstTag", p_tag, false),
        ("getFirstTagRecurse", p_tag, true),
        ("getFirstTagEval", p_tag_eval, true),
        ("getFirstTagEvalRecurse", p_tag_eval, true),
        ("getFirstTagEvalArgRecurse", p_tag_eval_arg, true),
        ("getFirstCategory", p_category, false),
        ("getFirstCategoryRecurse", p_category, true),
        ("getFirstEval", p_eval, false),
        ("getFirstEvalArg", p_eval_arg, false),
        ("getFirstEvalRecurse", p_eval, true),
        ("getFirstEvalArgRecurse", p_eval_arg, true),
        ("FindAndReturnCategory", p_category, false),
    ] {
        reg_first(b, name, mk, recurse, false);
    }
    for (name, mk, recurse) in [
        ("containsEval", p_eval as MkPred, false),
        ("containsEvalArg", p_eval_arg, false),
        ("containsEvalRecurse", p_eval, true),
        ("containsEvalArgRecurse", p_eval_arg, true),
        ("containsTag", p_tag, false),
        // `containsTagEval` goes through `getFirstTagEval`, which recurses.
        ("containsTagEval", p_tag_eval, true),
        ("containsTagRecurse", p_tag, true),
        ("containsTagEvalRecurse", p_tag_eval, true),
        ("containsTagEvalArgRecurse", p_tag_eval_arg, true),
        ("containsTypeEvalRecurse", p_type_eval, true),
        ("containsTypeEvalArgRecurse", p_type_eval_arg, true),
    ] {
        reg_first(b, name, mk, recurse, true);
    }
    // getCount.
    for (name, mk, recurse) in [
        ("getCountTag", p_tag as MkPred, false),
        ("getCountTagEval", p_tag_eval, false),
        ("getCountTagEvalArg", p_tag_eval_arg, false),
        ("getCountTagRecurse", p_tag, true),
        ("getCountTagEvalRecurse", p_tag_eval, true),
        ("getCountTagEvalArgRecurse", p_tag_eval_arg, true),
        ("getCountType", p_type, false),
        ("getCountTypeEval", p_type_eval, false),
        ("getCountTypeEvalArg", p_type_eval_arg, false),
        ("getCountTypeRecurse", p_type, true),
        ("getCountTypeEvalRecurse", p_type_eval, true),
        ("getCountTypeEvalArgRecurse", p_type_eval_arg, true),
        ("getCountEval", p_eval, false),
        ("getCountEvalArg", p_eval_arg, false),
        ("getCountEvalRecurse", p_eval, true),
        ("getCountEvalArgRecurse", p_eval_arg, true),
    ] {
        reg_count(b, name, mk, recurse);
    }
    // getAll: (predicate args[, result]); `optional` marks the overloads
    // that create the list.
    for (name, mk, recurse, optional) in [
        ("getAllType", p_type as MkPred, false, true),
        ("getAllTypeRecurse", p_type, true, true),
        ("getAllTypeEval", p_type_eval, false, true),
        ("getAllTypeEvalRecurse", p_type_eval, true, true),
        ("getAllTypeEvalArg", p_type_eval_arg, false, true),
        ("getAllTypeEvalArgRecurse", p_type_eval_arg, true, true),
        ("getAllTag", p_tag, false, true),
        ("getAllTagRecurse", p_tag, true, false),
        ("getAllTagEval", p_tag_eval, false, true),
        ("getAllTagEvalRecurse", p_tag_eval, true, false),
        ("getAllTagEvalArg", p_tag_eval_arg, false, true),
        ("getAllTagEvalArgRecurse", p_tag_eval_arg, true, false),
        ("getAllCategory", p_category, false, true),
        ("getAllCategoryRecurse", p_category, true, false),
        ("getAllEval", p_eval, false, true),
        ("getAllEvalRecurse", p_eval, true, true),
        ("getAllEvalArg", p_eval_arg, false, true),
        ("getAllEvalArgRecurse", p_eval_arg, true, true),
    ] {
        reg_all(b, name, mk, recurse, optional);
    }
    // getSome: (predicate args, count[, result]).
    for (name, mk, recurse, optional) in [
        ("getSomeType", p_type as MkPred, false, true),
        ("getSomeTypeRecurse", p_type, true, true),
        ("getSomeTypeEval", p_type_eval, false, true),
        ("getSomeTypeEvalRecurse", p_type_eval, true, true),
        ("getSomeTypeEvalArg", p_type_eval_arg, false, true),
        ("getSomeTypeEvalArgRecurse", p_type_eval_arg, true, true),
        ("getSomeTag", p_tag, false, true),
        ("getSomeTagRecurse", p_tag, true, true),
        ("getSomeTagEval", p_tag_eval, false, false),
        ("getSomeTagEvalRecurse", p_tag_eval, true, true),
        ("getSomeTagEvalArg", p_tag_eval_arg, false, false),
        ("getSomeTagEvalArgRecurse", p_tag_eval_arg, true, true),
        ("getSomeCategory", p_category, false, true),
        ("getSomeCategoryRecurse", p_category, true, false),
        ("getSomeEval", p_eval, false, true),
        ("getSomeEvalRecurse", p_eval, true, true),
        ("getSomeEvalArg", p_eval_arg, false, true),
        ("getSomeEvalArgRecurse", p_eval_arg, true, true),
    ] {
        reg_some(b, name, mk, recurse, optional);
    }
    // getBest (predicate, comparator); `getBest`/`getBestType` take a
    // Java Comparator.
    for (name, recurse) in [("getBestEval", false), ("getBestEvalRecurse", true)] {
        b.method(name, move |c| {
            let s = this(c)?;
            let p = Pred::Eval(fn_arg(c, 1)?);
            let cmp = cmp_eval(c, 2, None)?;
            let r = best(c.thread, &s, &p, &cmp, recurse);
            push_opt(c, r)
        });
    }
    for (name, recurse) in [("getBestEvalArg", false), ("getBestEvalArgRecurse", true)] {
        b.method(name, move |c| {
            let s = this(c)?;
            let p = Pred::EvalArg(fn_arg(c, 1)?, c.arg(3));
            let cmp = cmp_eval(c, 2, Some(3))?;
            let r = best(c.thread, &s, &p, &cmp, recurse);
            push_opt(c, r)
        });
    }
    for (name, recurse) in [("getBestTypeEval", false), ("getBestTypeEvalRecurse", true)] {
        b.method(name, move |c| {
            let s = this(c)?;
            let p = Pred::Type(str_arg(c, 1)?);
            let cmp = cmp_eval(c, 2, None)?;
            let r = best(c.thread, &s, &p, &cmp, recurse);
            push_opt(c, r)
        });
    }
    for (name, recurse) in [
        ("getBestTypeEvalArg", false),
        ("getBestTypeEvalArgRecurse", true),
    ] {
        b.method(name, move |c| {
            let s = this(c)?;
            let p = Pred::Type(str_arg(c, 1)?);
            let cmp = cmp_eval(c, 2, Some(3))?;
            let r = best(c.thread, &s, &p, &cmp, recurse);
            push_opt(c, r)
        });
    }
    // getBestCondition: the String overloads (the Predicate ones take a
    // Java Predicate).
    for (name, mk, recurse) in [
        ("getBestCondition", p_type as MkPred, false),
        ("getBestConditionRecurse", p_type, true),
        ("getBestConditionEval", p_eval, false),
        ("getBestConditionEvalRecurse", p_eval, true),
        ("getBestConditionEvalArg", p_eval_arg, false),
        ("getBestConditionEvalArgRecurse", p_eval_arg, true),
    ] {
        b.method(name, move |c| {
            let s = this(c)?;
            if (name == "getBestCondition" || name == "getBestConditionRecurse")
                && !matches!(c.arg(1), Value::Str(_) | Value::Num(_))
            {
                return Err(class_cast(&c.arg(1), "java.lang.String"));
            }
            let (p, _) = mk(c)?;
            let r = best_condition(c.thread, &s, &p, recurse)?;
            Ok(c.push(opt_item(r)))
        });
    }
    // getUses.
    b.method("getUsesType", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let f = collect(c.thread, &s, &p, false, None)?;
        Ok(c.push(uses(&f) as f64))
    });
    b.method("getUsesTypeRecurse", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let f = collect(c.thread, &s, &p, true, None)?;
        Ok(c.push(uses(&f) as f64))
    });
    // Aliases.
    b.method("getItemCount", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let recurse = c.nargs() > 2 && bool_arg(c, 2);
        let n = collect(c.thread, &s, &p, recurse, None)?.len();
        Ok(c.push(n as f64))
    });
    b.method("getItemCountRecurse", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let n = collect(c.thread, &s, &p, true, None)?.len();
        Ok(c.push(n as f64))
    });
    b.method("getItemFromTypeRecurse", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let r = first(c.thread, &s, &p, true);
        push_opt(c, r)
    });
    b.method("containsType", |c| {
        let s = this(c)?;
        let ty = str_arg(c, 1)?;
        let r = find_item(&s, &ty, false, false)?.is_some();
        Ok(c.push(r))
    });
    b.method("containsTypeRecurse", |c| {
        let s = this(c)?;
        let ty = str_arg(c, 1)?;
        let r = find_item(&s, &ty, true, false)?.is_some();
        Ok(c.push(r))
    });
    b.method("FindAndReturn", |c| {
        let s = this(c)?;
        match c.arg(2) {
            Value::Nil if c.nargs() <= 2 => {
                let p = Pred::Type(str_arg(c, 1)?);
                let r = first(c.thread, &s, &p, false);
                push_opt(c, r)
            }
            Value::Num(_) => {
                let p = Pred::Type(str_arg(c, 1)?);
                let count = int_arg(c, 2);
                fill_result(c, 99, true, false, |t, _| {
                    collect(t, &s, &p, false, Some((0, count)))
                })
            }
            _ => {
                // FindAndReturn(type, itemToCheck).
                let Some(ty) = opt_str_arg(c, 1) else {
                    return Ok(c.push(Value::Nil));
                };
                let check = list_arg(c, 2)?.ok_or_else(npe)?;
                let skip = as_list(&check).snapshot();
                let found = items(&s).into_iter().find(|r| {
                    compare_type_item(&ty, &r.borrow())
                        && !skip.iter().any(|v| v.ptr_id() == item_value(r).ptr_id())
                });
                Ok(c.push(opt_item(found)))
            }
        }
    });
    b.method("FindAll", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        fill_result(c, 99, true, false, |t, _| collect(t, &s, &p, false, None))
    });
    b.method("getItemsFromType", |c| {
        let s = this(c)?;
        let p = Pred::Type(str_arg(c, 1)?);
        let recurse = c.nargs() > 2 && bool_arg(c, 2);
        fill_result(c, 99, true, false, |t, _| collect(t, &s, &p, recurse, None))
    });
    b.method("getItemsFromFullType", |c| {
        let s = this(c)?;
        let ty = opt_str_arg(c, 1);
        let Some(ty) = ty.filter(|t| t.contains('.')) else {
            return Ok(c.push(list_value(Vec::new())));
        };
        let recurse = c.nargs() > 2 && bool_arg(c, 2);
        let p = Pred::Type(ty);
        fill_result(c, 99, true, false, |t, _| collect(t, &s, &p, recurse, None))
    });
    b.method("getItemsFromCategory", |c| {
        let s = this(c)?;
        let p = Pred::Category(str_arg(c, 1)?);
        fill_result(c, 99, true, false, |t, _| collect(t, &s, &p, false, None))
    });
}

fn register_items(b: &crate::expose::ClassBuilder) {
    b.method("getItems", |c| {
        let v = c.arg(0);
        let Value::Object(o) = &v else {
            return Err(npe());
        };
        let Some(l) = o.as_any().downcast_ref::<LuaItemContainer>() else {
            return Err(npe());
        };
        let mut cached = l.items_list.borrow_mut();
        if cached.is_none() {
            let (r, w) = (l.container.clone(), l.container.clone());
            let list = JavaList::live(
                move || r.borrow().items.iter().map(item_value).collect(),
                Some(Box::new(move |vals: Vec<Value>| {
                    let mut items = Vec::with_capacity(vals.len());
                    for v in &vals {
                        match v {
                            Value::Object(o) => match o.as_any().downcast_ref::<LuaInventoryItem>()
                            {
                                Some(i) => items.push(i.item.clone()),
                                None => {
                                    return Err(class_cast(v, "zombie.inventory.InventoryItem"))
                                }
                            },
                            _ => return Err(class_cast(v, "zombie.inventory.InventoryItem")),
                        }
                    }
                    w.borrow_mut().items = items;
                    Ok(())
                })),
            );
            *cached = Some(Value::Object(Rc::new(list)));
        }
        let v = cached.clone().unwrap();
        drop(cached);
        Ok(c.push(v))
    });
    b.method("setItems", |c| {
        let s = this(c)?;
        let vals = match list_arg(c, 1)? {
            Some(l) => as_list(&l).snapshot(),
            None => return Err(npe()),
        };
        let mut items = Vec::new();
        for v in &vals {
            match v {
                Value::Object(o) if o.as_any().is::<LuaInventoryItem>() => items.push(
                    o.as_any()
                        .downcast_ref::<LuaInventoryItem>()
                        .unwrap()
                        .item
                        .clone(),
                ),
                _ => return Err(class_cast(v, "zombie.inventory.InventoryItem")),
            }
        }
        s.borrow_mut().items = items;
        Ok(0)
    });
    b.method("emptyIt", |c| {
        this(c)?.borrow_mut().items = Vec::new();
        Ok(0)
    });
    b.method("clear", |c| {
        let s = this(c)?;
        let mut s = s.borrow_mut();
        s.items.clear();
        s.dirty = true;
        s.draw_dirty = true;
        Ok(0)
    });
    b.method("removeAllItems", |c| {
        let s = this(c)?;
        let p = parent(&s);
        {
            let mut m = s.borrow_mut();
            m.draw_dirty = true;
            if !p.is_nil() {
                m.dirty = true;
            }
        }
        for i in items(&s) {
            set_container(&i, None);
        }
        s.borrow_mut().items.clear();
        after_remove(c.thread, &s, Value::Nil)?;
        Ok(0)
    });
    b.method("takeItemsFrom", |c| {
        let s = this(c)?;
        let other = container_arg(c, 1)?;
        loop {
            let Some(item) = other.borrow().items.first().cloned() else {
                break;
            };
            remove(c.thread, &other, &item)?;
            add_item(c.thread, &s, Some(item))?;
        }
        Ok(0)
    });
    let add = |c: &mut NativeCall<'_>| -> LuaResult<usize> {
        let s = this(c)?;
        let a = c.arg(1);
        if is_item(&a) || a.is_nil() {
            let r = add_item(c.thread, &s, opt_item_arg(c, 1)?)?;
            return Ok(c.push(opt_item(r)));
        }
        let ty = str_arg(c, 1)?;
        if c.nargs() >= 3 {
            let delta = num_arg(c, 2) as f32;
            let added = add_item_uses(c.thread, &s, &ty, delta)?;
            return Ok(c.push(added));
        }
        let r = add_item_type(c.thread, &s, &ty)?;
        Ok(c.push(opt_item(r)))
    };
    b.method("AddItem", add);
    b.method("addItem", add);
    b.method("DoAddItem", |c| {
        let s = this(c)?;
        let r = add_item(c.thread, &s, opt_item_arg(c, 1)?)?;
        Ok(c.push(opt_item(r)))
    });
    b.method("DoAddItemBlind", |c| {
        let s = this(c)?;
        let r = add_item(c.thread, &s, opt_item_arg(c, 1)?)?;
        Ok(c.push(opt_item(r)))
    });
    b.method("AddItemBlind", |c| {
        let s = this(c)?;
        let Some(item) = opt_item_arg(c, 1)? else {
            return Ok(c.push(Value::Nil));
        };
        let e = item_env();
        let w = item.borrow().get_weight_with(e.context());
        if w + capacity_weight(c.thread, &s)? > capacity(&s) as f32 {
            return Ok(c.push(Value::Nil));
        }
        // The item's `container` is left as it was.
        s.borrow_mut().items.push(item.clone());
        Ok(c.push(item_value(&item)))
    });
    b.method("SpawnItem", |c| {
        let s = this(c)?;
        let a = c.arg(1);
        if is_item(&a) || a.is_nil() {
            // SpawnItem(InventoryItem): adds only a null item, then
            // `item.SynchSpawn()` (offline nothing) throws on null.
            if a.is_nil() {
                return Err(npe());
            }
            return Ok(0);
        }
        let ty = str_arg(c, 1)?;
        if c.nargs() >= 3 {
            let delta = num_arg(c, 2) as f32;
            let added = add_item_uses(c.thread, &s, &ty, delta)?;
            return Ok(c.push(added));
        }
        let r = add_item_type(c.thread, &s, &ty)?;
        Ok(c.push(opt_item(r)))
    });
    b.method("AddItems", |c| {
        let s = this(c)?;
        let a = c.arg(1);
        if c.nargs() <= 2 && !is_item(&a) && !matches!(a, Value::Str(_) | Value::Num(_)) {
            // AddItems(ArrayList<InventoryItem>).
            let l = list_arg(c, 1)?.ok_or_else(npe)?;
            for v in as_list(&l).snapshot() {
                let item = match &v {
                    Value::Object(o) => o
                        .as_any()
                        .downcast_ref::<LuaInventoryItem>()
                        .map(|i| i.item.clone()),
                    _ => None,
                }
                .ok_or_else(|| class_cast(&v, "zombie.inventory.InventoryItem"))?;
                add_item(c.thread, &s, Some(item))?;
            }
            return Ok(c.push(Value::Object(l)));
        }
        let ty = match opt_item_arg(c, 1) {
            Ok(Some(i)) => i.borrow().get_full_type(),
            _ => str_arg(c, 1)?,
        };
        let count = int_arg(c, 2);
        let mut out = Vec::new();
        for _ in 0..count {
            if let Some(i) = add_item_type(c.thread, &s, &ty)? {
                out.push(i);
            }
        }
        Ok(c.push(item_list(out)))
    });
    b.method("addItems", |c| {
        let s = this(c)?;
        let ty = str_arg(c, 1)?;
        let count = int_arg(c, 2);
        let mut out = Vec::new();
        for _ in 0..count {
            if let Some(i) = add_item_type(c.thread, &s, &ty)? {
                out.push(i);
            }
        }
        Ok(c.push(item_list(out)))
    });
    b.method("Remove", |c| {
        let s = this(c)?;
        let a = c.arg(1);
        if let Some(item) = opt_item_arg(c, 1).ok().flatten() {
            remove(c.thread, &s, &item)?;
            return Ok(0);
        }
        if let Value::Object(_) = a {
            return Err(LuaError::runtime(
                "ItemContainer.Remove(ItemType) is not ported",
            ));
        }
        // Remove(String itemTypes).
        let ty = str_arg(c, 1)?;
        for r in items(&s) {
            if r.borrow().get_type() != ty {
                continue;
            }
            let uses = r.borrow().get_current_uses();
            if uses > 1 {
                let e = item_env();
                r.borrow_mut()
                    .set_current_uses_with(uses - 1, Some(e.context()));
            } else {
                let mut m = s.borrow_mut();
                if let Some(i) = m.index_of(&r) {
                    m.items.remove(i);
                }
            }
            set_container(&r, None);
            let mut m = s.borrow_mut();
            m.draw_dirty = true;
            m.dirty = true;
            break;
        }
        Ok(0)
    });
    b.method("DoRemoveItem", |c| {
        let s = this(c)?;
        let item = opt_item_arg(c, 1)?.ok_or_else(npe)?;
        do_remove_item(c.thread, &s, &item)?;
        Ok(0)
    });
    b.method("RemoveAll", |c| {
        let s = this(c)?;
        let ty = opt_str_arg(c, 1);
        let count = if c.nargs() > 2 {
            int_arg(c, 2)
        } else {
            s.borrow().items.len() as i32
        };
        let p = parent(&s);
        {
            let mut m = s.borrow_mut();
            m.draw_dirty = true;
            if !p.is_nil() {
                m.dirty = true;
            }
        }
        let mut out = Vec::new();
        for r in items(&s) {
            let matches = {
                let i = r.borrow();
                ty.as_deref()
                    .is_some_and(|t| i.get_type() == t || i.get_full_type() == t)
            };
            if !matches {
                continue;
            }
            set_container(&r, None);
            out.push(r);
            s.borrow_mut().dirty = true;
            if out.len() as i64 >= count as i64 {
                break;
            }
        }
        for r in &out {
            let mut m = s.borrow_mut();
            if let Some(i) = m.index_of(r) {
                m.items.remove(i);
            }
        }
        Ok(c.push(item_list(out)))
    });
    b.method("RemoveOneOf", |c| {
        let s = this(c)?;
        let ty = str_arg(c, 1)?;
        let inside = c.nargs() <= 2 || bool_arg(c, 2);
        let r = remove_one_of(&s, &ty, inside);
        Ok(if c.nargs() > 2 {
            c.push(opt_item(r))
        } else {
            0
        })
    });
    b.method("removeItemWithID", |c| {
        let s = this(c)?;
        let id = int_arg(c, 1);
        let found = items(&s).into_iter().find(|i| i.borrow().id == id);
        if let Some(i) = &found {
            remove(c.thread, &s, i)?;
        }
        Ok(c.push(found.is_some()))
    });
    b.method("removeItemWithIDRecurse", |c| {
        let s = this(c)?;
        let id = int_arg(c, 1);
        let r = remove_with_id_recurse(c.thread, &s, id)?;
        Ok(c.push(r))
    });
    b.method("removeItemOnServer", |_| Ok(0));
    b.method("contains", |c| {
        let s = this(c)?;
        let a = c.arg(1);
        if is_item(&a) {
            let item = opt_item_arg(c, 1)?.unwrap();
            let r = contains_in(c.thread, &s, &item, c.nargs() > 2 && bool_arg(c, 2))?;
            return Ok(c.push(r));
        }
        let ty = str_arg(c, 1)?;
        let do_inv = c.nargs() > 2 && bool_arg(c, 2);
        let ignore_broken = c.nargs() > 3 && bool_arg(c, 3);
        let r = find_item(&s, &ty, do_inv, ignore_broken)?.is_some();
        Ok(c.push(r))
    });
    b.method("containsWithModule", |c| {
        let s = this(c)?;
        let mt = str_arg(c, 1)?;
        let with_delta = c.nargs() > 2 && bool_arg(c, 2);
        let (module, ty) = match mt.split_once('.') {
            Some((m, t)) => (m.to_string(), t.split('.').next().unwrap_or("").to_string()),
            None => ("Base".to_string(), mt.clone()),
        };
        let ty = ty.trim().to_string();
        let r = items(&s).iter().any(|r| {
            let i = r.borrow();
            i.get_type() == ty
                && i.get_module() == module
                && !(with_delta && is_drainable(&i) && i.get_current_uses() <= 0)
        });
        Ok(c.push(r))
    });
    b.method("containsID", |c| {
        let s = this(c)?;
        let id = int_arg(c, 1);
        let r = items(&s).iter().any(|i| i.borrow().id == id);
        Ok(c.push(r))
    });
    b.method("containsRecursive", |c| {
        let s = this(c)?;
        let item = opt_item_arg(c, 1)?;
        let r = item.is_some_and(|i| contains_recursive(&s, &i));
        Ok(c.push(r))
    });
    b.method("containsHumanCorpse", |c| {
        let s = this(c)?;
        let r = find_by(&s, &is_human_corpse, false).is_some();
        Ok(c.push(r))
    });
    b.method("findHumanCorpseItem", |c| {
        let s = this(c)?;
        let r = find_by(&s, &is_human_corpse, false);
        push_opt(c, Ok(r))
    });
    b.method("findItem", |c| {
        let s = this(c)?;
        if !matches!(c.arg(1), Value::Str(_) | Value::Num(_)) {
            return Err(LuaError::runtime(
                "ItemContainer.findItem(predicate, doInv) takes a Java callback",
            ));
        }
        let ty = str_arg(c, 1)?;
        let r = find_item(&s, &ty, bool_arg(c, 2), bool_arg(c, 3));
        push_opt(c, r)
    });
    b.method("Find", |c| {
        let s = this(c)?;
        if let Value::Object(_) = c.arg(1) {
            return Err(LuaError::runtime(
                "ItemContainer.Find(ItemType) is not ported",
            ));
        }
        let ty = opt_str_arg(c, 1);
        let r = items(&s)
            .into_iter()
            .find(|i| ty.as_deref().is_some_and(|t| i.borrow().get_type() == t));
        Ok(c.push(opt_item(r)))
    });
    b.method("getItemWithID", |c| {
        let s = this(c)?;
        let id = int_arg(c, 1);
        let r = items(&s).into_iter().find(|i| i.borrow().id == id);
        Ok(c.push(opt_item(r)))
    });
    let with_id_recursive = |c: &mut NativeCall<'_>| -> LuaResult<usize> {
        let s = this(c)?;
        let id = num_arg(c, 1) as i64;
        let r = item_with_id_recursive(&s, id);
        Ok(c.push(opt_item(r)))
    };
    b.method("getItemWithIDRecursiv", with_id_recursive);
    b.method("getItemById", with_id_recursive);
    b.method("haveThisKeyId", |c| {
        let s = this(c)?;
        let key_id = int_arg(c, 1);
        let r = have_key_id(&s, key_id)?;
        Ok(c.push(opt_item(r)))
    });
    b.method("getNumberOfItem", |c| {
        let s = this(c)?;
        let find = str_arg(c, 1)?;
        let rod = c.nargs() > 2 && bool_arg(c, 2);
        let n = match c.arg(3) {
            Value::Bool(inside) => number_of_item(&s, &find, rod, inside),
            Value::Object(o) if o.as_any().is::<JavaList>() => {
                let mut n = number_of_item(&s, &find, rod, false);
                for v in as_list(&o).snapshot() {
                    let other = container_of_value(&v)
                        .ok_or_else(|| class_cast(&v, ITEM_CONTAINER_CLASS))?;
                    if !Rc::ptr_eq(&other, &s) {
                        n += number_of_item(&other, &find, rod, false);
                    }
                }
                n
            }
            _ => number_of_item(&s, &find, rod, false),
        };
        Ok(c.push(n as f64))
    });
    b.method("getNumItems", |c| {
        let s = this(c)?;
        let like = str_arg(c, 1)?;
        let mut n = 0;
        for r in items(&s) {
            let i = r.borrow();
            if like.contains("Type:") {
                if matches!(i.kind, ItemKind::Food(_)) && like.contains("Food") {
                    n += i.get_current_uses();
                }
                if matches!(i.kind, ItemKind::Weapon(_)) && like.contains("Weapon") {
                    n += i.get_current_uses();
                }
            } else if i.get_type() == like {
                n += i.get_current_uses();
            }
        }
        Ok(c.push(n as f64))
    });
    b.method("getItemCountFromTypeRecurse", |c| {
        let s = this(c)?;
        let ty = opt_str_arg(c, 1);
        let n = count_full_type_recurse(&s, ty.as_deref());
        Ok(c.push(n as f64))
    });
    b.method("getItemFromType", |c| {
        let s = this(c)?;
        let mut ty = str_arg(c, 1)?;
        if c.nargs() <= 2 {
            let p = Pred::Type(ty);
            let r = first(c.thread, &s, &p, false);
            return push_opt(c, r);
        }
        let (chr, not_eq, ignore, inv) = if c.nargs() >= 6 {
            (c.arg(2), bool_arg(c, 3), bool_arg(c, 4), bool_arg(c, 5))
        } else {
            (Value::Nil, false, bool_arg(c, 2), bool_arg(c, 3))
        };
        if ty.contains('.') {
            ty = ty.split('.').nth(1).unwrap_or("").to_string();
        }
        let test = move |i: &InventoryItem| i.get_full_type() == ty || i.get_type() == ty;
        let r = item_from(c.thread, &s, &test, &chr, not_eq, ignore, inv);
        push_opt(c, r)
    });
    b.method("getItemFromTag", |c| {
        let s = this(c)?;
        let tag = str_arg(c, 1)?;
        let (chr, not_eq, ignore, inv) = if c.nargs() >= 6 {
            (c.arg(2), bool_arg(c, 3), bool_arg(c, 4), bool_arg(c, 5))
        } else {
            (Value::Nil, false, bool_arg(c, 2), bool_arg(c, 3))
        };
        let test = move |i: &InventoryItem| i.has_tag(&tag);
        let r = item_from(c.thread, &s, &test, &chr, not_eq, ignore, inv);
        push_opt(c, r)
    });
    b.method("getBestFood", |c| {
        let s = this(c)?;
        let mut best = None;
        let mut best_score = 0.0f32;
        for r in items(&s) {
            let i = r.borrow();
            if !matches!(i.kind, ItemKind::Food(_)) {
                continue;
            }
            // Food.getScore(desc).
            let mut score = 0.0f32 - i.get_hunger_change() * 100.0f32;
            if i.is_b_dangerous_uncooked() && !i.is_cooked() {
                score *= 0.2f32;
            }
            if i.get_age() > i.get_off_age() as f32 {
                score *= 0.2f32;
            }
            if score >= best_score {
                best_score = score;
                drop(i);
                best = Some(r);
            }
        }
        Ok(c.push(opt_item(best)))
    });
    b.method("getTotalFoodScore", |c| {
        let s = this(c)?;
        let mut score = 0.0f32;
        for r in items(&s) {
            let i = r.borrow();
            if matches!(i.kind, ItemKind::Food(_)) {
                score += 0.0f32 - i.get_hunger_change() * 100.0f32;
            }
        }
        Ok(c.push(score as f64))
    });
    b.method("getBestBandage", |c| {
        let s = this(c)?;
        let sm = crate::bind_registry::shared_script_manager();
        let r = items(&s).into_iter().find(|r| {
            pz_characters::character::items::script_item(&sm, &r.borrow())
                .is_some_and(|s| s.borrow().can_bandage)
        });
        Ok(c.push(opt_item(r)))
    });
}

fn is_human_corpse(i: &InventoryItem) -> bool {
    let t = i.get_type();
    t.eq_ignore_ascii_case("CorpseFemale") || t.eq_ignore_ascii_case("CorpseMale")
}

macro_rules! field {
    ($b:expr, $get:literal, $set:literal, $f:ident, bool) => {
        $b.method($get, |c| {
            let v = this(c)?.borrow().$f;
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            let v = bool_arg(c, 1);
            this(c)?.borrow_mut().$f = v;
            Ok(0)
        });
    };
    ($b:expr, $get:literal, $set:literal, $f:ident, f32) => {
        $b.method($get, |c| {
            let v = this(c)?.borrow().$f;
            Ok(c.push(v as f64))
        });
        $b.method($set, |c| {
            let v = num_arg(c, 1) as f32;
            this(c)?.borrow_mut().$f = v;
            Ok(0)
        });
    };
    ($b:expr, $get:literal, $set:literal, $f:ident, str) => {
        $b.method($get, |c| {
            let v = this(c)?.borrow().$f.clone();
            Ok(c.push(v.map_or(Value::Nil, Value::from)))
        });
        $b.method($set, |c| {
            let v = opt_str_arg(c, 1);
            this(c)?.borrow_mut().$f = v;
            Ok(0)
        });
    };
}

/// `getOpenSound` & co.: the set sound, else (no vehicle part) the
/// parent sprite's property.
fn sound(c: &mut NativeCall<'_>, set: Option<String>, prop: &str) -> LuaResult<Value> {
    if let Some(s) = set {
        return Ok(Value::from(s));
    }
    let s = this(c)?;
    let p = parent(&s);
    if p.is_nil() || call_or(c.thread, &p, "getSprite", &[], Value::Nil)?.is_nil() {
        return Ok(Value::Nil);
    }
    let props = call_or(c.thread, &p, "getProperties", &[], Value::Nil)?;
    let k = static_field(c.thread, "IsoPropertyType", prop);
    call(c.thread, &props, "get", &[k])
}

fn register_fields(b: &crate::expose::ClassBuilder) {
    b.method("getType", |c| {
        let v = this(c)?.borrow().kind.clone();
        Ok(c.push(v))
    });
    b.method("setType", |c| {
        let v = opt_str_arg(c, 1).unwrap_or_default();
        this(c)?.borrow_mut().kind = v;
        Ok(0)
    });
    b.method("getCapacity", |c| {
        let v = capacity(&this(c)?);
        Ok(c.push(v as f64))
    });
    b.method("setCapacity", |c| {
        let s = this(c)?;
        let cap = int_arg(c, 1);
        set_capacity(&s, cap, &c.arg(0));
        Ok(0)
    });
    field!(b, "isExplored", "setExplored", explored, bool);
    field!(b, "isDirty", "setDirty", dirty, bool);
    field!(b, "isDrawDirty", "setDrawDirty", draw_dirty, bool);
    field!(b, "isActive", "setActive", active, bool);
    field!(b, "isIsDevice", "setIsDevice", is_device, bool);
    field!(
        b,
        "isHasBeenLooted",
        "setHasBeenLooted",
        has_been_looted,
        bool
    );
    field!(b, "getAgeFactor", "setAgeFactor", age_factor, f32);
    field!(
        b,
        "getCookingFactor",
        "setCookingFactor",
        cooking_factor,
        f32
    );
    field!(
        b,
        "getCustomTemperature",
        "setCustomTemperature",
        custom_temperature,
        f32
    );
    field!(
        b,
        "getContainerPosition",
        "setContainerPosition",
        container_position,
        str
    );
    field!(
        b,
        "getFreezerPosition",
        "setFreezerPosition",
        freezer_position,
        str
    );
    b.method("getWeightReduction", |c| {
        let v = this(c)?.borrow().weight_reduction;
        Ok(c.push(v as f64))
    });
    b.method("setWeightReduction", |c| {
        let v = int_arg(c, 1).clamp(0, 100);
        this(c)?.borrow_mut().weight_reduction = v;
        Ok(0)
    });
    for (get, set) in [
        ("getOnlyAcceptCategory", "setOnlyAcceptCategory"),
        ("getAcceptItemFunction", "setAcceptItemFunction"),
    ] {
        b.method(get, move |c| {
            let s = this(c)?;
            let s = s.borrow();
            let v = if get == "getOnlyAcceptCategory" {
                s.only_accept_category.clone()
            } else {
                s.accept_item_function.clone()
            };
            Ok(c.push(v.map_or(Value::Nil, Value::from)))
        });
        b.method(set, move |c| {
            // StringUtils.discardNullOrWhitespace.
            let v = opt_str_arg(c, 1).filter(|s| !s.trim().is_empty());
            let s = this(c)?;
            let mut s = s.borrow_mut();
            if set == "setOnlyAcceptCategory" {
                s.only_accept_category = v;
            } else {
                s.accept_item_function = v;
            }
            Ok(0)
        });
    }
    for (get, set, prop) in [
        ("getOpenSound", "setOpenSound", "CONTAINER_OPEN_SOUND"),
        ("getCloseSound", "setCloseSound", "CONTAINER_CLOSE_SOUND"),
        ("getPutSound", "setPutSound", "CONTAINER_PUT_SOUND"),
        ("getTakeSound", "setTakeSound", "CONTAINER_TAKE_SOUND"),
    ] {
        b.method(get, move |c| {
            let s = this(c)?;
            let v = {
                let s = s.borrow();
                match get {
                    "getOpenSound" => s.open_sound.clone(),
                    "getCloseSound" => s.close_sound.clone(),
                    "getPutSound" => s.put_sound.clone(),
                    _ => s.take_sound.clone(),
                }
            };
            let v = sound(c, v, prop)?;
            Ok(c.push(v))
        });
        b.method(set, move |c| {
            let v = opt_str_arg(c, 1);
            let s = this(c)?;
            let mut s = s.borrow_mut();
            match set {
                "setOpenSound" => s.open_sound = v,
                "setCloseSound" => s.close_sound = v,
                "setPutSound" => s.put_sound = v,
                _ => s.take_sound = v,
            }
            Ok(0)
        });
    }
    b.method("isEmpty", |c| {
        let v = this(c)?.borrow().items.is_empty();
        Ok(c.push(v))
    });
    b.method("isFreezer", |c| {
        let v = this(c)?.borrow().is_freezer();
        Ok(c.push(v))
    });
    b.method("isFridge", |c| {
        let s = this(c)?;
        let v = is_fridge(c.thread, &s)?;
        Ok(c.push(v))
    });
    b.method("isStove", |c| {
        let v = is_stove(&this(c)?.borrow());
        Ok(c.push(v))
    });
    b.method("isMicrowave", |c| {
        let v = this(c)?.borrow().kind == "microwave";
        Ok(c.push(v))
    });
    b.method("isCorpse", |c| {
        let s = this(c)?;
        let v = matches!(
            s.borrow().kind.as_str(),
            "inventorymale" | "inventoryfemale"
        );
        Ok(c.push(v))
    });
    b.method("isPowered", |c| {
        let s = this(c)?;
        let v = is_powered(c.thread, &s)?;
        Ok(c.push(v))
    });
    let temp = |c: &mut NativeCall<'_>| -> LuaResult<usize> {
        let s = this(c)?;
        let v = temperature(c.thread, &s)?;
        Ok(c.push(v as f64))
    };
    b.method("getTemperature", temp);
    b.method("getTemprature", temp);
    b.method("isTemperatureChanging", |c| {
        let s = this(c)?;
        let p = parent(&s);
        if instance_of(&p, ISO_BARBECUE)
            || instance_of(&p, ISO_FIREPLACE)
            || instance_of(&p, ISO_STOVE)
        {
            let v = call(c.thread, &p, "isTemperatureChanging", &[])?;
            return Ok(c.push(v.truthy()));
        }
        let v = is_fridge_or_freezer_warming(c.thread, &s)?;
        Ok(c.push(v))
    });
    b.method("isFridgeOrFreezerWarming", |c| {
        let s = this(c)?;
        let v = is_fridge_or_freezer_warming(c.thread, &s)?;
        Ok(c.push(v))
    });
    // Vehicle part containers are not shared objects yet: `vehiclePart`
    // is always null.
    for name in [
        "getVehiclePart",
        "getVehiclePartOwner",
        "getVehicle",
        "getVehicleDoorPart",
        "getVehicleSeatDoorPart",
        "getVehicleSeatDoor",
        "getVehicleDoor",
    ] {
        b.method(name, |c| {
            this(c)?;
            Ok(c.push(Value::Nil))
        });
    }
    for name in [
        "isVehiclePart",
        "isVehicleSeat",
        "isOccupiedVehicleSeat",
        "doesVehicleDoorNeedOpening",
        "canCharacterOpenVehicleDoor",
        "canCharacterUnlockVehicleDoor",
    ] {
        b.method(name, |c| {
            this(c)?;
            Ok(c.push(false))
        });
    }
    // Offline: no packets.
    for name in ["requestSync", "requestServerItemsForContainer"] {
        b.method(name, |c| {
            this(c)?;
            Ok(0)
        });
    }
    b.method("isExistYet", |c| {
        // `SystemDisabler.doWorldSyncEnable` is false.
        this(c)?;
        Ok(c.push(true))
    });
    b.method("toString", |c| {
        let s = java_string(&c.arg(0));
        Ok(c.push(s))
    });
    b.method("equals", |c| {
        let a = this(c)?;
        let r = container_of_value(&c.arg(1)).is_some_and(|b| Rc::ptr_eq(&a, &b));
        Ok(c.push(r))
    });
    b.method("hashCode", |c| {
        let a = this(c)?;
        let p = Rc::as_ptr(&a) as usize as u64;
        Ok(c.push((p ^ (p >> 32)) as i32 as f64))
    });
}

fn register_parent(b: &crate::expose::ClassBuilder) {
    b.method("getParent", |c| {
        let v = parent(&this(c)?);
        Ok(c.push(v))
    });
    b.method("setParent", |c| {
        let s = this(c)?;
        set_parent(&s, c.arg(1));
        Ok(0)
    });
    b.method("getSourceGrid", |c| {
        let v = lua_ref(&this(c)?.borrow().source_grid);
        Ok(c.push(v))
    });
    b.method("setSourceGrid", |c| {
        let v = c.arg(1);
        this(c)?.borrow_mut().source_grid =
            (!v.is_nil()).then(|| Rc::new(LuaRef(v)) as Rc<dyn Any>);
        Ok(0)
    });
    b.method("getCharacter", |c| {
        let v = character(&this(c)?);
        Ok(c.push(v))
    });
    b.method("getContainingItem", |c| {
        let v = containing_item(&this(c)?);
        Ok(c.push(opt_item(v)))
    });
    b.method("getOutermostContainer", |c| {
        let mut s = this(c)?;
        while let Some(outer) = containing_item(&s).and_then(|b| container_of(&b)) {
            s = outer;
        }
        Ok(c.push(container_value(&s)))
    });
    // A bag's world item is not shared (no `getWorldItem` on items yet).
    b.method("getWorldItem", |c| {
        this(c)?;
        Ok(c.push(Value::Nil))
    });
    b.method("hasWorldItem", |c| {
        this(c)?;
        Ok(c.push(false))
    });
    b.method("getSquare", |c| {
        let mut s = this(c)?;
        while let Some(outer) = containing_item(&s).and_then(|b| container_of(&b)) {
            s = outer;
        }
        let grid = lua_ref(&s.borrow().source_grid);
        if !grid.is_nil() {
            return Ok(c.push(grid));
        }
        let p = parent(&s);
        if !p.is_nil() {
            let sq = call(c.thread, &p, "getSquare", &[])?;
            if !sq.is_nil() {
                return Ok(c.push(sq));
            }
        }
        Ok(c.push(Value::Nil))
    });
    b.method("getDisplayType", |c| {
        let s = this(c)?;
        if let Some(bag) = containing_item(&s) {
            let n = bag.borrow().get_name().to_string();
            return Ok(c.push(n));
        }
        let p = parent(&s);
        if instance_of(&p, ISO_WORLD_INVENTORY_OBJECT) {
            let item = call(c.thread, &p, "getItem", &[])?;
            let n = call(c.thread, &item, "getName", &[])?;
            return Ok(c.push(n));
        }
        if p.is_nil() {
            return Err(npe());
        }
        let sprite = call_or(c.thread, &p, "getSprite", &[], Value::Nil)?;
        if !sprite.is_nil() {
            let k = static_field(c.thread, "IsoPropertyType", "CONTAINER");
            let sp = call(c.thread, &sprite, "getProperties", &[])?;
            if !call(c.thread, &sp, "get", std::slice::from_ref(&k))?.is_nil() {
                let props = call_or(c.thread, &p, "getProperties", &[], Value::Nil)?;
                let v = call(c.thread, &props, "get", &[k])?;
                return Ok(c.push(v));
            }
        }
        let v = s.borrow().kind.clone();
        Ok(c.push(v))
    });
    b.method("getCustomName", |c| {
        let s = this(c)?;
        let key = format!("{}_customContainerName", s.borrow().kind);
        let p = parent(&s);
        if p.is_nil() {
            return Err(npe());
        }
        let md = call(c.thread, &p, "getModData", &[])?;
        let v = md.as_table().map_or(Value::Nil, |t| t.get_str(&key));
        Ok(c.push(v))
    });
    b.method("setCustomName", |c| {
        let s = this(c)?;
        let key = format!("{}_customContainerName", s.borrow().kind);
        let p = parent(&s);
        if p.is_nil() {
            return Err(npe());
        }
        let name = java_string(&c.arg(1));
        let md = call(c.thread, &p, "getModData", &[])?;
        match md.as_table() {
            Some(t) => t.set_str(&key, Value::from(name)),
            None => return Err(npe()),
        }
        // Offline: no `ReceiveContainerModData` packet.
        Ok(0)
    });
    b.method("isLockedToCharacter", |c| {
        let s = this(c)?;
        let p = parent(&s);
        if instance_of(&p, ISO_THUMPABLE) {
            let v = call(c.thread, &p, "isLockedToCharacter", &[c.arg(1)])?;
            return Ok(c.push(v.truthy()));
        }
        Ok(c.push(false))
    });
    b.method("dumpContentsInSquare", |c| {
        // The original removes while iterating forward, so every other
        // item stays.
        let s = this(c)?;
        let sq = c.arg(1);
        let mut i = 0;
        loop {
            let Some(item) = s.borrow().items.get(i).cloned() else {
                break;
            };
            {
                let mut m = s.borrow_mut();
                if let Some(at) = m.index_of(&item) {
                    m.items.remove(at);
                }
            }
            if sq.is_nil() {
                return Err(npe());
            }
            call(
                c.thread,
                &sq,
                "AddWorldInventoryItem",
                &[
                    item_value(&item),
                    Value::Num(0.0),
                    Value::Num(0.0),
                    Value::Num(0.0),
                ],
            )?;
            i += 1;
        }
        Ok(0)
    });
}

fn register_weights(b: &crate::expose::ClassBuilder) {
    b.method("getContentsWeight", |c| {
        let v = contents_weight(&this(c)?);
        Ok(c.push(v as f64))
    });
    b.method("getCapacityWeight", |c| {
        let s = this(c)?;
        let v = capacity_weight(c.thread, &s)?;
        Ok(c.push(v as f64))
    });
    b.method("getMaxWeight", |c| {
        let s = this(c)?;
        let v = max_weight(c.thread, &s)?;
        Ok(c.push(v as f64))
    });
    b.method("getAvailableWeightCapacity", |c| {
        let s = this(c)?;
        let occupied = capacity_weight(c.thread, &s)?;
        let max = max_weight(c.thread, &s)?;
        Ok(c.push((max - occupied) as f64))
    });
    b.method("getEffectiveCapacity", |c| {
        let s = this(c)?;
        let v = effective_capacity(c.thread, &s, &c.arg(1))?;
        Ok(c.push(v as f64))
    });
    b.method("getFreeCapacity", |c| {
        let s = this(c)?;
        let chr = c.arg(1);
        let v = free_capacity(c.thread, &s, &chr)?;
        Ok(c.push(v as f64))
    });
    b.method("isFull", |c| {
        let s = this(c)?;
        let chr = c.arg(1);
        let v = free_capacity(c.thread, &s, &chr)? <= 0.0;
        Ok(c.push(v))
    });
    b.method("hasRoomFor", |c| {
        let s = this(c)?;
        let chr = c.arg(1);
        if !is_item(&c.arg(2)) {
            let w = num_arg(c, 2) as f32;
            let floor = if c.nargs() > 3 {
                num_arg(c, 3) as f32
            } else {
                w
            };
            let v = has_room_for_weight(c.thread, &s, &chr, w, floor)?;
            return Ok(c.push(v));
        }
        let item = opt_item_arg(c, 2)?.unwrap();
        if !chr.is_nil() && instance_of(&parent(&s), ISO_GAME_CHARACTER) {
            let heavy = {
                let i = item.borrow();
                i.has_tag("base:heavyitem") || is_human_corpse(&i)
            };
            if heavy && !call(c.thread, &chr, "getVehicle", &[])?.is_nil() {
                return Ok(c.push(false));
            }
        }
        if !is_item_allowed(c.thread, &s, Some(&item))? {
            return Ok(c.push(false));
        }
        // STUB: a bag whose equip parent's inventory is another container
        // (`containingItem.getEquipParent()`) is not ported.
        let e = item_env();
        let w = item
            .borrow()
            .get_unequipped_weight(e.context(), e.owner(&item).as_ref());
        let v = has_room_for_weight(c.thread, &s, &chr, w, w)?;
        Ok(c.push(v))
    });
    b.method("isItemAllowed", |c| {
        let s = this(c)?;
        let item = opt_item_arg(c, 1)?;
        let v = is_item_allowed(c.thread, &s, item.as_ref())?;
        Ok(c.push(v))
    });
    b.method("isRemoveItemAllowed", |c| {
        let s = this(c)?;
        let Some(item) = opt_item_arg(c, 1)? else {
            return Ok(c.push(false));
        };
        let p = parent(&s);
        if p.is_nil() {
            return Ok(c.push(true));
        }
        let v = call_or(
            c.thread,
            &p,
            "isRemoveItemAllowedFromContainer",
            &[c.arg(0), item_value(&item)],
            Value::Bool(true),
        )?;
        Ok(c.push(v.truthy()))
    });
    b.method("canItemFit", |c| {
        let s = this(c)?;
        let item = opt_item_arg(c, 1)?.ok_or_else(npe)?;
        let chr = c.arg(2);
        if is_human_corpse(&item.borrow()) {
            let v = can_human_corpse_fit(c.thread, &s, &chr)?;
            return Ok(c.push(v));
        }
        let occupied = capacity_weight(c.thread, &s)?;
        let available = max_weight(c.thread, &s)? - occupied;
        let e = item_env();
        let w = item
            .borrow()
            .get_unequipped_weight(e.context(), e.owner(&item).as_ref());
        Ok(c.push(available >= w))
    });
    b.method("canHumanCorpseFit", |c| {
        let s = this(c)?;
        let v = can_human_corpse_fit(c.thread, &s, &c.arg(1))?;
        Ok(c.push(v))
    });
    b.method("isInCharacterInventory", |c| {
        let s = this(c)?;
        let v = in_character_inventory(c.thread, &s, &c.arg(1))?;
        Ok(c.push(v))
    });
    b.method("isInside", |c| {
        let s = this(c)?;
        let item = opt_item_arg(c, 1)?;
        let mut cur = s;
        loop {
            let Some(bag) = containing_item(&cur) else {
                return Ok(c.push(false));
            };
            if item.as_ref().is_some_and(|i| Rc::ptr_eq(i, &bag)) {
                return Ok(c.push(true));
            }
            match container_of(&bag) {
                Some(o) => cur = o,
                None => return Ok(c.push(false)),
            }
        }
    });
}

/// `getFreeCapacity(chr)`.
fn free_capacity(t: &mut Thread, c: &ContainerRef, chr: &Value) -> LuaResult<f32> {
    let cap = effective_capacity(t, c, chr)? as f32;
    let w = floating_point_correction(capacity_weight(t, c)?);
    Ok((cap - w).max(0.0))
}

/// `canHumanCorpseFit(chr)` (no vehicle parts).
fn can_human_corpse_fit(t: &mut Thread, c: &ContainerRef, chr: &Value) -> LuaResult<bool> {
    let available = if chr.is_nil() {
        max_weight(t, c)? - capacity_weight(t, c)?
    } else {
        effective_capacity(t, c, chr)? as f32 - contents_weight(c)
    };
    // IsoGameCharacter.getWeightAsCorpse().
    if available < 20.0 {
        return Ok(false);
    }
    const ALLOWED: [&str; 19] = [
        "bin",
        "cardboardbox",
        "crate",
        "militarycrate",
        "clothingdryer",
        "clothingdryerbasic",
        "clothingwasher",
        "coffin",
        "doghouse",
        "dumpster",
        "fireplace",
        "fridge",
        "freezer",
        "locker",
        "militarylocker",
        "postbox",
        "shelter",
        "tent",
        "wardrobe",
    ];
    let kind = c.borrow().kind.clone();
    Ok(ALLOWED.iter().any(|a| a.eq_ignore_ascii_case(&kind)))
}

/// Registers `zombie.inventory.ItemContainer`.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, ITEM_CONTAINER_CLASS, None);
    b.field("FRIDGE_FREEZER_TEMPERATURE", 0.2f32 as f64);
    b.field("POWER_SHUTOFF_HOUR_OF_DAY", 7.0);
    b.field("COLD_LOSS_HOURS", 6.0);
    b.static_fn("floatingPointCorrection", |c| {
        let v = floating_point_correction(num_arg(c, 0) as f32);
        Ok(c.push(v as f64))
    });
    b.ctor(|c| {
        // ItemContainer() / (int id) / (String, square, parent) /
        // (int id, String, square, parent).
        let (id, rest) = match (c.nargs(), c.arg(0)) {
            (0, _) => (-1, None),
            (1, _) => (int_arg(c, 0), None),
            (3, _) => (-1, Some(0)),
            _ => (int_arg(c, 0), Some(1)),
        };
        let s = match rest {
            None => SharedContainer {
                id,
                ..Default::default()
            },
            Some(at) => {
                let kind = opt_str_arg(c, at).unwrap_or_default();
                let p = c.arg(at + 2);
                // `isFridge()` in the constructor reads the parent's
                // properties only for other types.
                let mut s = SharedContainer::new(id, &kind);
                let grid = c.arg(at + 1);
                s.source_grid = (!grid.is_nil()).then(|| Rc::new(LuaRef(grid)) as Rc<dyn Any>);
                s.parent = (!p.is_nil()).then(|| Rc::new(LuaRef(p)) as Rc<dyn Any>);
                s
            }
        };
        let r = s.into_ref();
        if rest.is_some() && !r.borrow().is_fridge_type() && is_fridge(c.thread, &r)? {
            let mut m = r.borrow_mut();
            m.age_factor = 0.02;
            m.cooking_factor = 0.0;
        }
        Ok(c.push(container_value(&r)))
    });
    register_fields(&b);
    register_parent(&b);
    register_weights(&b);
    register_items(&b);
    b.method("addItemsToProcessItems", |c| {
        crate::bind_process_items::add_container_items(&this(c)?)?;
        Ok(0)
    });
    b.method("removeItemsFromProcessItems", |c| {
        crate::bind_process_items::remove_container_items(&this(c)?)?;
        Ok(0)
    });
    register_queries(&b);
    register_fluids(&b);
    // `InventoryItem.getContainer()`.
    let ib = expose_class(
        thread,
        crate::bind_items::INVENTORY_ITEM_CLASS,
        Some(crate::bind_items::GAME_ENTITY_CLASS),
    );
    ib.method("getContainer", |c| {
        let item = opt_item_arg(c, 0)?.ok_or_else(npe)?;
        Ok(c.push(item_container_value(&item)))
    });
    ib.method("getOutermostContainer", |c| {
        let item = opt_item_arg(c, 0)?.ok_or_else(npe)?;
        let v = shared::outermost_container_of(&item).map_or(Value::Nil, |o| container_value(&o));
        Ok(c.push(v))
    });
    register_bag(thread);
}

/// The bag item and its argument checks.
fn bag_this(c: &NativeCall<'_>) -> LuaResult<(ItemRef, ContainerRef)> {
    let item = opt_item_arg(c, 0)?.ok_or_else(npe)?;
    let inv = bag(&item).ok_or_else(|| {
        class_cast(
            &item_value(&item),
            "zombie.inventory.types.InventoryContainer",
        )
    })?;
    Ok((item, inv))
}

/// `50 - actualWeight`, the most a bag holds.
fn bag_limit(item: &ItemRef) -> f32 {
    50.0f32 - item.borrow().actual_weight
}

/// `zombie.inventory.types.InventoryContainer`'s inventory methods.
fn register_bag(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.inventory.types.InventoryContainer",
        Some(crate::bind_items::INVENTORY_ITEM_CLASS),
    );
    let inventory = |c: &mut NativeCall<'_>| -> LuaResult<usize> {
        let (_, inv) = bag_this(c)?;
        Ok(c.push(container_value(&inv)))
    };
    b.method("getInventory", inventory);
    b.method("getItemContainer", inventory);
    b.method("setItemContainer", |c| {
        let (item, _) = bag_this(c)?;
        let v = c.arg(1);
        let Some(inv) = container_of_value(&v) else {
            // A null container: the shared form always has one.
            return Err(LuaError::runtime(
                "InventoryContainer.setItemContainer(null) is not supported",
            ));
        };
        if let ItemKind::Container(d) = &mut item.borrow_mut().kind {
            d.container = inv;
        }
        Ok(0)
    });
    b.method("getCapacity", |c| {
        let (item, inv) = bag_this(c)?;
        let mut cap = capacity(&inv);
        let limit = bag_limit(&item);
        if cap as f32 > limit {
            cap = limit as i32;
        }
        Ok(c.push(cap as f64))
    });
    b.method("getEffectiveCapacity", |c| {
        let (item, inv) = bag_this(c)?;
        let mut cap = effective_capacity(c.thread, &inv, &c.arg(1))?;
        let limit = bag_limit(&item);
        if cap as f32 > limit {
            cap = limit as i32;
        }
        Ok(c.push(cap as f64))
    });
    b.method("setCapacity", |c| {
        let (item, inv) = bag_this(c)?;
        let cap = int_arg(c, 1);
        let limit = bag_limit(&item);
        if cap as f32 > limit {
            let name = java_string(&item_value(&item));
            DebugType::General.warn(
                None,
                &format!(
                    "Attempting to set capacity of {name}over maximum capacity of {}",
                    pz_kahlua::exec::java_value_of(&Value::Num(limit as f64))
                ),
            );
        }
        set_capacity(&inv, cap, &container_value(&inv));
        Ok(0)
    });
    b.method("getMaxItemSize", |c| {
        let (item, _) = bag_this(c)?;
        let v = item.borrow().template.max_item_size;
        Ok(c.push(v as f64))
    });
    b.method("setWeightReduction", |c| {
        let (item, inv) = bag_this(c)?;
        let v = int_arg(c, 1).clamp(0, 100);
        if let ItemKind::Container(d) = &mut item.borrow_mut().kind {
            d.weight_reduction = v;
        }
        inv.borrow_mut().weight_reduction = v;
        Ok(0)
    });
    b.method("isEmpty", |c| {
        let (_, inv) = bag_this(c)?;
        let v = inv.borrow().items.is_empty();
        Ok(c.push(v))
    });
}

/// `ItemContainer.setCapacity(capacity)` (`me` is the container's object,
/// for the warning).
fn set_capacity(s: &ContainerRef, cap: i32, me: &Value) {
    let old = s.borrow().capacity;
    if let Some(bag) = containing_item(s).filter(|_| old > 50) {
        let name = java_string(&item_value(&bag));
        DebugType::General.warn(
            None,
            &format!("Attempting to set capacity of {name}over maximum capacity of 50"),
        );
    } else if cap > 100 {
        let me = java_string(me);
        DebugType::General.warn(
            None,
            &format!("Attempting to set capacity of {me}over maximum capacity of 100"),
        );
    }
    s.borrow_mut().capacity = cap;
}

#[cfg(test)]
mod tests;
