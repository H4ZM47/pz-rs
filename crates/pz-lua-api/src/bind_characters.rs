//! Lua bindings for the character classes the main menu and character
//! creation use: `CharacterTraitDefinition`, `CharacterProfessionDefinition`
//! (state in `pz-scripts`), `CharacterTraits`, `ObservationFactory` and its
//! `Observation`, `SurvivorFactory` (with `SurvivorType`) and `SurvivorDesc`
//! with its `HumanVisual` and `WornItems` (state in `pz-characters`).
//!
//! The statics live in a shared [`Characters`] ([`shared_characters`],
//! [`install_characters`]) next to the shared `ScriptManager`
//! ([`crate::bind_registry::shared_script_manager`]).
//!
//! Lists the original returns as copies are `java.util.ArrayList`
//! snapshots; lists it returns live (`getMutuallyExclusiveTraits`,
//! `getGrantedRecipes`, `getExtras`, `getObservations`, the name lists) are
//! [`ListView`]s.
//!
//! Not exposed yet: members taking or returning `PerkFactory.Perk`
//! (`getXpBoosts`, `addXPBoost`, `getXPBoostMap`), textures, `HashMap`/
//! `LinkedHashMap` statics (`characterTraitDefinitions`, `observationMap`,
//! `CharacterTraits.getTraits`), boxed-number constants of
//! `CharacterTraits`, colour objects (`ImmutableColor`, `Color`), outfits and
//! item visuals, Java arrays (`CreateFamily`, `CreateSurvivorGroup`), saving
//! and loading. TODO(PerkFactory, Texture, ImmutableColor, Outfits, saves).

use crate::bind_core::LuaColorInfo;
use crate::bind_java_util::JavaList;
use crate::bind_registry::{
    java_error, key_arg, key_value, shared_script_manager, CHARACTER_PROFESSION_CLASS, CHARACTER_TRAIT_CLASS,
};
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this, JavaEnum, ListView};
use pz_characters::survivor::{meet, set_torso};
use pz_characters::{
    CharacterGender, CharacterTraits, Characters, ItemIdentity, Observation, ObservationRef, SurvivorDesc, SurvivorType,
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
pub const CHARACTER_GENDER_CLASS: &str = "zombie.characters.CharacterGender";
pub const ITEM_BODY_LOCATION_CLASS: &str = "zombie.scripting.objects.ItemBodyLocation";

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<Characters>>>> = const { RefCell::new(None) };
    static HANDLES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
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

fn trait_def_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<CharacterTraitDefinitionRef>> {
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

fn desc_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<RefCell<LuaSurvivorDesc>>> {
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

/// A `SurvivorDesc`'s `WornItems`.
pub struct WornItemsObject(pub Rc<RefCell<LuaSurvivorDesc>>);

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
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    register(thread, shared_script_manager(), shared_characters());
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
        let (r, w) = (d.clone(), d);
        let view = ListView::new(move || r.borrow().granted_recipes.iter().map(|s| Value::from(s.as_str())).collect())
            .with_clear(move || w.borrow_mut().granted_recipes.clear());
        Ok(c.push(Value::Object(Rc::new(view))))
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
        let view = ListView::new(move || r.borrow().mutually_exclusive_traits.iter().map(|k| trait_value(k.as_ref())).collect())
            .with_clear(move || w.borrow_mut().mutually_exclusive_traits.clear());
        Ok(c.push(Value::Object(Rc::new(view))))
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
        let (r, w) = (d.clone(), d);
        let view = ListView::new(move || r.borrow().granted_recipes.iter().map(|s| Value::from(s.as_str())).collect())
            .with_clear(move || w.borrow_mut().granted_recipes.clear());
        Ok(c.push(Value::Object(Rc::new(view))))
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

fn names_view(chars: &Rc<RefCell<Characters>>, pick: fn(&Characters) -> &Vec<String>) -> Value {
    let ch = chars.clone();
    Value::Object(Rc::new(ListView::new(move || {
        pick(&ch.borrow()).iter().map(|s| Value::from(s.as_str())).collect()
    })))
}

fn register_survivor_factory(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, SURVIVOR_FACTORY_CLASS, None);
    b.field("FemaleForenames", names_view(chars, |c| &c.survivor_factory.female_forenames));
    b.field("MaleForenames", names_view(chars, |c| &c.survivor_factory.male_forenames));
    b.field("Surnames", names_view(chars, |c| &c.survivor_factory.surnames));
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
        let (r, w) = (d.clone(), d);
        let view = ListView::new(move || r.borrow().extra.iter().map(|s| Value::from(s.as_str())).collect())
            .with_clear(move || w.borrow_mut().extra.clear());
        Ok(c.push(Value::Object(Rc::new(view))))
    });
    b.method("getObservations", |c| {
        let d = dobj(c, |o| o.desc.clone())?;
        let (r, w) = (d.clone(), d);
        let view = ListView::new(move || r.borrow().observations.iter().map(|o| observation_value(Some(o))).collect())
            .with_clear(move || w.borrow_mut().observations.clear());
        Ok(c.push(Value::Object(Rc::new(view))))
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
            o.worn_items.borrow_mut().get_or_insert_with(|| Value::Object(Rc::new(WornItemsObject(o.desc.clone())))).clone()
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

fn register_human_visual(thread: &mut Thread) {
    let b = expose_class(thread, HUMAN_VISUAL_CLASS, None);
    b.method("isFemale", |c| {
        let v = visual(c, |d| d.is_female())?;
        Ok(c.push(v))
    });
    b.method("isZombie", |c| Ok(c.push(false)));
    b.method("isSkeleton", |c| Ok(c.push(false)));
    b.method("getSkinTextureIndex", |c| {
        let v = visual(c, |d| d.human_visual.skin_texture)?;
        Ok(c.push(v))
    });
    b.method("setSkinTextureIndex", |c| {
        let v = int_arg(c, 1);
        visual(c, |d| d.human_visual.skin_texture = v)?;
        Ok(0)
    });
    b.method("getBodyHairIndex", |c| {
        let v = visual(c, |d| d.human_visual.body_hair)?;
        Ok(c.push(v))
    });
    b.method("setBodyHairIndex", |c| {
        let v = int_arg(c, 1);
        visual(c, |d| d.human_visual.body_hair = v)?;
        Ok(0)
    });
    b.method("setSkinTextureName", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.skin_texture_name = v)?;
        Ok(0)
    });
    // TODO(HairStyles): an unknown style is replaced by a random one.
    b.method("getHairModel", |c| {
        let v = visual(c, |d| opt_value(d.human_visual.hair_model.as_deref()))?;
        Ok(c.push(v))
    });
    b.method("setHairModel", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.hair_model = v)?;
        Ok(0)
    });
    // TODO(BeardStyles): an unknown style is replaced by a random one.
    b.method("getBeardModel", |c| {
        let v = visual(c, |d| {
            if d.is_female() {
                d.human_visual.beard_model = None;
            }
            opt_value(d.human_visual.beard_model.as_deref())
        })?;
        Ok(c.push(v))
    });
    b.method("setBeardModel", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.beard_model = v)?;
        Ok(0)
    });
    b.method("getNonAttachedHair", |c| {
        let v = visual(c, |d| opt_value(d.human_visual.non_attached_hair.as_deref()))?;
        Ok(c.push(v))
    });
    b.method("setNonAttachedHair", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.set_non_attached_hair(v.as_deref()))?;
        Ok(0)
    });
    b.method("setForceModelScript", |c| {
        let v = opt_string(c, 1);
        visual(c, |d| d.human_visual.force_model_script = v)?;
        Ok(0)
    });
    b.method("clear", |c| {
        visual(c, |d| d.human_visual.clear())?;
        Ok(0)
    });
    b.method("lerp", |c| {
        let v = pz_characters::HumanVisual::lerp(num_arg(c, 1) as f32, num_arg(c, 2) as f32, num_arg(c, 3) as f32);
        Ok(c.push(v as f64))
    });
    b.method("hasBodyVisualFromItemType", |c| {
        let t = opt_string(c, 1).ok_or_else(npe)?;
        let v = visual(c, |d| d.human_visual.has_body_visual_from_item_type(&t))?;
        Ok(c.push(v))
    });
    // TODO(ItemVisual): returns the removed `ItemVisual`.
    b.method("removeBodyVisualFromItemType", |c| {
        let t = opt_string(c, 1).ok_or_else(npe)?;
        visual(c, |d| d.human_visual.remove_body_visual_from_item_type(&t))?;
        Ok(c.push(Value::Nil))
    });
}

fn worn<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut LuaSurvivorDesc) -> R) -> LuaResult<R> {
    with_this::<WornItemsObject, _>(c, |o| f(&mut o.0.borrow_mut()))
}

fn location_value(k: Option<&Rc<RegistryKey>>) -> Value {
    key_value(ITEM_BODY_LOCATION_CLASS, k)
}

fn register_worn_items(thread: &mut Thread) {
    let b = expose_class(thread, WORN_ITEMS_CLASS, None);
    b.method("size", |c| {
        let v = worn(c, |d| d.worn_items.size() as i32)?;
        Ok(c.push(v))
    });
    b.method("isEmpty", |c| {
        let v = worn(c, |d| d.worn_items.is_empty())?;
        Ok(c.push(v))
    });
    b.method("clear", |c| {
        worn(c, |d| d.worn_items.clear())?;
        Ok(0)
    });
    b.method("get", |c| {
        let i = int_arg(c, 1);
        let w = worn(c, |d| d.worn_items.get(i).cloned())?.map_err(java_error)?;
        Ok(c.push(Value::Object(Rc::new(WornItemObject { location: w.location, item: w.item.0 }))))
    });
    b.method("getItem", |c| {
        let loc = key_arg(c, 1)?;
        let item = worn(c, |d| d.worn_items.item(loc.as_ref()))?.map_err(java_error)?;
        Ok(c.push(item.map(|i| i.0).unwrap_or_default()))
    });
    b.method("getItemByIndex", |c| {
        let i = int_arg(c, 1);
        let item = worn(c, |d| d.worn_items.item_by_index(i))?;
        Ok(c.push(item.map(|i| i.0).unwrap_or_default()))
    });
    b.method("setItem", |c| {
        let (loc, item) = (key_arg(c, 1)?, c.arg(2));
        let item = (!item.is_nil()).then_some(LuaItem(item));
        worn(c, |d| d.worn_items.set_item(loc, item))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("remove", |c| {
        let item = LuaItem(c.arg(1));
        worn(c, |d| d.worn_items.remove(&item))?;
        Ok(0)
    });
    b.method("contains", |c| {
        let item = LuaItem(c.arg(1));
        let v = worn(c, |d| d.worn_items.contains(&item))?;
        Ok(c.push(v))
    });
    b.method("getLocation", |c| {
        let item = LuaItem(c.arg(1));
        let k = worn(c, |d| d.worn_items.location(&item))?;
        Ok(c.push(location_value(k.as_ref())))
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

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn sm_with_keys() -> Rc<ScriptManager> {
        let sm = Rc::new(ScriptManager::new());
        for (reg, ids) in [
            ("character_trait", &["t:brave", "t:timid", "t:bold"][..]),
            ("character_profession", &["t:baker", "t:jobless"][..]),
            ("item_body_location", &["t:hat"][..]),
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
        drop(st);
        sm
    }

    fn setup() -> (Thread, Rc<RefCell<Characters>>) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
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
            m:getExtras():clear()
            assert(m:getHumanVisual() == m:getHumanVisual())
            m:getHumanVisual():setHairModel("Bald")
            assert(m:getHumanVisual():getHairModel() == "Bald")
            m:getHumanVisual():clear()
            assert(m:getHumanVisual():getHairModel() == nil and m:getHumanVisual():getBodyHairIndex() == -1)
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
}
