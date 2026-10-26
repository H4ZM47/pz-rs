//! Lua bindings for the character classes the main menu and character
//! creation use: `CharacterTraitDefinition`, `CharacterProfessionDefinition`
//! (state in `pz-scripts`), `CharacterTraits`, `ObservationFactory` and its
//! `Observation`, `SurvivorFactory` (with `SurvivorType`) and `SurvivorDesc`
//! with its `HumanVisual` and `WornItems`, and `BodyLocations` with its
//! `BodyLocationGroup`s and `BodyLocation`s (state in `pz-characters`).
//! Perk members live in [`crate::bind_skills`].
//!
//! The statics live in a shared [`Characters`] ([`shared_characters`],
//! [`install_characters`]) next to the shared `ScriptManager`
//! ([`crate::bind_registry::shared_script_manager`]).
//!
//! Lists the original returns as copies are `java.util.ArrayList`
//! snapshots; lists it returns live (`getMutuallyExclusiveTraits`,
//! `getGrantedRecipes`, `getExtras`, `getObservations`, the name lists,
//! `getAllGroups`, `getAllLocations`) are live [`JavaList`]s. The mutable
//! ones write back to their owner; an element of the wrong type is a
//! `ClassCastException` at the write (the original fails later, on use).
//! `getAllGroups`/`getAllLocations` are read-only.
//!
//! Not exposed yet: textures, `HashMap`/
//! `LinkedHashMap` statics (`characterTraitDefinitions`, `observationMap`,
//! `CharacterTraits.getTraits`), boxed-number constants of
//! `CharacterTraits`, `HumanVisual`'s outfit and `ItemVisual` returns,
//! Java arrays (`CreateFamily`, `CreateSurvivorGroup`), saving and loading.
//! TODO(Texture, saves).
//!
//! `HumanVisual`'s lazy getters (hair/beard models and colours, skin
//! texture and colour) use the shared population tables
//! ([`crate::bind_appearance::with_styles`]); `SurvivorDesc.HairCommonColors`
//! and `TrouserCommonColors` are live lists of `ImmutableColor` / `Color`.

use crate::bind_core::LuaColorInfo;
use crate::bind_item_visual::{ImmutableColor, IMMUTABLE_COLOR_CLASS};
use crate::bind_misc_classes::{immutable_color_arg, immutable_color_value};
use crate::bind_java_util::{class_cast, JavaList};
use crate::bind_registry::{
    java_error, key_arg, key_value, shared_script_manager, CHARACTER_PROFESSION_CLASS, CHARACTER_TRAIT_CLASS,
    ITEM_BODY_LOCATION_CLASS,
};
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this, JavaEnum};
use pz_characters::survivor::{meet, set_torso};
use pz_characters::{
    BodyLocationGroupRef, BodyLocationRef, CharacterGender, CharacterTraits, Characters, ItemIdentity, Observation,
    ObservationRef, SurvivorDesc, SurvivorType, WornItems,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::items::{
    self as defs, CharacterProfessionDefinition, CharacterProfessionDefinitionRef, CharacterTraitDefinition,
    CharacterTraitDefinitionRef,
};
use pz_scripts::registry::RegistryKey;
use pz_scripts::ScriptManager;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const TRAIT_DEFINITION_CLASS: &str = "zombie.characters.traits.CharacterTraitDefinition";
pub const PROFESSION_DEFINITION_CLASS: &str = "zombie.characters.professions.CharacterProfessionDefinition";
pub const CHARACTER_TRAITS_CLASS: &str = "zombie.characters.traits.CharacterTraits";
pub const OBSERVATION_FACTORY_CLASS: &str = "zombie.characters.traits.ObservationFactory";
pub const OBSERVATION_CLASS: &str = "zombie.characters.traits.ObservationFactory$Observation";
pub const SURVIVOR_FACTORY_CLASS: &str = "zombie.characters.SurvivorFactory";
pub const SURVIVOR_TYPE_CLASS: &str = "zombie.characters.SurvivorFactory$SurvivorType";
pub const SURVIVOR_DESC_CLASS: &str = "zombie.characters.SurvivorDesc";
pub const HUMAN_VISUAL_CLASS: &str = "zombie.core.skinnedmodel.visual.HumanVisual";
pub const WORN_ITEMS_CLASS: &str = "zombie.characters.WornItems.WornItems";
pub const WORN_ITEM_CLASS: &str = "zombie.characters.WornItems.WornItem";
pub const BODY_LOCATIONS_CLASS: &str = "zombie.characters.WornItems.BodyLocations";
pub const BODY_LOCATION_GROUP_CLASS: &str = "zombie.characters.WornItems.BodyLocationGroup";
pub const BODY_LOCATION_CLASS: &str = "zombie.characters.WornItems.BodyLocation";
pub const CHARACTER_GENDER_CLASS: &str = "zombie.characters.CharacterGender";

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<Characters>>>> = const { RefCell::new(None) };
    static HANDLES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
    /// `SurvivorDesc.instance` (the character object), by descriptor.
    static INSTANCES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

/// Canonical descriptors and their character instances belong to one
/// Lua environment; fresh character statics are installed by bootstrap.
pub(crate) fn reset_session_handles() {
    HANDLES.with(|h| h.borrow_mut().clear());
    INSTANCES.with(|i| i.borrow_mut().clear());
}

/// `SurvivorDesc.HairCommonColors`: a live `ArrayList<ImmutableColor>`.
fn hair_common_colors_value(chars: &Rc<RefCell<Characters>>) -> Value {
    let (r, w) = (chars.clone(), chars.clone());
    let list = JavaList::live(
        move || {
            let chars = r.borrow();
            chars
                .survivor_desc
                .hair_common_colors
                .iter()
                .map(|k| immutable_color_value(ImmutableColor { r: k.r, g: k.g, b: k.b, a: k.a }))
                .collect()
        },
        Some(Box::new(move |v: Vec<Value>| -> LuaResult<()> {
            let cols = v
                .iter()
                .map(|x| match x {
                    Value::Object(o) => o.as_any().downcast_ref::<ImmutableColor>().copied(),
                    _ => None,
                }
                .ok_or_else(|| class_cast(x, IMMUTABLE_COLOR_CLASS)))
                .collect::<LuaResult<Vec<_>>>()?;
            w.borrow_mut().survivor_desc.hair_common_colors =
                cols.into_iter().map(|k| pz_characters::ImmutableColor { r: k.r, g: k.g, b: k.b, a: k.a }).collect();
            Ok(())
        })),
    );
    Value::Object(Rc::new(list))
}

/// `SurvivorDesc.setInstance(instance)` from Rust (the `IsoSurvivor`
/// constructor).
pub fn set_survivor_desc_instance(desc: &Rc<RefCell<LuaSurvivorDesc>>, instance: Value) {
    INSTANCES.with(|m| m.borrow_mut().insert(Rc::as_ptr(desc) as usize, instance));
}

/// Make `chars` the state `register_all` binds.
pub fn install_characters(chars: Rc<RefCell<Characters>>) {
    SHARED.with(|s| *s.borrow_mut() = Some(chars));
}

/// The shared character statics, created empty on first use.
pub fn shared_characters() -> Rc<RefCell<Characters>> {
    SHARED.with(|s| s.borrow_mut().get_or_insert_with(Default::default).clone())
}

/// One Lua object per shared engine object (`Rc` identity).
fn canonical<T: ?Sized>(rc: &Rc<T>, make: impl FnOnce() -> Value) -> Value {
    let id = Rc::as_ptr(rc) as *const () as usize;
    HANDLES.with(|h| h.borrow_mut().entry(id).or_insert_with(make).clone())
}

fn opt_string(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

fn opt_value(s: Option<&str>) -> Value {
    s.map(Value::from).unwrap_or(Value::Nil)
}

fn trait_value(k: Option<&Rc<RegistryKey>>) -> Value {
    key_value(CHARACTER_TRAIT_CLASS, k)
}

fn trait_list(keys: &[Option<Rc<RegistryKey>>]) -> Value {
    Value::Object(Rc::new(JavaList::new(keys.iter().map(|k| trait_value(k.as_ref())).collect())))
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

/// A live `ArrayList<String>` over an owner's strings.
fn live_strings<O: 'static>(
    owner: Rc<RefCell<O>>,
    pick: fn(&O) -> &Vec<String>,
    pick_mut: fn(&mut O) -> &mut Vec<String>,
) -> Value {
    let r = owner.clone();
    let write = move |v: Vec<Value>| -> LuaResult<()> {
        let v = v
            .iter()
            .map(|x| x.as_str().map(str::to_string).ok_or_else(|| class_cast(x, "java.lang.String")))
            .collect::<LuaResult<Vec<_>>>()?;
        *pick_mut(&mut owner.borrow_mut()) = v;
        Ok(())
    };
    let read = move || pick(&r.borrow()).iter().map(|s| Value::from(s.as_str())).collect();
    Value::Object(Rc::new(JavaList::live(read, Some(Box::new(write)))))
}

/// A list element that must be a `CharacterTrait` key (or `null`).
fn trait_elem(v: &Value) -> LuaResult<Option<Rc<RegistryKey>>> {
    match v {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<crate::bind_registry::KeyObject>() {
            Some(k) => Ok(Some(k.key.clone())),
            None => Err(class_cast(v, CHARACTER_TRAIT_CLASS)),
        },
        _ => Err(class_cast(v, CHARACTER_TRAIT_CLASS)),
    }
}

// ---- object types ----

/// A `CharacterTraitDefinition`.
pub struct TraitDefObject(pub CharacterTraitDefinitionRef);

impl LuaObject for TraitDefObject {
    fn class_name(&self) -> &str {
        TRAIT_DEFINITION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn trait_def_value(d: Option<&CharacterTraitDefinitionRef>) -> Value {
    match d {
        Some(d) => canonical(d, || Value::Object(Rc::new(TraitDefObject(d.clone())))),
        None => Value::Nil,
    }
}

pub fn trait_def_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<CharacterTraitDefinitionRef>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<TraitDefObject>() {
            Some(d) => Ok(Some(d.0.clone())),
            None => Err(LuaError::runtime(format!("Expected CharacterTraitDefinition but got {}", o.class_name()))),
        },
        _ => Err(LuaError::runtime("Expected CharacterTraitDefinition")),
    }
}

/// A `CharacterProfessionDefinition`.
pub struct ProfessionDefObject(pub CharacterProfessionDefinitionRef);

impl LuaObject for ProfessionDefObject {
    fn class_name(&self) -> &str {
        PROFESSION_DEFINITION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn profession_def_value(d: Option<&CharacterProfessionDefinitionRef>) -> Value {
    match d {
        Some(d) => canonical(d, || Value::Object(Rc::new(ProfessionDefObject(d.clone())))),
        None => Value::Nil,
    }
}

fn profession_def_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<CharacterProfessionDefinitionRef>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ProfessionDefObject>() {
            Some(d) => Ok(Some(d.0.clone())),
            None => Err(LuaError::runtime(format!("Expected CharacterProfessionDefinition but got {}", o.class_name()))),
        },
        _ => Err(LuaError::runtime("Expected CharacterProfessionDefinition")),
    }
}

/// A `CharacterTraits`.
pub struct CharacterTraitsObject(pub RefCell<CharacterTraits>);

impl LuaObject for CharacterTraitsObject {
    fn class_name(&self) -> &str {
        CHARACTER_TRAITS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An `ObservationFactory.Observation`.
pub struct ObservationObject(pub ObservationRef);

impl LuaObject for ObservationObject {
    fn class_name(&self) -> &str {
        OBSERVATION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn observation_value(o: Option<&ObservationRef>) -> Value {
    match o {
        Some(o) => canonical(o, || Value::Object(Rc::new(ObservationObject(o.clone())))),
        None => Value::Nil,
    }
}

/// A worn item: any Lua value, compared by identity.
#[derive(Clone)]
pub struct LuaItem(pub Value);

impl ItemIdentity for LuaItem {
    fn same(&self, other: &Self) -> bool {
        self.0.raw_equals(&other.0)
    }
}

pub type LuaSurvivorDesc = SurvivorDesc<LuaItem>;

/// A `SurvivorDesc`, with the objects for its visual, worn items and
/// `getMeta()` table.
pub struct SurvivorDescObject {
    pub desc: Rc<RefCell<LuaSurvivorDesc>>,
    visual: RefCell<Option<Value>>,
    worn_items: RefCell<Option<Value>>,
    meta: RefCell<Value>,
}

impl SurvivorDescObject {
    pub fn new_value(desc: LuaSurvivorDesc) -> Value {
        Value::Object(Rc::new(SurvivorDescObject {
            desc: Rc::new(RefCell::new(desc)),
            visual: RefCell::default(),
            worn_items: RefCell::default(),
            meta: RefCell::default(),
        }))
    }
}

impl LuaObject for SurvivorDescObject {
    fn class_name(&self) -> &str {
        SURVIVOR_DESC_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The canonical Lua object of an existing descriptor.
pub fn survivor_desc_value(desc: &Rc<RefCell<LuaSurvivorDesc>>) -> Value {
    canonical(desc, || {
        Value::Object(Rc::new(SurvivorDescObject {
            desc: desc.clone(),
            visual: RefCell::default(),
            worn_items: RefCell::default(),
            meta: RefCell::default(),
        }))
    })
}

/// Makes `v` (a `SurvivorDesc` object) the canonical object of its
/// descriptor, unless it already has one.
pub fn adopt_survivor_desc_value(v: &Value) {
    if let Value::Object(o) = v {
        if let Some(d) = o.as_any().downcast_ref::<SurvivorDescObject>() {
            let desc = d.desc.clone();
            canonical(&desc, || v.clone());
        }
    }
}

pub fn desc_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<RefCell<LuaSurvivorDesc>>> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<SurvivorDescObject>() {
            Some(d) => Ok(d.desc.clone()),
            None => Err(LuaError::runtime(format!("Expected SurvivorDesc but got {}", o.class_name()))),
        },
        Value::Nil => Err(npe()),
        _ => Err(LuaError::runtime("Expected SurvivorDesc")),
    }
}

/// A `SurvivorDesc`'s `HumanVisual`.
pub struct HumanVisualObject(pub Rc<RefCell<LuaSurvivorDesc>>);

impl LuaObject for HumanVisualObject {
    fn class_name(&self) -> &str {
        HUMAN_VISUAL_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["zombie.core.skinnedmodel.visual.BaseVisual"]
    }
}

/// A `WornItems`: a `SurvivorDesc`'s, or a character's (`getWornItems()`,
/// whose items are the ids of items in its inventory).
#[derive(Clone)]
pub enum WornItemsObject {
    Desc(Rc<RefCell<LuaSurvivorDesc>>),
    Character(crate::bind_character_sim::LuaGameCharacterRef),
}

impl LuaObject for WornItemsObject {
    fn class_name(&self) -> &str {
        WORN_ITEMS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `WornItem`.
pub struct WornItemObject {
    pub location: Option<Rc<RegistryKey>>,
    pub item: Value,
}

impl LuaObject for WornItemObject {
    fn class_name(&self) -> &str {
        WORN_ITEM_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `BodyLocationGroup`.
pub struct BodyLocationGroupObject(pub BodyLocationGroupRef);

impl LuaObject for BodyLocationGroupObject {
    fn class_name(&self) -> &str {
        BODY_LOCATION_GROUP_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(crate) fn group_value(g: &BodyLocationGroupRef) -> Value {
    canonical(g, || Value::Object(Rc::new(BodyLocationGroupObject(g.clone()))))
}

/// A `BodyLocation`.
pub struct BodyLocationObject(pub BodyLocationRef);

impl LuaObject for BodyLocationObject {
    fn class_name(&self) -> &str {
        BODY_LOCATION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn body_location_value(l: Option<&BodyLocationRef>) -> Value {
    match l {
        Some(l) => canonical(l, || Value::Object(Rc::new(BodyLocationObject(l.clone())))),
        None => Value::Nil,
    }
}

/// A `CharacterGender` constant (not exposed as a class by the original).
pub struct GenderObject(pub CharacterGender);

impl LuaObject for GenderObject {
    fn class_name(&self) -> &str {
        CHARACTER_GENDER_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.gender_str().to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static GENDERS: [Value; 2] = [
        Value::Object(Rc::new(GenderObject(CharacterGender::Male))),
        Value::Object(Rc::new(GenderObject(CharacterGender::Female))),
    ];
}

fn gender_value(g: CharacterGender) -> Value {
    GENDERS.with(|v| v[g as usize].clone())
}

fn survivor_type_value(t: SurvivorType) -> Value {
    enum_value(SURVIVOR_TYPE_CLASS, t.name())
}

fn survivor_type_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<SurvivorType>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<JavaEnum>() {
            Some(e) if e.class == SURVIVOR_TYPE_CLASS => Ok(Some(SurvivorType::VALUES[e.ordinal as usize])),
            _ => Err(LuaError::runtime(format!("Expected SurvivorType but got {}", o.class_name()))),
        },
        _ => Err(LuaError::runtime("Expected SurvivorType")),
    }
}

// ---- registration ----

/// Register the character classes, bound to `sm` and `chars`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    register_trait_definition(thread, &sm);
    register_profession_definition(thread, &sm);
    register_character_traits(thread, &sm);
    register_observations(thread, &chars);
    register_survivor_factory(thread, &sm, &chars);
    register_survivor_desc(thread, &sm, &chars);
    register_human_visual(thread);
    register_worn_items(thread);
    register_body_locations(thread, &chars);
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    let chars = shared_characters();
    // `Translator.getText` for the name lists (the translator once booted).
    chars.borrow_mut().translator = Some(Rc::new(|k: &str| {
        crate::bind_text::with_translator(|t| t.get_text(k, &[])).unwrap_or_else(|| k.to_string())
    }));
    register(thread, shared_script_manager(), chars);
}

fn tdef(c: &NativeCall<'_>) -> LuaResult<CharacterTraitDefinitionRef> {
    with_this::<TraitDefObject, _>(c, |o| o.0.clone())
}

fn register_trait_definition(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, TRAIT_DEFINITION_CLASS, None);
    let s = sm.clone();
    b.static_fn("getTraits", move |c| {
        let list = defs::character_trait_definitions(&s).iter().map(|d| trait_def_value(Some(d))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(list)))))
    });
    let s = sm.clone();
    b.static_fn("getCharacterTraitDefinition", move |c| {
        let k = key_arg(c, 0)?;
        Ok(c.push(trait_def_value(defs::character_trait_definition(&s, k.as_ref()).as_ref())))
    });
    let s = sm.clone();
    b.static_fn("addCharacterTraitDefinition", move |c| {
        let disabled = c.nargs() >= 6 && bool_arg(c, 5);
        let d = defs::add_character_trait_definition(
            &s,
            key_arg(c, 0)?,
            str_arg(c, 1).as_deref(),
            int_arg(c, 2),
            str_arg(c, 3).as_deref(),
            bool_arg(c, 4),
            disabled,
        )
        .map_err(java_error)?;
        Ok(c.push(trait_def_value(Some(&d))))
    });
    let s = sm.clone();
    b.static_fn("setMutualExclusive", move |c| {
        let (a, bb) = (key_arg(c, 0)?, key_arg(c, 1)?);
        defs::set_mutual_exclusive(&s, a.as_ref(), bb.as_ref()).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.static_fn("reset", move |_| {
        defs::reset_character_trait_definitions(&s);
        Ok(0)
    });
    b.ctor(|c| {
        let d = CharacterTraitDefinition::new(
            key_arg(c, 0)?,
            str_arg(c, 1).as_deref(),
            int_arg(c, 2),
            str_arg(c, 3).as_deref(),
            bool_arg(c, 4),
            bool_arg(c, 5),
        )
        .map_err(java_error)?;
        Ok(c.push(trait_def_value(Some(&Rc::new(RefCell::new(d))))))
    });

    b.method("getType", |c| {
        let d = tdef(c)?;
        let v = trait_value(d.borrow().character_trait_type.as_ref());
        Ok(c.push(v))
    });
    for name in ["getUIName", "getLabel", "getLeftLabel"] {
        b.method(name, |c| {
            let d = tdef(c)?;
            let v = opt_value(d.borrow().display_name.as_deref());
            Ok(c.push(v))
        });
    }
    b.method("getRightLabel", |c| {
        let d = tdef(c)?;
        let v = d.borrow().right_label();
        Ok(c.push(v))
    });
    b.method("getCost", |c| {
        let d = tdef(c)?;
        let v = d.borrow().cost;
        Ok(c.push(v))
    });
    b.method("isFree", |c| {
        let d = tdef(c)?;
        let v = d.borrow().is_profession_trait;
        Ok(c.push(v))
    });
    b.method("getDescription", |c| {
        let d = tdef(c)?;
        let v = opt_value(d.borrow().description.as_deref());
        Ok(c.push(v))
    });
    b.method("setDescription", |c| {
        let d = tdef(c)?;
        d.borrow_mut().description = opt_string(c, 1);
        Ok(0)
    });
    b.method("isDisabledInMultiplayer", |c| {
        let d = tdef(c)?;
        let v = d.borrow().disabled_in_multiplayer;
        Ok(c.push(v))
    });
    b.method("setDisabledInMultiplayer", |c| {
        let d = tdef(c)?;
        d.borrow_mut().disabled_in_multiplayer = bool_arg(c, 1);
        Ok(0)
    });
    b.method("getGrantedTraits", |c| {
        let d = tdef(c)?;
        let v = trait_list(&d.borrow().granted_traits);
        Ok(c.push(v))
    });
    b.method("getGrantedRecipes", |c| {
        let d = tdef(c)?;
        Ok(c.push(live_strings(d, |d| &d.granted_recipes, |d| &mut d.granted_recipes)))
    });
    b.method("isGrantedRecipe", |c| {
        let d = tdef(c)?;
        let v = opt_string(c, 1).is_some_and(|r| d.borrow().is_granted_recipe(&r));
        Ok(c.push(v))
    });
    b.method("hasGrantedRecipes", |c| {
        let d = tdef(c)?;
        let v = !d.borrow().granted_recipes.is_empty();
        Ok(c.push(v))
    });
    let s = sm.clone();
    b.method("addGrantedTrait", move |c| {
        let d = tdef(c)?;
        defs::add_granted_trait(&s, &d, key_arg(c, 1)?);
        Ok(0)
    });
    b.method("addGrantedRecipe", |c| {
        let d = tdef(c)?;
        let r = opt_string(c, 1).ok_or_else(npe)?;
        d.borrow_mut().granted_recipes.push(r);
        Ok(0)
    });
    b.method("getMutuallyExclusiveTraits", |c| {
        let d = tdef(c)?;
        let (r, w) = (d.clone(), d);
        let write = move |v: Vec<Value>| -> LuaResult<()> {
            w.borrow_mut().mutually_exclusive_traits = v.iter().map(trait_elem).collect::<LuaResult<_>>()?;
            Ok(())
        };
        let read = move || r.borrow().mutually_exclusive_traits.iter().map(|k| trait_value(k.as_ref())).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::live(read, Some(Box::new(write)))))))
    });
    b.method("addMutuallyExclusive", |c| {
        let d = tdef(c)?;
        d.borrow_mut().add_mutually_exclusive(key_arg(c, 1)?);
        Ok(0)
    });
    b.method("hasMutuallyExclusiveTraits", |c| {
        let d = tdef(c)?;
        let v = d.borrow().has_mutually_exclusive_traits();
        Ok(c.push(v))
    });
    let s = sm.clone();
    b.method("isMutuallyExclusive", move |c| {
        let d = tdef(c)?;
        let other = trait_def_arg(c, 1)?.ok_or_else(npe)?;
        Ok(c.push(defs::is_mutually_exclusive(&s, &d, &other)))
    });
}

fn pdef(c: &NativeCall<'_>) -> LuaResult<CharacterProfessionDefinitionRef> {
    with_this::<ProfessionDefObject, _>(c, |o| o.0.clone())
}

fn register_profession_definition(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, PROFESSION_DEFINITION_CLASS, None);
    let s = sm.clone();
    b.static_fn("getProfessions", move |c| {
        let list = defs::character_profession_definitions(&s).iter().map(|d| profession_def_value(Some(d))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(list)))))
    });
    let s = sm.clone();
    b.static_fn("getCharacterProfessionDefinition", move |c| {
        let k = key_arg(c, 0)?;
        Ok(c.push(profession_def_value(defs::character_profession_definition(&s, k.as_ref()).as_ref())))
    });
    let s = sm.clone();
    b.static_fn("addCharacterProfessionDefinition", move |c| {
        let d = defs::add_character_profession_definition(
            &s,
            key_arg(c, 0)?,
            str_arg(c, 1).as_deref(),
            int_arg(c, 2),
            str_arg(c, 3).as_deref(),
            str_arg(c, 4).as_deref(),
        )
        .map_err(java_error)?;
        Ok(c.push(profession_def_value(Some(&d))))
    });
    let s = sm.clone();
    b.static_fn("reset", move |_| {
        defs::reset_character_profession_definitions(&s);
        Ok(0)
    });
    let s = sm.clone();
    b.ctor(move |c| {
        let d = CharacterProfessionDefinition::new(
            &s,
            key_arg(c, 0)?,
            str_arg(c, 1).as_deref(),
            int_arg(c, 2),
            str_arg(c, 3).as_deref(),
            str_arg(c, 4).as_deref(),
        )
        .map_err(java_error)?;
        Ok(c.push(profession_def_value(Some(&Rc::new(RefCell::new(d))))))
    });

    b.method("getType", |c| {
        let d = pdef(c)?;
        let v = key_value(CHARACTER_PROFESSION_CLASS, d.borrow().character_profession_type.as_ref());
        Ok(c.push(v))
    });
    for name in ["getUIName", "getLabel", "getLeftLabel"] {
        b.method(name, |c| {
            let d = pdef(c)?;
            let v = opt_value(d.borrow().display_name.as_deref());
            Ok(c.push(v))
        });
    }
    b.method("getRightLabel", |c| {
        let d = pdef(c)?;
        let v = d.borrow().right_label();
        Ok(c.push(v))
    });
    b.method("getCost", |c| {
        let d = pdef(c)?;
        let v = d.borrow().cost;
        Ok(c.push(v))
    });
    b.method("getDescription", |c| {
        let d = pdef(c)?;
        let v = opt_value(d.borrow().description.as_deref());
        Ok(c.push(v))
    });
    b.method("setDescription", |c| {
        let d = pdef(c)?;
        d.borrow_mut().description = opt_string(c, 1);
        Ok(0)
    });
    b.method("getGrantedTraits", |c| {
        let d = pdef(c)?;
        let v = trait_list(&d.borrow().granted_traits);
        Ok(c.push(v))
    });
    b.method("getGrantedRecipes", |c| {
        let d = pdef(c)?;
        Ok(c.push(live_strings(d, |d| &d.granted_recipes, |d| &mut d.granted_recipes)))
    });
    b.method("isGrantedRecipe", |c| {
        let d = pdef(c)?;
        let v = opt_string(c, 1).is_some_and(|r| d.borrow().is_granted_recipe(&r));
        Ok(c.push(v))
    });
    b.method("hasGrantedRecipes", |c| {
        let d = pdef(c)?;
        let v = !d.borrow().granted_recipes.is_empty();
        Ok(c.push(v))
    });
    b.method("addGrantedTrait", |c| {
        let d = pdef(c)?;
        d.borrow_mut().granted_traits.push(key_arg(c, 1)?);
        Ok(0)
    });
    b.method("addGrantedRecipe", |c| {
        let d = pdef(c)?;
        let r = opt_string(c, 1).ok_or_else(npe)?;
        d.borrow_mut().granted_recipes.push(r);
        Ok(0)
    });
}

fn traits_with<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut CharacterTraits) -> R) -> LuaResult<R> {
    with_this::<CharacterTraitsObject, _>(c, |o| f(&mut o.0.borrow_mut()))
}

fn register_character_traits(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, CHARACTER_TRAITS_CLASS, None);
    let s = sm.clone();
    b.ctor(move |c| {
        let t = CharacterTraits::new(&s);
        Ok(c.push(Value::Object(Rc::new(CharacterTraitsObject(RefCell::new(t))))))
    });
    b.method("get", |c| {
        let k = key_arg(c, 1)?;
        let v = traits_with(c, |t| t.get(k.as_ref()))?.map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("set", |c| {
        let (k, v) = (key_arg(c, 1)?, bool_arg(c, 2));
        let r = traits_with(c, |t| t.set(k.as_ref(), v))?.map_err(java_error)?;
        Ok(c.push(r))
    });
    b.method("add", |c| {
        let k = key_arg(c, 1)?;
        traits_with(c, |t| t.add(k.as_ref()))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("remove", |c| {
        let k = key_arg(c, 1)?;
        traits_with(c, |t| t.remove(k.as_ref()))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("getKnownTraits", |c| {
        let known = traits_with(c, |t| t.known_traits())?;
        let list = known.iter().map(|k| trait_value(Some(k))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::new(list)))))
    });
    type Modifier = fn(&CharacterTraits, &ScriptManager) -> pz_scripts::ScriptResult<f32>;
    let modifiers: [(&str, Modifier); 3] = [
        ("getTraitDamageDealtReductionModifier", CharacterTraits::trait_damage_dealt_reduction_modifier),
        ("getTraitEnduranceLossModifier", CharacterTraits::trait_endurance_loss_modifier),
        ("getTraitWeatherPenaltyModifier", CharacterTraits::trait_weather_penalty_modifier),
    ];
    for (name, f) in modifiers {
        let s = sm.clone();
        b.method(name, move |c| {
            let v = traits_with(c, |t| f(t, &s))?.map_err(java_error)?;
            Ok(c.push(v as f64))
        });
    }
}

fn obs<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Observation) -> R) -> LuaResult<R> {
    with_this::<ObservationObject, _>(c, |o| f(&mut o.0.borrow_mut()))
}

fn register_observations(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, OBSERVATION_FACTORY_CLASS, None);
    b.static_fn("init", |_| Ok(0));
    let ch = chars.clone();
    b.static_fn("addObservation", move |c| {
        let t = opt_string(c, 0).ok_or_else(npe)?;
        ch.borrow_mut().observations.add_observation(&t, str_arg(c, 1).as_deref(), str_arg(c, 2).as_deref());
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("getObservation", move |c| {
        let o = opt_string(c, 0).and_then(|n| ch.borrow().observations.get_observation(&n));
        Ok(c.push(observation_value(o.as_ref())))
    });
    let ch = chars.clone();
    b.static_fn("setMutualExclusive", move |c| {
        let (a, bb) = (opt_string(c, 0).ok_or_else(npe)?, opt_string(c, 1).ok_or_else(npe)?);
        ch.borrow().observations.set_mutual_exclusive(&a, &bb).map_err(java_error)?;
        Ok(0)
    });

    let b = expose_class(thread, OBSERVATION_CLASS, None);
    b.ctor(|c| {
        let o = Observation::new(str_arg(c, 0).as_deref(), str_arg(c, 1).as_deref(), str_arg(c, 2).as_deref());
        Ok(c.push(observation_value(Some(&Rc::new(RefCell::new(o))))))
    });
    for name in ["getLabel", "getLeftLabel", "getName"] {
        b.method(name, |c| {
            let v = obs(c, |o| opt_value(o.name.as_deref()))?;
            Ok(c.push(v))
        });
    }
    b.method("getRightLabel", |c| Ok(c.push(Value::Nil)));
    b.method("getDescription", |c| {
        let v = obs(c, |o| opt_value(o.description.as_deref()))?;
        Ok(c.push(v))
    });
    b.method("getTraitID", |c| {
        let v = obs(c, |o| opt_value(o.trait_id.as_deref()))?;
        Ok(c.push(v))
    });
    b.method("setDescription", |c| {
        let v = opt_string(c, 1);
        obs(c, |o| o.description = v)?;
        Ok(0)
    });
    b.method("setTraitID", |c| {
        let v = opt_string(c, 1);
        obs(c, |o| o.trait_id = v)?;
        Ok(0)
    });
    b.method("setName", |c| {
        let v = opt_string(c, 1);
        obs(c, |o| o.name = v)?;
        Ok(0)
    });
}


fn register_survivor_factory(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, SURVIVOR_FACTORY_CLASS, None);
    b.field("FemaleForenames", live_strings(chars.clone(), |c| &c.survivor_factory.female_forenames, |c| &mut c.survivor_factory.female_forenames));
    b.field("MaleForenames", live_strings(chars.clone(), |c| &c.survivor_factory.male_forenames, |c| &mut c.survivor_factory.male_forenames));
    b.field("Surnames", live_strings(chars.clone(), |c| &c.survivor_factory.surnames, |c| &mut c.survivor_factory.surnames));
    let ch = chars.clone();
    b.static_fn("Reset", move |_| {
        ch.borrow_mut().survivor_factory_reset();
        Ok(0)
    });
    type Add = fn(&mut Characters, &str);
    let adders: [(&str, Add); 3] = [
        ("addSurname", Characters::add_surname),
        ("addFemaleForename", Characters::add_female_forename),
        ("addMaleForename", Characters::add_male_forename),
    ];
    for (name, add) in adders {
        let ch = chars.clone();
        b.static_fn(name, move |c| {
            // `"prefix" + null` concatenates "null".
            let n = opt_string(c, 0).unwrap_or_else(|| "null".to_string());
            add(&mut ch.borrow_mut(), &n);
            Ok(0)
        });
    }
    let ch = chars.clone();
    b.static_fn("getRandomSurname", move |c| {
        let s = ch.borrow().random_surname().map_err(java_error)?;
        Ok(c.push(s))
    });
    let ch = chars.clone();
    b.static_fn("getRandomForename", move |c| {
        let s = ch.borrow().random_forename(bool_arg(c, 0)).map_err(java_error)?;
        Ok(c.push(s))
    });
    let ch = chars.clone();
    b.static_fn("randomName", move |c| {
        let d = desc_arg(c, 0)?;
        ch.borrow().random_name(&mut d.borrow_mut()).map_err(java_error)?;
        Ok(0)
    });
    b.static_fn("setTorso", |c| {
        let d = desc_arg(c, 0)?;
        set_torso(&mut d.borrow_mut());
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.static_fn("CreateSurvivor", move |c| {
        let mut chars = ch.borrow_mut();
        let d = match (c.nargs(), survivor_type_arg(c, 0)?) {
            (0, _) => chars.create_survivor(&s),
            (1, t) => chars.create_survivor_of_type(&s, t.ok_or_else(npe)?),
            (_, t) => chars.create_survivor_with(&s, t.ok_or_else(npe)?, bool_arg(c, 1)),
        }
        .map_err(java_error)?;
        drop(chars);
        Ok(c.push(SurvivorDescObject::new_value(d)))
    });

    let b = expose_enum(thread, SURVIVOR_TYPE_CLASS, SurvivorType::NAMES);
    let _ = b;
}

fn dobj<R>(c: &NativeCall<'_>, f: impl FnOnce(&SurvivorDescObject) -> R) -> LuaResult<R> {
    with_this::<SurvivorDescObject, _>(c, f)
}

fn desc<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut LuaSurvivorDesc) -> R) -> LuaResult<R> {
    dobj(c, |o| f(&mut o.desc.borrow_mut()))
}

macro_rules! desc_string {
    ($b:expr, $get:literal, $set:literal, $field:ident) => {
        $b.method($get, |c| {
            let v = desc(c, |d| opt_value(d.$field.as_deref()))?;
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            let v = opt_string(c, 1);
            desc(c, |d| d.$field = v)?;
            Ok(0)
        });
    };
}

macro_rules! desc_float {
    ($b:expr, $get:literal, $set:literal, $field:ident) => {
        $b.method($get, |c| {
            let v = desc(c, |d| d.$field as f64)?;
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            let v = num_arg(c, 1) as f32;
            desc(c, |d| d.$field = v)?;
            Ok(0)
        });
    };
}

fn color_info_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<pz_core::core::ColorInfo> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<LuaColorInfo>() {
            Some(ci) => Ok(ci.c.get()),
            None => Err(LuaError::runtime(format!("Expected ColorInfo but got {}", o.class_name()))),
        },
        _ => Err(npe()),
    }
}

fn register_survivor_desc(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, SURVIVOR_DESC_CLASS, None);
    let ch = chars.clone();
    b.static_fn("getIDCount", move |c| {
        let v = ch.borrow().survivor_desc.id_count;
        Ok(c.push(v))
    });
    let ch = chars.clone();
    b.static_fn("setIDCount", move |c| {
        ch.borrow_mut().survivor_desc.id_count = int_arg(c, 0);
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("addTrouserColor", move |c| {
        let ci = color_info_arg(c, 0)?;
        ch.borrow_mut().add_trouser_color(ci.r, ci.g, ci.b, ci.a);
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("addHairColor", move |c| {
        let ci = color_info_arg(c, 0)?;
        ch.borrow_mut().add_hair_color(ci.r, ci.g, ci.b, ci.a);
        Ok(0)
    });
    // The final `HairCommonColors` / `TrouserCommonColors` lists (filled by
    // Lua's `addHairColor` / `addTrouserColor`); `getCommonHairColor`
    // returns the same list.
    let hair = hair_common_colors_value(chars);
    b.field("HairCommonColors", hair.clone());
    let (r, w) = (chars.clone(), chars.clone());
    let trousers = JavaList::live(
        move || {
            let chars = r.borrow();
            chars.survivor_desc.trouser_common_colors.iter().map(|k| crate::bind_misc_classes::color_value_of(k.r, k.g, k.b, k.a)).collect()
        },
        Some(Box::new(move |v: Vec<Value>| -> LuaResult<()> {
            let cols = v
                .iter()
                .map(|x| crate::bind_misc_classes::color_of(x).ok_or_else(|| class_cast(x, "zombie.core.Color")))
                .collect::<LuaResult<Vec<_>>>()?;
            w.borrow_mut().survivor_desc.trouser_common_colors =
                cols.into_iter().map(|[r, g, b, a]| pz_characters::Color { r, g, b, a }).collect();
            Ok(())
        })),
    );
    b.field("TrouserCommonColors", Value::Object(Rc::new(trousers)));
    b.method("getCommonHairColor", move |c| Ok(c.push(hair.clone())));
    b.static_fn("getRandomSkinColor", |c| {
        let [r, g, bl] = pz_characters::population::random_skin_color();
        Ok(c.push(crate::bind_misc_classes::color_value_of(r, g, bl, 1.0)))
    });
    b.method("setInstance", |c| {
        let (d, v) = (dobj(c, |o| o.desc.clone())?, c.arg(1));
        INSTANCES.with(|m| {
            let id = Rc::as_ptr(&d) as usize;
            if v.is_nil() {
                m.borrow_mut().remove(&id);
            } else {
                m.borrow_mut().insert(id, v);
            }
        });
        Ok(0)
    });
    b.method("getInstance", |c| {
        let d = dobj(c, |o| o.desc.clone())?;
        let v = INSTANCES.with(|m| m.borrow().get(&(Rc::as_ptr(&d) as usize)).cloned());
        Ok(c.push(v.unwrap_or_default()))
    });
    b.method("dressInNamedOutfit", |c| {
        let d = dobj(c, |o| o.desc.clone())?;
        let name = opt_string(c, 1);
        let visuals = crate::bind_appearance::with_dress_context(c.thread, |ctx| {
            let mut desc = d.borrow_mut();
            let female = desc.is_female();
            let mut hv = std::mem::replace(&mut desc.human_visual, pz_inventory::visual::HumanVisual::new(0));
            let mut v = Vec::new();
            let r = pz_characters::population::dress_in_named_outfit(&mut hv, female, name.as_deref(), &mut v, ctx);
            desc.human_visual = hv;
            r.map(|()| v)
        })
        .unwrap_or_else(|| Ok(Vec::new()))
        .map_err(LuaError::runtime)?;
        crate::bind_item_visual::set_worn_items_from_item_visuals(c.thread, &d, &visuals).map_err(java_error)?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.ctor(move |c| {
        let mut chars = ch.borrow_mut();
        let d = match c.arg(0) {
            Value::Object(_) => {
                let other = desc_arg(c, 0)?;
                let other = other.borrow();
                SurvivorDesc::copy_of(&mut chars, &s, &other)
            }
            Value::Bool(_) => SurvivorDesc::new_unregistered(&mut chars, &s),
            _ => SurvivorDesc::new(&mut chars, &s),
        }
        .map_err(java_error)?;
        drop(chars);
        Ok(c.push(SurvivorDescObject::new_value(d)))
    });

    desc_string!(b, "getForename", "setForename", forename);
    desc_string!(b, "getSurname", "setSurname", surname);
    desc_string!(b, "getTorso", "setTorso", torso);
    desc_string!(b, "getInventoryScript", "setInventoryScript", inventory_script);
    desc_string!(b, "getVoicePrefix", "setVoicePrefix", voice_prefix);
    desc_float!(b, "getVoicePitch", "setVoicePitch", voice_pitch);
    desc_float!(b, "getBravery", "setBravery", bravery);
    desc_float!(b, "getLoner", "setLoner", loner);
    desc_float!(b, "getAggressiveness", "setAggressiveness", aggressiveness);
    desc_float!(b, "getCompassion", "setCompassion", compassion);
    desc_float!(b, "getTemper", "setTemper", temper);
    desc_float!(b, "getFriendliness", "setFriendliness", friendliness);
    desc_float!(b, "getFavourindoors", "setFavourindoors", favourindoors);
    desc_float!(b, "getLoyalty", "setLoyalty", loyalty);
    b.method("getVoiceType", |c| {
        let v = desc(c, |d| d.voice_type)?;
        Ok(c.push(v))
    });
    b.method("setVoiceType", |c| {
        let v = int_arg(c, 1);
        desc(c, |d| d.voice_type = v)?;
        Ok(0)
    });
    b.method("getID", |c| {
        let v = desc(c, |d| d.id)?;
        Ok(c.push(v))
    });
    b.method("setID", |c| {
        let v = int_arg(c, 1);
        desc(c, |d| d.id = v)?;
        Ok(0)
    });
    b.method("isDead", |c| {
        let v = desc(c, |d| d.dead)?;
        Ok(c.push(v))
    });
    b.method("setDead", |c| {
        let v = bool_arg(c, 1);
        desc(c, |d| d.dead = v)?;
        Ok(0)
    });
    b.method("isFemale", |c| {
        let v = desc(c, |d| d.is_female())?;
        Ok(c.push(v))
    });
    b.method("setFemale", |c| {
        let v = bool_arg(c, 1);
        desc(c, |d| d.set_female(v))?;
        Ok(0)
    });
    b.method("isZombie", |c| Ok(c.push(false)));
    b.method("isSkeleton", |c| Ok(c.push(false)));
    b.method("isLeader", |c| Ok(c.push(false)));
    b.method("getCharacterGender", |c| {
        let g = desc(c, |d| d.character_gender)?;
        Ok(c.push(gender_value(g)))
    });
    b.method("setCharacterGender", |c| {
        let g = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<GenderObject>().map(|g| g.0),
            _ => None,
        };
        // A `null` gender reads as male (`characterGender == FEMALE` fails).
        desc(c, |d| d.character_gender = g.unwrap_or(CharacterGender::Male))?;
        Ok(0)
    });
    b.method("getFullname", |c| {
        let v = desc(c, |d| d.full_name())?;
        Ok(c.push(v))
    });
    b.method("getType", |c| {
        let t = desc(c, |d| d.survivor_type)?;
        Ok(c.push(survivor_type_value(t)))
    });
    b.method("setType", |c| {
        let t = survivor_type_arg(c, 1)?.ok_or_else(npe)?;
        desc(c, |d| d.survivor_type = t)?;
        Ok(0)
    });
    b.method("getCharacterProfession", |c| {
        let p = desc(c, |d| d.character_profession.clone())?;
        Ok(c.push(key_value(CHARACTER_PROFESSION_CLASS, p.as_ref())))
    });
    b.method("setCharacterProfession", |c| {
        let p = key_arg(c, 1)?;
        desc(c, |d| d.character_profession = p)?;
        Ok(0)
    });
    b.method("isCharacterProfession", |c| {
        let p = key_arg(c, 1)?;
        let v = desc(c, |d| d.is_character_profession(p.as_ref()))?;
        Ok(c.push(v))
    });
    b.method("setProfessionSkills", |c| {
        let p = profession_def_arg(c, 1)?.ok_or_else(npe)?;
        let p = p.borrow();
        desc(c, |d| d.set_profession_skills(&p))?;
        Ok(0)
    });
    b.method("getExtras", |c| {
        let d = dobj(c, |o| o.desc.clone())?;
        Ok(c.push(live_strings(d, |d| &d.extra, |d| &mut d.extra)))
    });
    b.method("getObservations", |c| {
        let d = dobj(c, |o| o.desc.clone())?;
        let (r, w) = (d.clone(), d);
        let write = move |v: Vec<Value>| -> LuaResult<()> {
            let obs = v
                .iter()
                .map(|x| match x {
                    Value::Object(o) => o.as_any().downcast_ref::<ObservationObject>().map(|o| o.0.clone()),
                    _ => None,
                })
                .zip(&v)
                .map(|(o, x)| o.ok_or_else(|| class_cast(x, OBSERVATION_CLASS)))
                .collect::<LuaResult<_>>()?;
            w.borrow_mut().observations = obs;
            Ok(())
        };
        let read = move || r.borrow().observations.iter().map(|o| observation_value(Some(o))).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::live(read, Some(Box::new(write)))))))
    });
    let ch = chars.clone();
    b.method("addObservation", move |c| {
        let o = opt_string(c, 1).ok_or_else(npe)?;
        let chars = ch.borrow();
        desc(c, |d| d.add_observation(&chars, &o))?;
        Ok(0)
    });
    b.method("hasObservation", |c| {
        let o = opt_string(c, 1).ok_or_else(npe)?;
        let v = desc(c, |d| d.has_observation(&o))?;
        Ok(c.push(v))
    });
    b.method("isAggressive", |c| {
        let v = desc(c, |d| d.is_aggressive())?;
        Ok(c.push(v))
    });
    b.method("isFriendly", |c| {
        let v = desc(c, |d| d.is_friendly())?;
        Ok(c.push(v))
    });
    b.method("meet", |c| {
        let (a, other) = (dobj(c, |o| o.desc.clone())?, desc_arg(c, 1)?);
        meet(&a, &other);
        Ok(0)
    });
    b.method("getMetCount", |c| {
        // TODO(HashMap): the no-argument overload returns the map.
        let other = desc_arg(c, 1)?;
        let id = other.borrow().id;
        let v = desc(c, |d| d.met_count_of(id))?;
        Ok(c.push(v))
    });
    b.method("getDescription", |c| {
        let s = opt_string(c, 1).unwrap_or_else(|| "null".to_string());
        let v = desc(c, |d| d.description(&s))?;
        Ok(c.push(v))
    });
    b.method("getHumanVisual", |c| {
        let v = dobj(c, |o| {
            o.visual
                .borrow_mut()
                .get_or_insert_with(|| Value::Object(Rc::new(HumanVisualObject(o.desc.clone()))))
                .clone()
        })?;
        Ok(c.push(v))
    });
    b.method("getWornItems", |c| {
        let v = dobj(c, |o| {
            o.worn_items.borrow_mut().get_or_insert_with(|| Value::Object(Rc::new(WornItemsObject::Desc(o.desc.clone())))).clone()
        })?;
        Ok(c.push(v))
    });
    b.method("setWornItem", |c| {
        let (loc, item) = (key_arg(c, 1)?, c.arg(2));
        let item = (!item.is_nil()).then_some(LuaItem(item));
        desc(c, |d| d.worn_items.set_item(loc, item))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("getWornItem", |c| {
        let loc = key_arg(c, 1)?;
        let item = desc(c, |d| d.worn_items.item(loc.as_ref()))?.map_err(java_error)?;
        Ok(c.push(item.map(|i| i.0).unwrap_or_default()))
    });
    b.method("getMeta", |c| {
        let v = survivor_meta(c)?;
        Ok(c.push(v))
    });
    b.method("getCalculatedToughness", |c| {
        let meta = survivor_meta(c)?;
        let tab = c.thread.globals().rawget(&Value::str("MetaSurvivor"));
        let f = match tab {
            Value::Table(t) => t.rawget(&Value::str("getCalculatedToughness")),
            _ => return Err(npe()),
        };
        let r = c.thread.pcall(&f, &[meta]).map_err(|(e, _)| LuaError::Kahlua(e))?;
        let tough = r.into_iter().next().unwrap_or_default();
        match tough {
            Value::Num(n) => Ok(c.push(n as i32)),
            _ => Err(LuaError::Runtime { message: String::new(), class: "java.lang.ClassCastException" }),
        }
    });
}

/// `getMeta`: `createMetaSurvivor(this)` from the Lua environment, cached.
fn survivor_meta(c: &mut NativeCall<'_>) -> LuaResult<Value> {
    let (cached, this) = (dobj(c, |o| o.meta.borrow().clone())?, c.arg(0));
    if !cached.is_nil() {
        return Ok(cached);
    }
    let f = c.thread.globals().rawget(&Value::str("createMetaSurvivor"));
    let r = c.thread.pcall(&f, &[this]).map_err(|(e, _)| LuaError::Kahlua(e))?;
    let meta = r.into_iter().next().unwrap_or_default();
    if !matches!(meta, Value::Table(_) | Value::Nil) {
        return Err(LuaError::Runtime { message: String::new(), class: "java.lang.ClassCastException" });
    }
    dobj(c, |o| *o.meta.borrow_mut() = meta.clone())?;
    Ok(meta)
}

fn visual<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut LuaSurvivorDesc) -> R) -> LuaResult<R> {
    with_this::<HumanVisualObject, _>(c, |o| f(&mut o.0.borrow_mut()))
}

/// Runs `f` on the visual of a `HumanVisual` object with its owner (the
/// descriptor) and the real style lookups ([`crate::bind_appearance::with_styles`]).
fn visual_styled<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_inventory::visual::HumanVisual, &LuaSurvivorDesc, &dyn pz_inventory::visual::human::HumanStyles) -> R,
) -> LuaResult<R> {
    let desc = with_this::<HumanVisualObject, _>(c, |o| o.0.clone())?;
    Ok(crate::bind_appearance::with_styles(c.thread, |styles| {
        let mut d = desc.borrow_mut();
        let mut hv = std::mem::replace(&mut d.human_visual, pz_inventory::visual::HumanVisual::new(0));
        let r = f(&mut hv, &d, styles);
        d.human_visual = hv;
        r
    }))
}

fn rgb_value(c: Option<pz_inventory::visual::Rgb>) -> Value {
    c.map_or(Value::Nil, |[r, g, b]| immutable_color_value(ImmutableColor { r, g, b, a: 1.0 }))
}

/// An `ImmutableColor` argument as the visual's RGB (`null` stays unset).
fn rgb_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<pz_inventory::visual::Rgb>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        v => immutable_color_arg(c, i).map(|k| Some([k.r, k.g, k.b])).ok_or_else(|| class_cast(&v, IMMUTABLE_COLOR_CLASS)),
    }
}

fn register_human_visual(thread: &mut Thread) {
    use pz_inventory::visual::human::HumanVisualOwner;
    use pz_inventory::visual::HumanVisual;
    let b = expose_class(thread, HUMAN_VISUAL_CLASS, None);
    // `HumanVisual.isFemale()` etc. ask the owner (`IHumanVisual`).
    b.method("isFemale", |c| {
        let v = visual(c, |d| HumanVisualOwner::is_female(d))?;
        Ok(c.push(v))
    });
    b.method("isZombie", |c| {
        let v = visual(c, |d| HumanVisualOwner::is_zombie(d))?;
        Ok(c.push(v))
    });
    b.method("isSkeleton", |c| {
        let v = visual(c, |d| HumanVisualOwner::is_skeleton(d))?;
        Ok(c.push(v))
    });
    b.method("getSkinTextureIndex", |c| {
        let v = visual(c, |d| d.human_visual.get_skin_texture_index())?;
        Ok(c.push(v))
    });
    b.method("setSkinTextureIndex", |c| {
        let v = int_arg(c, 1);
        visual(c, |d| d.human_visual.set_skin_texture_index(v))?;
        Ok(0)
    });
    b.method("getSkinTexture", |c| {
        let name = visual_styled(c, |hv, d, s| hv.get_skin_texture(d, s))?;
        // `textures.get(skinTexture)` on an empty skin list throws.
        let name = name.ok_or_else(|| LuaError::Runtime {
            message: "Index 0 out of bounds for length 0".into(),
            class: "java.lang.IndexOutOfBoundsException",
        })?;
        Ok(c.push(name))
    });
    b.method("getSkinColor", |c| {
        let v = visual_styled(c, |hv, _, s| hv.get_skin_color(s))?;
        Ok(c.push(rgb_value(Some(v))))
    });
    b.method("setSkinColor", |c| {
        let v = rgb_arg(c, 1)?;
        visual(c, |d| d.human_visual.set_skin_color(v))?;
        Ok(0)
    });
    b.method("getBodyHairIndex", |c| {
        let v = visual(c, |d| d.human_visual.get_body_hair_index())?;
        Ok(c.push(v))
    });
    b.method("setBodyHairIndex", |c| {
        let v = int_arg(c, 1);
        visual(c, |d| d.human_visual.set_body_hair_index(v))?;
        Ok(0)
    });
    b.method("setSkinTextureName", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_skin_texture_name(v.as_deref()))?;
        Ok(0)
    });
    type GetColor = fn(&mut HumanVisual, &dyn pz_inventory::visual::human::HumanStyles) -> Option<pz_inventory::visual::Rgb>;
    type SetColor = fn(&mut HumanVisual, Option<pz_inventory::visual::Rgb>);
    let colors: [(&str, GetColor, &str, SetColor); 4] = [
        ("getHairColor", |v, s| v.get_hair_color(s), "setHairColor", HumanVisual::set_hair_color),
        ("getBeardColor", |v, s| v.get_beard_color(s), "setBeardColor", HumanVisual::set_beard_color),
        ("getNaturalHairColor", |v, s| v.get_natural_hair_color(s), "setNaturalHairColor", HumanVisual::set_natural_hair_color),
        ("getNaturalBeardColor", |v, s| v.get_natural_beard_color(s), "setNaturalBeardColor", HumanVisual::set_natural_beard_color),
    ];
    for (get, g, set, st) in colors {
        b.method(get, move |c| {
            let v = visual_styled(c, |hv, _, s| g(hv, s))?;
            Ok(c.push(rgb_value(v)))
        });
        b.method(set, move |c| {
            let v = rgb_arg(c, 1)?;
            visual(c, |d| st(&mut d.human_visual, v))?;
            Ok(0)
        });
    }
    b.method("getHairModel", |c| {
        let v = visual_styled(c, |hv, d, s| hv.get_hair_model(d, s))?;
        Ok(c.push(opt_value(v.as_deref())))
    });
    b.method("setHairModel", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_hair_model(v.as_deref()))?;
        Ok(0)
    });
    b.method("getBeardModel", |c| {
        let v = visual_styled(c, |hv, d, s| hv.get_beard_model(d, s))?;
        Ok(c.push(opt_value(v.as_deref())))
    });
    b.method("setBeardModel", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_beard_model(v.as_deref()))?;
        Ok(0)
    });
    b.method("getNonAttachedHair", |c| {
        let v = visual(c, |d| opt_value(d.human_visual.get_non_attached_hair()))?;
        Ok(c.push(v))
    });
    b.method("setNonAttachedHair", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_non_attached_hair(v.as_deref()))?;
        Ok(0)
    });
    b.method("setForceModelScript", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_force_model_script(v.as_deref()))?;
        Ok(0)
    });
    b.method("clear", |c| {
        visual(c, |d| d.human_visual.clear())?;
        Ok(0)
    });
    b.method("lerp", |c| {
        let v = HumanVisual::lerp(num_arg(c, 1) as f32, num_arg(c, 2) as f32, num_arg(c, 3) as f32);
        Ok(c.push(v as f64))
    });
    b.method("hasBodyVisualFromItemType", |c| {
        let t = opt_string(c, 1).ok_or_else(npe)?;
        let v = visual(c, |d| d.human_visual.has_body_visual_from_item_type(&t))?;
        Ok(c.push(v))
    });
    // TODO(ItemVisual Lua object): the body visual methods return the
    // `ItemVisual` (nil here).
    b.method("removeBodyVisualFromItemType", |c| {
        let t = opt_string(c, 1).ok_or_else(npe)?;
        visual(c, |d| d.human_visual.remove_body_visual_from_item_type(&t))?;
        Ok(c.push(Value::Nil))
    });
    b.method("addBodyVisualFromItemType", |c| {
        let t = opt_string(c, 1).ok_or_else(npe)?;
        let sm = shared_script_manager();
        let name = defs::find_item(&sm, &t).and_then(|i| i.borrow().clothing_item.clone());
        add_body_visual(c, name)
    });
    for name in ["addBodyVisualFromClothingItemName", "addBodyVisual"] {
        b.method(name, |c| add_body_visual(c, opt_string(c, 1)));
    }
}

/// `addBodyVisualFromClothingItemName`: the script item for the clothing
/// item, its ready asset, no body visual of that name yet; then a
/// randomized `ClothingItemReference` synched into the new visual.
fn add_body_visual(c: &mut NativeCall<'_>, clothing_name: Option<String>) -> LuaResult<usize> {
    let Some(name) = clothing_name.filter(|n| !n.trim().is_empty()) else {
        return Ok(c.push(Value::Nil));
    };
    let desc = with_this::<HumanVisualObject, _>(c, |o| o.0.clone())?;
    let sm = shared_script_manager();
    let Some(script) = defs::get_item_for_clothing_item(&sm, &name) else {
        return Ok(c.push(Value::Nil));
    };
    let (full_type, asset) = {
        let s = script.borrow();
        (s.full_name(), s.clothing_item.clone())
    };
    let r = crate::bind_appearance::with_dress_context(c.thread, |ctx| {
        let Some(item) = asset.as_deref().and_then(|a| ctx.clothing.get(a)) else {
            return Ok(false);
        };
        let mut d = desc.borrow_mut();
        if d.human_visual.body_visuals.iter().any(|v| v.clothing_item_name == name) {
            return Ok(false);
        }
        let mut r = pz_characters::population::ClothingItemReference { item_guid: Some(item.guid.clone()), ..Default::default() };
        r.randomize(ctx.outfit)?;
        let mut v = pz_inventory::visual::ItemVisual::new(&full_type);
        pz_characters::population::dress::synch_with_outfit(&mut v, &r, item);
        d.human_visual.body_visuals.push(v);
        Ok::<bool, String>(true)
    });
    match r {
        Some(Err(e)) => Err(LuaError::runtime(e)),
        _ => Ok(c.push(Value::Nil)),
    }
}

/// What a `WornItems` object works on: its items and the conversions
/// between them and Lua values.
pub(crate) trait WornBacking {
    type Item: ItemIdentity;
    /// Runs `f` on the worn items.
    fn with<R>(&self, f: impl FnOnce(&mut WornItems<Self::Item>) -> R) -> LuaResult<R>;
    /// The Lua value of an item (`nil` for `None`).
    fn value(&self, item: Option<Self::Item>) -> Value;
    /// Argument `i` as an item (`nil` is `None`).
    fn item_arg(&self, c: &NativeCall<'_>, i: usize) -> LuaResult<Option<Self::Item>>;
}

/// A `SurvivorDesc`'s worn items: any Lua values.
struct DescWorn(Rc<RefCell<LuaSurvivorDesc>>);

impl WornBacking for DescWorn {
    type Item = LuaItem;
    fn with<R>(&self, f: impl FnOnce(&mut WornItems<LuaItem>) -> R) -> LuaResult<R> {
        Ok(f(&mut self.0.borrow_mut().worn_items))
    }
    fn value(&self, item: Option<LuaItem>) -> Value {
        item.map(|i| i.0).unwrap_or_default()
    }
    fn item_arg(&self, c: &NativeCall<'_>, i: usize) -> LuaResult<Option<LuaItem>> {
        let v = c.arg(i);
        Ok((!v.is_nil()).then_some(LuaItem(v)))
    }
}

/// Runs `$body` with `$b` bound to the `WornItems` object's backing.
macro_rules! with_worn {
    ($c:expr, |$b:ident| $body:expr) => {{
        match with_this::<WornItemsObject, _>($c, |o| o.clone())? {
            WornItemsObject::Desc(d) => {
                let $b = DescWorn(d);
                $body
            }
            WornItemsObject::Character(chr) => {
                let $b = crate::bind_character_sim::CharacterWorn(chr);
                $body
            }
        }
    }};
}

fn location_value(k: Option<&Rc<RegistryKey>>) -> Value {
    key_value(ITEM_BODY_LOCATION_CLASS, k)
}

fn register_worn_items(thread: &mut Thread) {
    let b = expose_class(thread, WORN_ITEMS_CLASS, None);
    b.method("size", |c| {
        let v = with_worn!(c, |w| w.with(|w| w.size() as i32))?;
        Ok(c.push(v))
    });
    b.method("isEmpty", |c| {
        let v = with_worn!(c, |w| w.with(|w| w.is_empty()))?;
        Ok(c.push(v))
    });
    b.method("clear", |c| {
        with_worn!(c, |w| w.with(|w| w.clear()))?;
        Ok(0)
    });
    b.method("get", |c| {
        let i = int_arg(c, 1);
        let v = with_worn!(c, |b| {
            let w = b.with(|w| w.get(i).cloned())?.map_err(java_error)?;
            WornItemObject { location: w.location, item: b.value(Some(w.item)) }
        });
        Ok(c.push(Value::Object(Rc::new(v))))
    });
    b.method("getItem", |c| {
        let loc = key_arg(c, 1)?;
        let v = with_worn!(c, |b| {
            let item = b.with(|w| w.item(loc.as_ref()))?.map_err(java_error)?;
            b.value(item)
        });
        Ok(c.push(v))
    });
    b.method("getItemByIndex", |c| {
        let i = int_arg(c, 1);
        let v = with_worn!(c, |b| {
            let item = b.with(|w| w.item_by_index(i))?;
            b.value(item)
        });
        Ok(c.push(v))
    });
    b.method("setItem", |c| {
        let loc = key_arg(c, 1)?;
        with_worn!(c, |b| {
            let item = b.item_arg(c, 2)?;
            b.with(|w| w.set_item(loc, item))?.map_err(java_error)?;
        });
        Ok(0)
    });
    b.method("remove", |c| {
        with_worn!(c, |b| {
            if let Some(item) = b.item_arg(c, 1)? {
                b.with(|w| w.remove(&item))?;
            }
        });
        Ok(0)
    });
    b.method("contains", |c| {
        let v = with_worn!(c, |b| match b.item_arg(c, 1)? {
            Some(item) => b.with(|w| w.contains(&item))?,
            None => false,
        });
        Ok(c.push(v))
    });
    b.method("getLocation", |c| {
        let k = with_worn!(c, |b| match b.item_arg(c, 1)? {
            Some(item) => b.with(|w| w.location(&item))?,
            None => None,
        });
        Ok(c.push(location_value(k.as_ref())))
    });
    b.method("getBodyLocationGroup", |c| {
        let g = with_worn!(c, |b| b.with(|w| w.body_location_group().clone())?);
        Ok(c.push(group_value(&g)))
    });

    let b = expose_class(thread, WORN_ITEM_CLASS, None);
    b.method("getItem", |c| {
        let v = with_this::<WornItemObject, _>(c, |w| w.item.clone())?;
        Ok(c.push(v))
    });
    b.method("getLocation", |c| {
        let v = with_this::<WornItemObject, _>(c, |w| location_value(w.location.as_ref()))?;
        Ok(c.push(v))
    });
}

fn group(c: &NativeCall<'_>) -> LuaResult<BodyLocationGroupRef> {
    with_this::<BodyLocationGroupObject, _>(c, |o| o.0.clone())
}

fn body_location(c: &NativeCall<'_>) -> LuaResult<BodyLocationRef> {
    with_this::<BodyLocationObject, _>(c, |o| o.0.clone())
}

fn register_body_locations(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, BODY_LOCATIONS_CLASS, None);
    let ch = chars.clone();
    b.static_fn("getGroup", move |c| {
        let Some(id) = opt_string(c, 0) else {
            return Err(LuaError::Runtime { message: "id is null".into(), class: "java.lang.NullPointerException" });
        };
        let g = ch.borrow_mut().body_locations.get_group(&id).map_err(java_error)?;
        Ok(c.push(group_value(&g)))
    });
    let ch = chars.clone();
    b.static_fn("reset", move |_| {
        ch.borrow_mut().body_locations.reset();
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("getAllGroups", move |c| {
        let ch = ch.clone();
        let view = JavaList::live(move || ch.borrow().body_locations.all_groups().iter().map(group_value).collect(), None);
        Ok(c.push(Value::Object(Rc::new(view))))
    });

    let b = expose_class(thread, BODY_LOCATION_GROUP_CLASS, None);
    b.method("getId", |c| {
        let v = group(c)?.borrow().id().to_string();
        Ok(c.push(v))
    });
    b.method("getLocation", |c| {
        let k = key_arg(c, 1)?;
        let l = group(c)?.borrow().location(k.as_ref()).map_err(java_error)?;
        Ok(c.push(body_location_value(l.as_ref())))
    });
    b.method("getOrCreateLocation", |c| {
        let k = key_arg(c, 1)?;
        let l = group(c)?.borrow_mut().get_or_create_location(k).map_err(java_error)?;
        Ok(c.push(body_location_value(Some(&l))))
    });
    b.method("getLocationByIndex", |c| {
        let l = group(c)?.borrow().location_by_index(int_arg(c, 1));
        Ok(c.push(body_location_value(l.as_ref())))
    });
    b.method("moveLocationToIndex", |c| {
        let k = key_arg(c, 1)?;
        group(c)?.borrow_mut().move_location_to_index(k.as_ref(), int_arg(c, 2)).map_err(java_error)?;
        Ok(0)
    });
    b.method("size", |c| {
        let v = group(c)?.borrow().size() as i32;
        Ok(c.push(v))
    });
    b.method("indexOf", |c| {
        let k = key_arg(c, 1)?;
        let v = group(c)?.borrow().index_of(k.as_ref()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("setExclusive", |c| {
        let (a, k) = (key_arg(c, 1)?, key_arg(c, 2)?);
        group(c)?.borrow_mut().set_exclusive(a, k).map_err(java_error)?;
        Ok(0)
    });
    b.method("setHideModel", |c| {
        let (a, k) = (key_arg(c, 1)?, key_arg(c, 2)?);
        group(c)?.borrow_mut().set_hide_model(a.as_ref(), k).map_err(java_error)?;
        Ok(0)
    });
    b.method("setAltModel", |c| {
        let (a, k) = (key_arg(c, 1)?, key_arg(c, 2)?);
        group(c)?.borrow_mut().set_alt_model(a.as_ref(), k).map_err(java_error)?;
        Ok(0)
    });
    type GroupTest = fn(
        &pz_characters::BodyLocationGroup,
        Option<&Rc<RegistryKey>>,
        Option<&Rc<RegistryKey>>,
    ) -> pz_scripts::ScriptResult<bool>;
    let tests: [(&str, GroupTest); 3] = [
        ("isExclusive", pz_characters::BodyLocationGroup::is_exclusive),
        ("isHideModel", pz_characters::BodyLocationGroup::is_hide_model),
        ("isAltModel", pz_characters::BodyLocationGroup::is_alt_model),
    ];
    for (name, f) in tests {
        b.method(name, move |c| {
            let (a, k) = (key_arg(c, 1)?, key_arg(c, 2)?);
            let v = f(&group(c)?.borrow(), a.as_ref(), k.as_ref()).map_err(java_error)?;
            Ok(c.push(v))
        });
    }
    b.method("setMultiItem", |c| {
        let k = key_arg(c, 1)?;
        group(c)?.borrow_mut().set_multi_item(k.as_ref(), bool_arg(c, 2)).map_err(java_error)?;
        Ok(0)
    });
    b.method("isMultiItem", |c| {
        let k = key_arg(c, 1)?;
        let v = group(c)?.borrow().is_multi_item(k.as_ref()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("getAllLocations", |c| {
        let g = group(c)?;
        let view =
            JavaList::live(move || g.borrow().all_locations().iter().map(|l| body_location_value(Some(l))).collect(), None);
        Ok(c.push(Value::Object(Rc::new(view))))
    });

    let b = expose_class(thread, BODY_LOCATION_CLASS, None);
    // The setters return `this` for chaining.
    type Setter = fn(&mut pz_characters::BodyLocation, Option<Rc<RegistryKey>>);
    let setters: [(&str, Setter); 3] = [
        ("setExclusive", pz_characters::BodyLocation::set_exclusive),
        ("setHideModel", pz_characters::BodyLocation::set_hide_model),
        ("setAltModel", pz_characters::BodyLocation::set_alt_model),
    ];
    for (name, f) in setters {
        b.method(name, move |c| {
            let k = key_arg(c, 1)?;
            f(&mut body_location(c)?.borrow_mut(), k);
            Ok(c.push(c.arg(0)))
        });
    }
    type Test = fn(&pz_characters::BodyLocation, Option<&Rc<RegistryKey>>) -> bool;
    let tests: [(&str, Test); 3] = [
        ("isExclusive", pz_characters::BodyLocation::is_exclusive),
        ("isHideModel", pz_characters::BodyLocation::is_hide_model),
        ("isAltModel", pz_characters::BodyLocation::is_alt_model),
    ];
    for (name, f) in tests {
        b.method(name, move |c| {
            let k = key_arg(c, 1)?;
            let v = f(&body_location(c)?.borrow(), k.as_ref());
            Ok(c.push(v))
        });
    }
    b.method("setMultiItem", |c| {
        body_location(c)?.borrow_mut().multi_item = bool_arg(c, 1);
        Ok(c.push(c.arg(0)))
    });
    b.method("isMultiItem", |c| {
        let v = body_location(c)?.borrow().multi_item;
        Ok(c.push(v))
    });
    b.method("isId", |c| {
        let k = key_arg(c, 1)?;
        let v = body_location(c)?.borrow().is_id(k.as_ref()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("getId", |c| {
        let v = body_location(c)?.borrow().id.clone();
        Ok(c.push(location_value(v.as_ref())))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn sm_with_keys() -> Rc<ScriptManager> {
        let sm = Rc::new(ScriptManager::new());
        for (reg, ids) in [
            ("character_trait", &["t:brave", "t:timid", "t:bold"][..]),
            ("character_profession", &["t:baker", "t:jobless"][..]),
            ("item_body_location", &["t:hat", "t:mask", "t:ring"][..]),
        ] {
            for id in ids {
                sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
            }
        }
        let mut st = sm.statics.items.borrow_mut();
        st.constants.insert("character_trait", "BRAVE", "t:brave");
        st.constants.insert("character_trait", "TIMID", "t:timid");
        st.constants.insert("character_trait", "BOLD", "t:bold");
        st.constants.insert("character_profession", "BAKER", "t:baker");
        st.constants.insert("character_profession", "UNEMPLOYED", "t:jobless");
        st.constants.insert("item_body_location", "HAT", "t:hat");
        st.constants.insert("item_body_location", "MASK", "t:mask");
        st.constants.insert("item_body_location", "RING", "t:ring");
        drop(st);
        sm
    }

    fn setup() -> (Thread, Rc<RefCell<Characters>>) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_core::register(&mut t, None, Rc::new(RefCell::new(pz_core::core::Core::new())));
        let sm = sm_with_keys();
        crate::bind_registry::register(&mut t, sm.clone());
        let chars = Rc::new(RefCell::new(Characters::default()));
        register(&mut t, sm, chars.clone());
        (t, chars)
    }

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    #[test]
    fn trait_and_profession_definitions() {
        let (mut t, _) = setup();
        run(
            &mut t,
            r#"
            local brave = CharacterTraitDefinition.addCharacterTraitDefinition(CharacterTrait.BRAVE, "UI_Brave", 4, "UI_BraveDesc", false)
            local timid = CharacterTraitDefinition.addCharacterTraitDefinition(CharacterTrait.TIMID, "UI_Timid", -2, "", false, true)
            local bold = CharacterTraitDefinition.addCharacterTraitDefinition(CharacterTrait.BOLD, "UI_Bold", 0, nil, true)
            CharacterTraitDefinition.setMutualExclusive(CharacterTrait.BRAVE, CharacterTrait.TIMID)
            bold:addGrantedTrait(CharacterTrait.BRAVE)
            assert(bold:getMutuallyExclusiveTraits():contains(CharacterTrait.TIMID))
            assert(timid:getMutuallyExclusiveTraits():size() == 2)
            -- Live lists write back to their owner.
            local ex = bold:getMutuallyExclusiveTraits()
            ex:add(CharacterTrait.BRAVE)
            assert(bold:getMutuallyExclusiveTraits():size() == 2 and ex:indexOf(CharacterTrait.BRAVE) == 1)
            ex:remove(CharacterTrait.BRAVE)
            assert(bold:getMutuallyExclusiveTraits():size() == 1)
            local rec = bold:getGrantedRecipes()
            rec:add("Make Thing")
            assert(bold:isGrantedRecipe("Make Thing") and bold:getGrantedRecipes():get(0) == "Make Thing")
            assert(not pcall(function() rec:add(1) end) and rec:size() == 1)
            assert(bold:isMutuallyExclusive(timid))
            local list = CharacterTraitDefinition.getTraits()
            assert(list:size() == 3 and list:get(0) == brave and list:get(2) == bold)
            assert(CharacterTraitDefinition.getCharacterTraitDefinition(CharacterTrait.TIMID) == timid)
            assert(brave:getLabel() == "UI_Brave" and brave:getRightLabel() == "-4" and timid:getRightLabel() == "+2")
            assert(timid:getDescription() == nil and timid:isDisabledInMultiplayer() and bold:isFree())
            brave:setDescription("x")
            assert(brave:getDescription() == "x" and brave:getType() == CharacterTrait.BRAVE)
            assert(bold:getGrantedTraits():get(0) == CharacterTrait.BRAVE)

            local p = CharacterProfessionDefinition.addCharacterProfessionDefinition(CharacterProfession.BAKER, "UI_Baker", -4, "", "")
            p:addGrantedTrait(CharacterTrait.BOLD)
            p:addGrantedRecipe("Bread")
            assert(p:isGrantedRecipe("Bread") and p:hasGrantedRecipes())
            assert(p:getDescription() == "" and p:getRightLabel() == "+4")
            assert(CharacterProfessionDefinition.getProfessions():get(0) == p)
            assert(CharacterProfessionDefinition.getCharacterProfessionDefinition(CharacterProfession.BAKER) == p)
            CharacterTraitDefinition.reset()
            assert(CharacterTraitDefinition.getTraits():isEmpty())
            "#,
        )
        .unwrap();
        let e = run(&mut t, "CharacterTraitDefinition.setMutualExclusive(CharacterTrait.BRAVE, CharacterTrait.TIMID)").unwrap_err();
        assert!(e.contains("NullPointerException"), "{e}");
    }

    #[test]
    fn traits_observations_and_survivors() {
        let (mut t, chars) = setup();
        // `getHairModel` keeps a known style (invented table).
        let pop = pz_characters::population::Population {
            hair_styles: Some(
                pz_characters::population::HairStyles::from_xml(b"<hairStyles><male><name>Bald</name><model/></male></hairStyles>")
                    .unwrap(),
            ),
            ..Default::default()
        };
        crate::bind_appearance::install_population(Rc::new(RefCell::new(pop)));
        run(
            &mut t,
            r#"
            local ct = CharacterTraits.new()
            assert(not ct:get(CharacterTrait.BRAVE))
            assert(ct:set(CharacterTrait.BRAVE, true))
            ct:add(CharacterTrait.TIMID)
            assert(ct:getKnownTraits():size() == 2)

            ObservationFactory.addObservation("Calm", "Calm", "d")
            ObservationFactory.addObservation("Jumpy", "Jumpy", "e")
            ObservationFactory.setMutualExclusive("Calm", "Jumpy")
            local o = ObservationFactory.getObservation("Calm")
            assert(o == ObservationFactory.getObservation("Calm") and o:getLabel() == "Calm" and o:getRightLabel() == nil)
            assert(Observation.new("a", "b", "c"):getTraitID() == "a")

            SurvivorFactory.addFemaleForename("Ann")
            SurvivorFactory.addMaleForename("Bob")
            SurvivorFactory.addSurname("Cole")
            assert(SurvivorFactory.Surnames:size() == 1)
            local d = SurvivorFactory.CreateSurvivor()
            assert(d:getSurname() == "SurvivorSurname_Cole")
            assert(d:getCharacterProfession() == CharacterProfession.UNEMPLOYED)
            local m = SurvivorFactory.CreateSurvivor(SurvivorType.Friendly, false)
            assert(not m:isFemale() and m:getTorso() == "Male" and m:getType() == SurvivorType.Friendly)
            assert(m:getID() == d:getID() + 1 and SurvivorDesc.getIDCount() == m:getID() + 1)
            m:addObservation("Calm")
            assert(m:hasObservation("Calm") and m:getObservations():get(0) == o)
            m:getExtras():add("extra")
            assert(m:getExtras():contains("extra"))
            m:getExtras():clear()
            assert(m:getExtras():isEmpty())
            assert(m:getHumanVisual() == m:getHumanVisual())
            m:getHumanVisual():setHairModel("Bald")
            assert(m:getHumanVisual():getHairModel() == "Bald")
            m:getHumanVisual():clear()
            -- Cleared: the lazy getter draws a style again (the only one).
            assert(m:getHumanVisual():getHairModel() == "Bald" and m:getHumanVisual():getBodyHairIndex() == -1)
            assert(m:getWornItems():isEmpty())
            m:setCharacterProfession(CharacterProfession.BAKER)
            assert(m:isCharacterProfession(CharacterProfession.BAKER))
            SurvivorFactory.randomName(m)
            assert(m:getFullname() == "SurvivorName_Bob SurvivorSurname_Cole")
            SurvivorDesc.addHairColor(ColorInfo.new(0.1, 0.2, 0.3, 1))
            local copy = SurvivorDesc.new(m)
            assert(copy:getID() == 0 and copy:getForename() == "SurvivorName_Bob")
            "#,
        )
        .unwrap();
        assert_eq!(chars.borrow().survivor_desc.hair_common_colors.len(), 1);
        let e = run(&mut t, "SurvivorFactory.CreateSurvivor():setWornItem(nil, {})").unwrap_err();
        assert!(e.contains("NullPointerException"), "{e}");
        run(&mut t, "SurvivorFactory.Reset()").unwrap();
        let e = run(&mut t, "SurvivorFactory.getRandomSurname()").unwrap_err();
        assert!(e.contains("IndexOutOfBoundsException") || e.contains("out of bounds"), "{e}");
    }

    #[test]
    fn body_locations() {
        let (mut t, chars) = setup();
        run(
            &mut t,
            r#"
            local g = BodyLocations.getGroup("Human")
            assert(g == BodyLocations.getGroup("Human") and g:getId() == "Human")
            assert(BodyLocations.getAllGroups():size() == 1)
            assert(not pcall(function() BodyLocations.getAllGroups():clear() end))
            local hat = g:getOrCreateLocation(ItemBodyLocation.HAT)
            assert(hat == g:getOrCreateLocation(ItemBodyLocation.HAT) and hat:getId() == ItemBodyLocation.HAT)
            g:getOrCreateLocation(ItemBodyLocation.MASK)
            g:getOrCreateLocation(ItemBodyLocation.RING)
            assert(g:size() == 3 and g:indexOf(ItemBodyLocation.RING) == 2 and g:getLocationByIndex(5) == nil)
            g:setExclusive(ItemBodyLocation.HAT, ItemBodyLocation.MASK)
            g:setHideModel(ItemBodyLocation.HAT, ItemBodyLocation.RING)
            g:setAltModel(ItemBodyLocation.MASK, ItemBodyLocation.HAT)
            g:setMultiItem(ItemBodyLocation.RING, true)
            assert(g:isExclusive(ItemBodyLocation.MASK, ItemBodyLocation.HAT) and hat:isExclusive(ItemBodyLocation.MASK))
            assert(g:isHideModel(ItemBodyLocation.HAT, ItemBodyLocation.RING) and not hat:isHideModel(ItemBodyLocation.MASK))
            assert(g:isAltModel(ItemBodyLocation.MASK, ItemBodyLocation.HAT) and g:isMultiItem(ItemBodyLocation.RING))
            assert(hat:setMultiItem(true) == hat and hat:isMultiItem() and hat:isId(ItemBodyLocation.HAT))
            g:moveLocationToIndex(ItemBodyLocation.RING, 0)
            assert(g:getAllLocations():get(0):getId() == ItemBodyLocation.RING)
            assert(ItemBodyLocation.HAT:getTranslationName() == "t:hat" and tostring(ItemBodyLocation.HAT) == "t:hat")
            assert(SurvivorDesc.new():getWornItems():getBodyLocationGroup() == g)
            "#,
        )
        .unwrap();
        assert_eq!(chars.borrow().body_locations.all_groups()[0].borrow().size(), 3);
        let e = run(&mut t, "BodyLocations.getGroup('Human'):setExclusive(nil, ItemBodyLocation.HAT)").unwrap_err();
        assert!(e.contains("NullPointerException"), "{e}");
        let e = run(&mut t, "BodyLocations.getGroup('')").unwrap_err();
        assert!(e.contains("id is empty"), "{e}");
    }

    #[test]
    fn common_hair_colours_and_lazy_visual() {
        let (mut t, chars) = setup();
        install_characters(chars.clone());
        crate::bind_misc_classes::register(&mut t, None);
        let mut pop = pz_characters::population::Population::default();
        pop.pop_template.female_skins = vec!["SkinA".into(), "SkinB".into(), "SkinC".into()];
        crate::bind_appearance::install_population(Rc::new(RefCell::new(pop)));
        pz_core::random::outfit_rng::set_seed(77);
        let (colour, skin) = (pz_core::random::outfit_rng::next(2), pz_core::random::outfit_rng::next(3));
        pz_core::random::outfit_rng::set_seed(77);
        run(
            &mut t,
            &format!(
                r#"
            SurvivorDesc.addHairColor(ColorInfo.new(0.5, 0.25, 0.125, 1))
            SurvivorDesc.addHairColor(ColorInfo.new(0.75, 1.5, 0, 1))
            local d = SurvivorDesc.new()
            d:setFemale(true)
            local l = d:getCommonHairColor()
            assert(l == SurvivorDesc.HairCommonColors and l:size() == 2)
            assert(l:get(0):getGreenFloat() == 0.25 and l:get(1):getGreenFloat() == 1)
            local hv = d:getHumanVisual()
            assert(hv:getSkinTextureIndex() == -1)
            local c = hv:getHairColor()
            assert(c:getRedFloat() == l:get({colour}):getRedFloat())
            assert(hv:getNaturalBeardColor():getBlueFloat() == c:getBlueFloat())
            assert(hv:getSkinTexture() == ({{"SkinA", "SkinB", "SkinC"}})[{skin} + 1])
            assert(hv:getSkinTextureIndex() == {skin})
            hv:setHairColor(ImmutableColor.new(0.1, 0.2, 0.3, 1))
            assert(hv:getHairColor():getGreenFloat() == hv:getHairColor():getGreenFloat())
            assert(tostring(ImmutableColor.new(2, 0.5, -1)) == "ImmutableColor (1.0,0.5,0.0,1.0)")
            assert(ImmutableColor.new(0.5, 0.5, 0.5, 1):scale(4):getRedFloat() == 1)
            SurvivorFactory.Reset()
            assert(l:size() == 0)
            "#
            ),
        )
        .unwrap();
        assert!(chars.borrow().survivor_desc.hair_common_colors.is_empty());
    }
}
