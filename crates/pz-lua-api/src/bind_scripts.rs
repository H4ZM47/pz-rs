//! Lua bindings for `zombie.scripting`: `ScriptManager` (with the
//! `getScriptManager()` global and `ScriptManager.instance`), `ScriptType`,
//! `ScriptModule`, `BaseScriptObject` and the script classes
//! (`Item`, the recipe classes, models, vehicles, craft recipes, sounds and
//! the definition scripts), over the shared ScriptManager
//! ([`crate::bind_registry::shared_script_manager`]).
//!
//! Every script, and every object inside one that Lua can reach
//! (`VehicleScript.Part`, `Recipe.Source`, a craft recipe's `InputScript`,
//! ...), is a [`ScriptHandle`]: the owning script plus a projection to the
//! object. Handles are canonical (one Lua object per script, or per path
//! inside one) so Lua `==` and table keys behave like Java references.
//!
//! Lists the original returns are `java.util.ArrayList` snapshots.
//!
//! Not exposed yet: members taking or returning characters, players and
//! inventory items (`InstanceItem`, `checkAutoLearn`, `isUnwanted`, ...),
//! textures, `ScriptType.Flags`/`ScriptLoadMode` objects (`hasFlag`,
//! `hasFlags`, `setVerbose`), `ScriptManager.Load`
//! and `searchFolders` (file system), `resolveGetItemTypes` (mutates Java
//! lists from Lua callbacks). TODO(inventory, IsoGameCharacter, Texture).

mod crafting;
mod item;
mod models;
mod recipes;
mod vehicles;

use crate::bind_core::JavaList;
use crate::bind_registry::{java_error, key_arg, key_value};
use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, global, int_arg, num_arg, str_arg, this, ClassBuilder, JavaEnum,
    ListView,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::entity::components::crafting::{CraftRecipe, CraftRecipeComponentScript};
use pz_scripts::objects::entity::{ComponentScript, ComponentType, GameEntityScript};
use pz_scripts::objects::items::Item;
use pz_scripts::{BaseScriptObject, ScriptLoadMode, ScriptManager, ScriptModule, ScriptObject, ScriptRef, ScriptType};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const SCRIPT_MANAGER_CLASS: &str = "zombie.scripting.ScriptManager";
pub const SCRIPT_TYPE_CLASS: &str = "zombie.scripting.ScriptType";
pub const SCRIPT_MODULE_CLASS: &str = "zombie.scripting.objects.ScriptModule";
pub const BASE_SCRIPT_OBJECT_CLASS: &str = "zombie.scripting.objects.BaseScriptObject";
pub const GAME_ENTITY_SCRIPT_CLASS: &str = "zombie.scripting.entity.GameEntityScript";
pub const COMPONENT_SCRIPT_CLASS: &str = "zombie.scripting.entity.ComponentScript";
pub const ITEM_CLASS: &str = "zombie.scripting.objects.Item";
pub const CRAFT_RECIPE_CLASS: &str = "zombie.scripting.entity.components.crafting.CraftRecipe";

/// Script classes (the `ScriptObject::class_name`s) with their superclass.
const SCRIPT_CLASSES: &[(&str, &str)] = &[
    (GAME_ENTITY_SCRIPT_CLASS, BASE_SCRIPT_OBJECT_CLASS),
    (ITEM_CLASS, GAME_ENTITY_SCRIPT_CLASS),
    ("zombie.scripting.entity.GameEntityTemplate", BASE_SCRIPT_OBJECT_CLASS),
    (CRAFT_RECIPE_CLASS, BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.itemConfig.ItemConfig", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.core.skinnedmodel.runtime.RuntimeAnimationScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.iso.SpriteModel", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.AnimationsMesh", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.CharacterProfessionDefinitionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.CharacterTraitDefinitionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.ClockScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.EnergyDefinitionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.EvolvedRecipe", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.Fixing", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.FluidDefinitionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.FluidFilterScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.GameSoundScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.ItemFilterScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.MannequinScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.ModelScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.PhysicsHitReactionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.PhysicsShapeScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.RagdollScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.Recipe", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.SoundTimelineScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.StringListScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.TimedActionScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.UniqueRecipe", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.VehicleScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.VehicleTemplate", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.XuiColorsScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.XuiConfigScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.XuiLayoutScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.scripting.objects.XuiSkinScript", BASE_SCRIPT_OBJECT_CLASS),
    ("zombie.vehicles.VehicleEngineRPM", BASE_SCRIPT_OBJECT_CLASS),
];

thread_local! {
    static HANDLES: RefCell<HashMap<(usize, String), Value>> = RefCell::new(HashMap::new());
    static MODULES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

// ---- handles ----

type Get = Rc<dyn for<'a> Fn(&'a dyn Any) -> Option<&'a dyn Any>>;
type GetMut = Rc<dyn for<'a> Fn(&'a mut dyn Any) -> Option<&'a mut dyn Any>>;

/// Where a handle points inside its script.
#[derive(Clone)]
struct Path {
    key: String,
    get: Get,
    get_mut: Option<GetMut>,
}

/// A script, or an object inside one, exposed to Lua.
#[derive(Clone)]
pub struct ScriptHandle {
    pub root: ScriptRef,
    class: &'static str,
    path: Option<Path>,
    /// The handle this one was reached from.
    parent: Option<Rc<ScriptHandle>>,
}

impl LuaObject for ScriptHandle {
    fn class_name(&self) -> &str {
        self.class
    }
    fn to_lua_string(&self) -> String {
        if self.class == ITEM_CLASS {
            if let Ok(Some(s)) = self.read_any(|a| cast::<Item>(a).map(Item::to_java_string)) {
                return s;
            }
        }
        format!("{}@{:x}", self.class, self.root.as_ptr() as usize)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        match self.class {
            ITEM_CLASS => &[GAME_ENTITY_SCRIPT_CLASS, BASE_SCRIPT_OBJECT_CLASS],
            c if c.starts_with("zombie.scripting.entity.components.") && c.ends_with("Script") => {
                &[COMPONENT_SCRIPT_CLASS, BASE_SCRIPT_OBJECT_CLASS]
            }
            _ if self.path.is_none() => &[BASE_SCRIPT_OBJECT_CLASS],
            _ => &[],
        }
    }
}

/// `T` as found at `a`: the object itself, a component behind its
/// `Box<dyn ComponentScript>`, or an `Item`'s `GameEntityScript` part.
pub fn cast<T: 'static>(a: &dyn Any) -> Option<&T> {
    if let Some(t) = a.downcast_ref::<T>() {
        return Some(t);
    }
    if let Some(c) = a.downcast_ref::<Box<dyn ComponentScript>>() {
        return c.as_any().downcast_ref::<T>();
    }
    if let Some(i) = a.downcast_ref::<Item>() {
        return (&i.entity as &dyn Any).downcast_ref::<T>();
    }
    None
}

pub fn cast_mut<T: 'static>(a: &mut dyn Any) -> Option<&mut T> {
    if a.is::<T>() {
        return a.downcast_mut::<T>();
    }
    if a.is::<Box<dyn ComponentScript>>() {
        return a.downcast_mut::<Box<dyn ComponentScript>>()?.as_any_mut().downcast_mut::<T>();
    }
    if a.is::<Item>() {
        return (&mut a.downcast_mut::<Item>()?.entity as &mut dyn Any).downcast_mut::<T>();
    }
    None
}

fn busy() -> LuaError {
    LuaError::runtime("script object is being modified")
}

fn gone() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

fn wrong_type<T>(class: &str) -> LuaError {
    LuaError::runtime(format!("{class} is not a {}", std::any::type_name::<T>()))
}

/// Java's `NullPointerException`.
pub fn npe() -> LuaError {
    gone()
}

impl ScriptHandle {
    fn new(root: ScriptRef, class: &'static str) -> Self {
        ScriptHandle { root, class, path: None, parent: None }
    }

    pub fn is_root(&self) -> bool {
        self.path.is_none()
    }

    pub fn parent(&self) -> Option<&ScriptHandle> {
        self.parent.as_deref()
    }

    /// Address of the object the handle points at.
    pub fn target_ptr(&self) -> LuaResult<usize> {
        self.read_any(|a| a as *const dyn Any as *const () as usize)
    }

    fn key(&self) -> (usize, String) {
        (self.root.as_ptr() as usize, self.path.as_ref().map(|p| p.key.clone()).unwrap_or_default())
    }

    fn read_any<R>(&self, f: impl FnOnce(&dyn Any) -> R) -> LuaResult<R> {
        let root = self.root.try_borrow().map_err(|_| busy())?;
        let any: &dyn Any = &*root;
        let target = match &self.path {
            None => any,
            Some(p) => (p.get)(any).ok_or_else(gone)?,
        };
        Ok(f(target))
    }

    fn write_any<R>(&self, f: impl FnOnce(&mut dyn Any) -> R) -> LuaResult<R> {
        let mut root = self.root.try_borrow_mut().map_err(|_| busy())?;
        let any: &mut dyn Any = &mut *root;
        let target = match &self.path {
            None => any,
            Some(Path { get_mut: Some(g), .. }) => g(any).ok_or_else(gone)?,
            Some(_) => return Err(LuaError::runtime(format!("{} is read-only here", self.class))),
        };
        Ok(f(target))
    }

    /// Run `f` on the object as a `T`.
    pub fn read<T: 'static, R>(&self, f: impl FnOnce(&T) -> R) -> LuaResult<R> {
        self.read_any(|a| cast::<T>(a).map(f))?.ok_or_else(|| wrong_type::<T>(self.class))
    }

    pub fn write<T: 'static, R>(&self, f: impl FnOnce(&mut T) -> R) -> LuaResult<R> {
        self.write_any(|a| cast_mut::<T>(a).map(f))?.ok_or_else(|| wrong_type::<T>(self.class))
    }

    /// Run `f` on the script the handle is (a root script or an embedded
    /// craft recipe).
    pub fn script<R>(&self, f: impl FnOnce(&dyn ScriptObject) -> R) -> LuaResult<R> {
        if self.path.is_none() {
            let root = self.root.try_borrow().map_err(|_| busy())?;
            return Ok(f(&*root));
        }
        self.read_any(|a| cast::<CraftRecipe>(a).map(|r| f(r)))?
            .ok_or_else(|| wrong_type::<BaseScriptObject>(self.class))
    }

    pub fn script_mut<R>(&self, f: impl FnOnce(&mut dyn ScriptObject) -> R) -> LuaResult<R> {
        if self.path.is_none() {
            let mut root = self.root.try_borrow_mut().map_err(|_| busy())?;
            return Ok(f(&mut *root));
        }
        self.write_any(|a| cast_mut::<CraftRecipe>(a).map(|r| f(r)))?
            .ok_or_else(|| wrong_type::<BaseScriptObject>(self.class))
    }

    /// The `BaseScriptObject` state of a script or component.
    pub fn base<R>(&self, f: impl FnOnce(&BaseScriptObject) -> R) -> LuaResult<R> {
        if self.path.is_none() {
            let root = self.root.try_borrow().map_err(|_| busy())?;
            return Ok(f(root.base()));
        }
        self.read_any(|a| {
            if let Some(r) = cast::<CraftRecipe>(a) {
                Some(f(r.base()))
            } else {
                a.downcast_ref::<Box<dyn ComponentScript>>().map(|c| f(&c.component().base))
            }
        })?
        .ok_or_else(|| wrong_type::<BaseScriptObject>(self.class))
    }

    pub fn base_mut<R>(&self, f: impl FnOnce(&mut BaseScriptObject) -> R) -> LuaResult<R> {
        if self.path.is_none() {
            let mut root = self.root.try_borrow_mut().map_err(|_| busy())?;
            return Ok(f(root.base_mut()));
        }
        self.write_any(|a| {
            if a.is::<Box<dyn ComponentScript>>() {
                return a.downcast_mut::<Box<dyn ComponentScript>>().map(|c| f(&mut c.component_mut().base));
            }
            cast_mut::<CraftRecipe>(a).map(|r| f(r.base_mut()))
        })?
        .ok_or_else(|| wrong_type::<BaseScriptObject>(self.class))
    }

    fn sub(&self, key: &str, get: Get, get_mut: Option<GetMut>, class: &'static str) -> Value {
        let parent = self.path.clone();
        let key = match &parent {
            Some(p) => format!("{}/{key}", p.key),
            None => key.to_string(),
        };
        let (pg, pgm) = (parent.as_ref().map(|p| p.get.clone()), parent.as_ref().map(|p| p.get_mut.clone()));
        let full_get: Get = Rc::new(move |a: &dyn Any| {
            let p = match &pg {
                Some(g) => g(a)?,
                None => a,
            };
            get(p)
        });
        let full_get_mut: Option<GetMut> = match (get_mut, pgm) {
            (Some(gm), None) => Some(gm),
            (Some(gm), Some(Some(pgm))) => Some(Rc::new(move |a: &mut dyn Any| gm(pgm(a)?))),
            _ => None,
        };
        let h = ScriptHandle {
            root: self.root.clone(),
            class,
            path: Some(Path { key, get: full_get, get_mut: full_get_mut }),
            parent: Some(Rc::new(self.clone())),
        };
        canonical(h)
    }

    /// The object `f` finds inside this one (read-only).
    pub fn child<P: 'static, C: 'static>(
        &self,
        class: &'static str,
        key: &str,
        f: impl Fn(&P) -> Option<&C> + 'static,
    ) -> Value {
        let get: Get = Rc::new(move |a: &dyn Any| f(cast::<P>(a)?).map(|c| c as &dyn Any));
        self.sub(key, get, None, class)
    }

    /// The object `f`/`fm` find inside this one, writable through `fm`.
    pub fn child_mut<P: 'static, C: 'static>(
        &self,
        class: &'static str,
        key: &str,
        f: impl Fn(&P) -> Option<&C> + 'static,
        fm: impl Fn(&mut P) -> Option<&mut C> + 'static,
    ) -> Value {
        let get: Get = Rc::new(move |a: &dyn Any| f(cast::<P>(a)?).map(|c| c as &dyn Any));
        let get_mut: GetMut = Rc::new(move |a: &mut dyn Any| fm(cast_mut::<P>(a)?).map(|c| c as &mut dyn Any));
        self.sub(key, get, Some(get_mut), class)
    }
}

pub(crate) fn canonical(h: ScriptHandle) -> Value {
    let key = h.key();
    HANDLES.with(|m| m.borrow_mut().entry(key).or_insert_with(|| Value::Object(Rc::new(h))).clone())
}

/// The Lua class of a script object.
fn class_of(s: &dyn ScriptObject) -> &'static str {
    match s.class_name() {
        "GenericScript" => BASE_SCRIPT_OBJECT_CLASS,
        c => c,
    }
}

/// The canonical Lua object for a script (nil for `None`).
pub fn script_value(r: Option<&ScriptRef>) -> Value {
    let Some(r) = r else { return Value::Nil };
    let key = (r.as_ptr() as usize, String::new());
    if let Some(v) = HANDLES.with(|m| m.borrow().get(&key).cloned()) {
        return v;
    }
    let class = match r.try_borrow() {
        Ok(s) => class_of(&*s),
        Err(_) => BASE_SCRIPT_OBJECT_CLASS,
    };
    canonical(ScriptHandle::new(r.clone(), class))
}

/// A `java.util.ArrayList` holding `items`.
pub fn list_value(items: Vec<Value>) -> Value {
    Value::Object(Rc::new(JavaList(items)))
}

/// An `ArrayList` of scripts.
pub fn script_list<'a>(scripts: impl IntoIterator<Item = &'a ScriptRef>) -> Value {
    list_value(scripts.into_iter().map(|s| script_value(Some(s))).collect())
}

/// An `ArrayList<String>`.
pub fn string_list<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Value {
    list_value(items.into_iter().map(|s| Value::from(s.as_ref())).collect())
}

/// A list field that may be `null`.
pub fn opt_string_list<S: AsRef<str>>(items: Option<impl IntoIterator<Item = S>>) -> Value {
    items.map(string_list).unwrap_or_default()
}

pub fn this_handle(c: &NativeCall<'_>) -> LuaResult<ScriptHandle> {
    let o = this::<ScriptHandle>(c)?;
    Ok(o.as_any().downcast_ref::<ScriptHandle>().unwrap().clone())
}

/// A script argument (nil is `null`).
pub fn handle_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ScriptHandle>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ScriptHandle>() {
            Some(h) => Ok(Some(h.clone())),
            None => Err(LuaError::runtime(format!("Expected a script object but got {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected a script object but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

/// A read-only `java.util.HashMap` snapshot (entries in the map's order).
pub struct JavaMap(pub Vec<(Value, Value)>);

pub const HASH_MAP_CLASS: &str = "java.util.HashMap";

impl LuaObject for JavaMap {
    fn class_name(&self) -> &str {
        HASH_MAP_CLASS
    }
    fn to_lua_string(&self) -> String {
        let parts: Vec<String> = self
            .0
            .iter()
            .map(|(k, v)| format!("{}={}", pz_kahlua::exec::java_value_of(k), pz_kahlua::exec::java_value_of(v)))
            .collect();
        format!("{{{}}}", parts.join(", "))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn register_map(thread: &mut Thread) {
    if !thread.class_metatables.get_str(HASH_MAP_CLASS).is_nil() {
        return;
    }
    let b = expose_class(thread, HASH_MAP_CLASS, None);
    fn map<R>(c: &NativeCall<'_>, f: impl FnOnce(&[(Value, Value)]) -> R) -> LuaResult<R> {
        let o = this::<JavaMap>(c)?;
        Ok(f(&o.as_any().downcast_ref::<JavaMap>().unwrap().0))
    }
    b.method("get", |c| {
        let k = c.arg(1);
        let v = map(c, |m| m.iter().find(|(mk, _)| mk.raw_equals(&k)).map(|(_, v)| v.clone()))?;
        Ok(c.push(v.unwrap_or_default()))
    });
    b.method("containsKey", |c| {
        let k = c.arg(1);
        let r = map(c, |m| m.iter().any(|(mk, _)| mk.raw_equals(&k)))?;
        Ok(c.push(r))
    });
    b.method("size", |c| {
        let n = map(c, |m| m.len())?;
        Ok(c.push(n as f64))
    });
    b.method("isEmpty", |c| {
        let r = map(c, |m| m.is_empty())?;
        Ok(c.push(r))
    });
    b.method("keySet", |c| {
        let keys = map(c, |m| m.iter().map(|(k, _)| k.clone()).collect())?;
        Ok(c.push(list_value(keys)))
    });
    b.method("values", |c| {
        let vals = map(c, |m| m.iter().map(|(_, v)| v.clone()).collect())?;
        Ok(c.push(list_value(vals)))
    });
    b.method("toString", |c| {
        let s = map(c, |m| JavaMap(m.to_vec()).to_lua_string())?;
        Ok(c.push(s))
    });
}

// ---- conversions ----

/// Java return value to Lua, as Kahlua converts it.
pub trait IntoLua {
    fn into_lua(self) -> Value;
}

impl IntoLua for Value {
    fn into_lua(self) -> Value {
        self
    }
}
impl IntoLua for bool {
    fn into_lua(self) -> Value {
        Value::Bool(self)
    }
}
macro_rules! num_into_lua {
    ($($t:ty),*) => {$(
        impl IntoLua for $t {
            fn into_lua(self) -> Value {
                Value::Num(self as f64)
            }
        }
    )*};
}
num_into_lua!(i8, i16, i32, i64, usize, f32, f64);
impl IntoLua for String {
    fn into_lua(self) -> Value {
        Value::from(self)
    }
}
impl IntoLua for &str {
    fn into_lua(self) -> Value {
        Value::from(self)
    }
}
impl IntoLua for &String {
    fn into_lua(self) -> Value {
        Value::from(self.as_str())
    }
}
impl<T: IntoLua> IntoLua for Option<T> {
    fn into_lua(self) -> Value {
        self.map(IntoLua::into_lua).unwrap_or_default()
    }
}

/// Lua argument to a Java parameter.
pub trait FromLua: Sized {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self;
}

impl FromLua for bool {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self {
        bool_arg(c, i)
    }
}
impl FromLua for i32 {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self {
        int_arg(c, i)
    }
}
impl FromLua for f32 {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self {
        num_arg(c, i) as f32
    }
}
impl FromLua for f64 {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self {
        num_arg(c, i)
    }
}
impl FromLua for Option<String> {
    fn from_lua(c: &NativeCall<'_>, i: usize) -> Self {
        str_arg(c, i).map(|s| s.to_string())
    }
}

pub fn opt_str(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

/// A `String` parameter that is dereferenced (`null` throws).
pub fn req_str(c: &NativeCall<'_>, i: usize) -> LuaResult<String> {
    opt_str(c, i).ok_or_else(npe)
}

/// The strings of a list argument (an `ArrayList` or a Lua array).
pub fn list_arg(c: &NativeCall<'_>, i: usize) -> Vec<Value> {
    match c.arg(i) {
        Value::Object(o) => {
            if let Some(l) = o.as_any().downcast_ref::<JavaList>() {
                l.0.clone()
            } else if let Some(v) = o.as_any().downcast_ref::<ListView>() {
                (v.items)()
            } else {
                Vec::new()
            }
        }
        Value::Table(t) => (1..).map(|i| t.get_int(i)).take_while(|v| !v.is_nil()).collect(),
        _ => Vec::new(),
    }
}

// ---- method helpers ----

/// A no-argument getter on `T`.
pub fn get<T: 'static, R: IntoLua>(b: &ClassBuilder, name: &str, f: impl Fn(&T) -> R + 'static) {
    b.method(name, move |c| {
        let v = this_handle(c)?.read::<T, _>(|t| f(t).into_lua())?;
        Ok(c.push(v))
    });
}

/// A one-argument setter on `T`.
pub fn set<T: 'static, A: FromLua>(b: &ClassBuilder, name: &str, f: impl Fn(&mut T, A) + 'static) {
    b.method(name, move |c| {
        let a = A::from_lua(c, 1);
        this_handle(c)?.write::<T, _>(|t| f(t, a))?;
        Ok(0)
    });
}

/// A getter returning an object made from the handle.
pub fn get_obj<T: 'static>(b: &ClassBuilder, name: &str, f: impl Fn(&ScriptHandle, &T) -> Value + 'static) {
    b.method(name, move |c| {
        let h = this_handle(c)?;
        let v = h.read::<T, _>(|t| f(&h, t))?;
        Ok(c.push(v))
    });
}

/// A method with arguments: `f(call, handle)`.
pub fn method(
    b: &ClassBuilder,
    name: &str,
    f: impl Fn(&mut NativeCall<'_>, &ScriptHandle) -> LuaResult<Value> + 'static,
) {
    b.method(name, move |c| {
        let h = this_handle(c)?;
        let v = f(c, &h)?;
        Ok(c.push(v))
    });
}

/// A void method with arguments.
pub fn void(b: &ClassBuilder, name: &str, f: impl Fn(&mut NativeCall<'_>, &ScriptHandle) -> LuaResult<()> + 'static) {
    b.method(name, move |c| {
        let h = this_handle(c)?;
        f(c, &h)?;
        Ok(0)
    });
}

fn script_type_arg(c: &NativeCall<'_>, i: usize) -> Option<ScriptType> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == SCRIPT_TYPE_CLASS)
            .and_then(|e| ScriptType::ALL.iter().copied().find(|t| t.name() == e.name)),
        _ => None,
    }
}

pub fn script_type_value(t: ScriptType) -> Value {
    enum_value(SCRIPT_TYPE_CLASS, t.name())
}

/// A `ScriptLoadMode` argument (`Init` unless it names `Reload`).
fn load_mode_arg(c: &NativeCall<'_>, i: usize) -> ScriptLoadMode {
    match c.arg(i) {
        Value::Object(o) if o.to_lua_string() == "Reload" => ScriptLoadMode::Reload,
        _ => ScriptLoadMode::Init,
    }
}

// ---- ScriptModule ----

/// A `ScriptModule`.
pub struct ModuleObject(pub Rc<ScriptModule>);

impl LuaObject for ModuleObject {
    fn class_name(&self) -> &str {
        SCRIPT_MODULE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn module_value(m: Option<&Rc<ScriptModule>>) -> Value {
    let Some(m) = m else { return Value::Nil };
    MODULES.with(|ms| {
        ms.borrow_mut()
            .entry(Rc::as_ptr(m) as usize)
            .or_insert_with(|| Value::Object(Rc::new(ModuleObject(m.clone()))))
            .clone()
    })
}

pub fn module_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<Rc<ScriptModule>>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ModuleObject>() {
            Some(m) => Ok(Some(m.0.clone())),
            None => Err(LuaError::runtime(format!("Expected ScriptModule but got {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected ScriptModule but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn this_module(c: &NativeCall<'_>) -> LuaResult<Rc<ScriptModule>> {
    let o = this::<ModuleObject>(c)?;
    Ok(o.as_any().downcast_ref::<ModuleObject>().unwrap().0.clone())
}

/// `ScriptModule.getX(name)` and the type they look up.
const MODULE_GETTERS: &[(&str, ScriptType)] = &[
    ("getVehicleTemplate", ScriptType::VehicleTemplate),
    ("getGameEntityTemplate", ScriptType::EntityTemplate),
    ("getItem", ScriptType::Item),
    ("getRecipe", ScriptType::Recipe),
    ("getUniqueRecipe", ScriptType::UniqueRecipe),
    ("getEvolvedRecipe", ScriptType::EvolvedRecipe),
    ("getFixing", ScriptType::Fixing),
    ("getAnimationsMesh", ScriptType::AnimationMesh),
    ("getClockScript", ScriptType::Clock),
    ("getMannequinScript", ScriptType::Mannequin),
    ("getModelScript", ScriptType::Model),
    ("getPhysicsShape", ScriptType::PhysicsShape),
    ("getSpriteModel", ScriptType::SpriteModel),
    ("getGameSound", ScriptType::Sound),
    ("getSoundTimeline", ScriptType::SoundTimeline),
    ("getVehicle", ScriptType::Vehicle),
    ("getAnimation", ScriptType::RuntimeAnimation),
    ("getVehicleEngineRPM", ScriptType::VehicleEngineRPM),
    ("getItemConfig", ScriptType::ItemConfig),
    ("getGameEntityScript", ScriptType::Entity),
    ("getXuiConfigScript", ScriptType::XuiConfig),
    ("getXuiLayout", ScriptType::XuiLayout),
    ("getXuiStyle", ScriptType::XuiStyle),
    ("getXuiDefaultStyle", ScriptType::XuiDefaultStyle),
    ("getXuiGlobalColors", ScriptType::XuiColor),
    ("getXuiSkinScript", ScriptType::XuiSkin),
    ("getItemFilter", ScriptType::ItemFilter),
    ("getFluidFilter", ScriptType::FluidFilter),
    ("getCraftRecipe", ScriptType::CraftRecipe),
    ("getStringList", ScriptType::StringList),
    ("getEnergyDefinitionScript", ScriptType::EnergyDefinition),
    ("getFluidDefinitionScript", ScriptType::FluidDefinition),
    ("getTimedActionScript", ScriptType::TimedAction),
    ("getRagdollScript", ScriptType::Ragdoll),
    ("getCharacterTraitScript", ScriptType::CharacterTraitDefinition),
    ("getCharacterProfessionScript", ScriptType::CharacterProfessionDefinition),
    ("getPhysicsHitReactionScript", ScriptType::PhysicsHitReaction),
];

fn register_module(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, SCRIPT_MODULE_CLASS, None);
    for &(name, t) in MODULE_GETTERS {
        let s = sm.clone();
        b.method(name, move |c| {
            let m = this_module(c)?;
            let name = req_str(c, 1)?;
            Ok(c.push(script_value(m.get(&s, t, &name).as_ref())))
        });
    }
    b.method("getName", |c| {
        let m = this_module(c)?;
        Ok(c.push(m.name()))
    });
    b.method("CheckExitPoints", |c| {
        let m = this_module(c)?;
        Ok(c.push(m.check_exit_points()))
    });
    b.method("Reset", |c| {
        this_module(c)?.reset();
        Ok(0)
    });
    let s = sm.clone();
    b.method("Load", move |c| {
        let m = this_module(c)?;
        let mode = load_mode_arg(c, 1);
        m.load(&s, mode, &req_str(c, 2)?, &req_str(c, 3)?).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("ParseScriptPP", move |c| {
        let m = this_module(c)?;
        let mode = load_mode_arg(c, 1);
        m.parse_script_pp(&s, mode, &req_str(c, 2)?).map_err(java_error)?;
        Ok(0)
    });
    b.method("equals", |c| {
        let r = c.arg(0).raw_equals(&c.arg(1));
        Ok(c.push(r))
    });
}

// ---- ScriptType ----

fn register_script_type(thread: &mut Thread) {
    let names: Vec<&'static str> = ScriptType::ALL.iter().map(|t| t.name()).collect();
    let b = expose_enum(thread, SCRIPT_TYPE_CLASS, &names);
    b.static_fn("GetEnumListLua", |c| {
        let list: Vec<Value> = ScriptType::enum_list_lua().into_iter().map(script_type_value).collect();
        Ok(c.push(list_value(list)))
    });
    fn this_type(c: &NativeCall<'_>) -> LuaResult<ScriptType> {
        script_type_arg(c, 0).ok_or_else(|| LuaError::runtime("Expected ScriptType"))
    }
    b.method("isTemplate", |c| {
        let t = this_type(c)?;
        Ok(c.push(t.is_template()))
    });
    b.method("isCritical", |c| {
        this_type(c)?;
        Ok(c.push(false))
    });
    b.method("getScriptTag", |c| {
        let t = this_type(c)?;
        Ok(c.push(t.script_tag()))
    });
    // TODO(ScriptType.verbose): the original's flag is mutable; the port
    // keeps the static-initializer value.
    b.method("isVerbose", |c| {
        let t = this_type(c)?;
        Ok(c.push(t.default_verbose()))
    });
}

// ---- BaseScriptObject, GameEntityScript, ComponentScript ----

fn register_base(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, BASE_SCRIPT_OBJECT_CLASS, None);
    method(&b, "getObsolete", |_, h| h.script(|s| s.get_obsolete().into_lua()));
    method(&b, "isEnabled", |_, h| h.base(|b| b.enabled.into_lua()));
    method(&b, "isDebugOnly", |_, h| h.base(|b| b.debug_only.into_lua()));
    method(&b, "getScriptVersion", |_, h| h.base(|b| b.script_version().into_lua()));
    method(&b, "getScriptObjectName", |_, h| h.base(|b| b.script_object_name().into_lua()));
    method(&b, "getScriptObjectType", |_, h| h.base(|b| script_type_value(b.script_object_type())));
    method(&b, "getScriptObjectFullType", |_, h| h.base(|b| b.full_type().map_err(java_error))?.map(IntoLua::into_lua));
    method(&b, "getModule", |_, h| h.base(|b| module_value(b.module().as_ref())));
    method(&b, "debugString", |_, h| h.base(|b| b.debug_string().into_lua()));
    method(&b, "getLoadedScriptBodyCount", |_, h| h.base(|b| b.loaded_script_body_count().into_lua()));
    method(&b, "getLoadedScriptBodies", |_, h| {
        h.base(|b| list_value(b.loaded_script_bodies().iter().map(|s| s.as_deref().into_lua()).collect()))
    });
    method(&b, "getScriptLines", |_, h| {
        h.base(|b| {
            let mut lines = Vec::new();
            b.all_script_lines(&mut lines);
            string_list(lines)
        })
    });
    method(&b, "getAllScriptLines", |c, h| {
        let mut lines: Vec<Value> = list_arg(c, 1);
        let mut more = Vec::new();
        h.base(|b| b.all_script_lines(&mut more))?;
        lines.extend(more.into_iter().map(Value::from));
        Ok(list_value(lines))
    });
    method(&b, "getBodyScriptLines", |c, h| {
        let index = int_arg(c, 1);
        let mut lines: Vec<Value> = list_arg(c, 2);
        let mut more = Vec::new();
        h.base(|b| b.body_script_lines(index, &mut more))?;
        lines.extend(more.into_iter().map(Value::from));
        Ok(list_value(lines))
    });
    void(&b, "resetLoadedScriptBodies", |_, h| h.base_mut(|b| b.reset_loaded_script_bodies()));
    void(&b, "addLoadedScriptBody", |c, h| {
        let (m, body) = (opt_str(c, 1), opt_str(c, 2).unwrap_or_else(|| "null".into()));
        h.base_mut(|b| b.add_loaded_script_body(m.as_deref(), &body))
    });
    void(&b, "calculateScriptVersion", |_, h| h.base_mut(|b| b.calculate_script_version())?.map_err(java_error));
    void(&b, "LoadCommonBlock", |c, h| {
        let body = req_str(c, 1)?;
        h.base_mut(|b| b.load_common_block_str(&body))?.map_err(java_error)
    });
    void(&b, "setModule", |c, h| {
        let m = module_arg(c, 1)?.ok_or_else(npe)?;
        h.script_mut(|s| s.set_module(&m))
    });
    let s = sm.clone();
    void(&b, "InitLoadPP", move |c, h| {
        let name = req_str(c, 1)?;
        h.script_mut(|o| o.init_load_pp(&s, &name))
    });
    let s = sm.clone();
    void(&b, "Load", move |c, h| {
        let (name, body) = (req_str(c, 1)?, req_str(c, 2)?);
        h.script_mut(|o| o.load(&s, &name, &body))?.map_err(java_error)
    });
    void(&b, "PreReload", |_, h| h.script_mut(|o| o.pre_reload()));
    void(&b, "reset", |_, h| h.script_mut(|o| o.reset()));
    let s = sm.clone();
    void(&b, "OnScriptsLoaded", move |c, h| {
        let mode = load_mode_arg(c, 1);
        h.script_mut(|o| o.on_scripts_loaded(&s, mode))?.map_err(java_error)
    });
    let s = sm.clone();
    void(&b, "OnLoadedAfterLua", move |_, h| h.script_mut(|o| o.on_loaded_after_lua(&s))?.map_err(java_error));
    let s = sm.clone();
    void(&b, "OnPostWorldDictionaryInit", move |_, h| {
        h.script_mut(|o| o.on_post_world_dictionary_init(&s))?.map_err(java_error)
    });
    b.method("getVersion", |c| {
        let h = this_handle(c)?;
        let simple = crate::expose::simple_name(h.class);
        Err(LuaError::Runtime {
            message: format!("Not implemented. class = {simple}"),
            class: "java.lang.RuntimeException",
        })
    });
    b.method("toString", |c| {
        let s = this_handle(c)?.to_lua_string();
        Ok(c.push(s))
    });
    b.method("equals", |c| {
        let r = c.arg(0).raw_equals(&c.arg(1));
        Ok(c.push(r))
    });
    b.method("hashCode", |c| {
        let h = this_handle(c)?;
        Ok(c.push((h.root.as_ptr() as usize as u32 as i32) as f64))
    });

    for &(class, sup) in SCRIPT_CLASSES {
        expose_class(thread, class, Some(sup));
    }

    let b = expose_class(thread, GAME_ENTITY_SCRIPT_CLASS, Some(BASE_SCRIPT_OBJECT_CLASS));
    get(&b, "getName", |e: &GameEntityScript| e.name().map(str::to_string));
    get(&b, "getModuleName", |e: &GameEntityScript| e.module_name());
    method(&b, "getFullName", |_, h| {
        h.read(|e: &GameEntityScript| e.full_name().map_err(java_error))?.map(IntoLua::into_lua)
    });
    get(&b, "getModID", |e: &GameEntityScript| e.mod_id().map(str::to_string));
    get(&b, "getExistsAsVanilla", |e: &GameEntityScript| e.exists_as_vanilla());
    get(&b, "getFileAbsPath", |e: &GameEntityScript| e.file_abs_path().map(str::to_string));
    get(&b, "getRegistry_id", |e: &GameEntityScript| e.registry_id());
    void(&b, "setRegistry_id", |c, h| {
        let id = int_arg(c, 1) as i16;
        h.write(|e: &mut GameEntityScript| e.set_registry_id(id))?.map_err(java_error)
    });
    let s = sm.clone();
    void(&b, "setModID", move |c, h| {
        let id = req_str(c, 1)?;
        // `GameClient.client`: the main menu runs no client connection.
        h.write(|e: &mut GameEntityScript| e.set_mod_id(&s, false, &id))?.map_err(java_error)
    });
    get(&b, "hasComponents", |e: &GameEntityScript| e.has_components());
    get(&b, "getDisplayNameDebug", |e: &GameEntityScript| e.display_name_debug());
    method(&b, "containsComponent", |c, h| {
        let t = component_type_arg(c, 1);
        h.read(|e: &GameEntityScript| t.is_some_and(|t| e.contains_component(t)).into_lua())
    });
    method(&b, "getComponentScripts", |_, h| {
        let n = h.read(|e: &GameEntityScript| e.component_scripts().len())?;
        let list = (0..n).map(|i| component_value(h, i)).collect::<LuaResult<Vec<_>>>()?;
        Ok(list_value(list))
    });
    method(&b, "getComponentScriptFor", |c, h| {
        let t = component_type_arg(c, 1);
        let i =
            h.read(|e: &GameEntityScript| e.component_scripts().iter().position(|s| Some(s.component_type()) == t))?;
        match i {
            Some(i) => component_value(h, i),
            None => Ok(Value::Nil),
        }
    });
    let s = sm.clone();
    void(&b, "LoadAttribute", move |c, h| {
        let (k, v) = (req_str(c, 1)?, req_str(c, 2)?);
        h.write(|e: &mut GameEntityScript| e.load_attribute(&s, &k, &v))?.map_err(java_error)?;
        Ok(())
    });
    void(&b, "copyFrom", |c, h| {
        let other = handle_arg(c, 1)?.ok_or_else(npe)?;
        if other.root.ptr_eq(&h.root) {
            return Ok(());
        }
        other.read(|o: &GameEntityScript| h.write(|e: &mut GameEntityScript| e.copy_from(o)))??.map_err(java_error)
    });

    let b = expose_class(thread, COMPONENT_SCRIPT_CLASS, Some(BASE_SCRIPT_OBJECT_CLASS));
    let s = sm.clone();
    method(&b, "getName", move |_, h| {
        h.read_any(|a| a.downcast_ref::<Box<dyn ComponentScript>>().map(|c| c.component().name(&s)))?
            .ok_or_else(|| wrong_type::<Box<dyn ComponentScript>>(h.class))?
            .map(IntoLua::into_lua)
            .map_err(java_error)
    });
    method(&b, "getComponentType", |_, h| {
        let t = h
            .read_any(|a| a.downcast_ref::<Box<dyn ComponentScript>>().map(|c| c.component_type()))?
            .ok_or_else(npe)?;
        Ok(enum_value(COMPONENT_TYPE_CLASS, t.name()))
    });
    let b = expose_class(thread, CRAFT_RECIPE_COMPONENT_CLASS, Some(COMPONENT_SCRIPT_CLASS));
    get_obj(&b, "getCraftRecipe", |h, c: &CraftRecipeComponentScript| match c.craft_recipe() {
        Some(_) => h.child(CRAFT_RECIPE_CLASS, "craftRecipe", |c: &CraftRecipeComponentScript| c.craft_recipe()),
        None => Value::Nil,
    });
    get(&b, "getBuildCategory", |c: &CraftRecipeComponentScript| c.build_category().map(str::to_string));
}

pub const COMPONENT_TYPE_CLASS: &str = "zombie.entity.ComponentType";
pub const CRAFT_RECIPE_COMPONENT_CLASS: &str = "zombie.scripting.entity.components.crafting.CraftRecipeComponentScript";

fn component_type_arg(c: &NativeCall<'_>, i: usize) -> Option<ComponentType> {
    match c.arg(i) {
        Value::Object(o) => ComponentType::value_of(&o.to_lua_string()).ok(),
        _ => None,
    }
}

/// The handle on component `index` of an entity script.
fn component_value(h: &ScriptHandle, index: usize) -> LuaResult<Value> {
    let class = h.read(|e: &GameEntityScript| e.component_scripts()[index].class_name())?;
    let class = if class.ends_with("CraftRecipeComponentScript") { CRAFT_RECIPE_COMPONENT_CLASS } else { class };
    Ok(h.child(class, &format!("component{index}"), move |e: &GameEntityScript| e.component_scripts().get(index)))
}

/// The craft recipe an entity script embeds (`CraftRecipeComponentScript`).
fn entity_recipe(h: &ScriptHandle) -> Value {
    h.child(CRAFT_RECIPE_CLASS, "component:CraftRecipe/craftRecipe", |e: &GameEntityScript| {
        e.component::<CraftRecipeComponentScript>(ComponentType::CraftRecipe)?.craft_recipe()
    })
}

// ---- ScriptManager ----

/// `ScriptManager.instance`.
pub struct ScriptManagerObject(pub Rc<ScriptManager>);

impl LuaObject for ScriptManagerObject {
    fn class_name(&self) -> &str {
        SCRIPT_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `ScriptManager.getX(name)` (`getScript` on the type's collection).
const MANAGER_GETTERS: &[(&str, ScriptType)] = &[
    ("getVehicleTemplate", ScriptType::VehicleTemplate),
    ("getGameEntityTemplate", ScriptType::EntityTemplate),
    ("getItem", ScriptType::Item),
    ("getRecipe", ScriptType::Recipe),
    ("getUniqueRecipe", ScriptType::UniqueRecipe),
    ("getEvolvedRecipe", ScriptType::EvolvedRecipe),
    ("getFixing", ScriptType::Fixing),
    ("getAnimationsMesh", ScriptType::AnimationMesh),
    ("getClockScript", ScriptType::Clock),
    ("getMannequinScript", ScriptType::Mannequin),
    ("getModelScript", ScriptType::Model),
    ("getPhysicsShape", ScriptType::PhysicsShape),
    ("getGameSound", ScriptType::Sound),
    ("getSoundTimeline", ScriptType::SoundTimeline),
    ("getSpriteModel", ScriptType::SpriteModel),
    ("getVehicle", ScriptType::Vehicle),
    ("getRuntimeAnimationScript", ScriptType::RuntimeAnimation),
    ("getVehicleEngineRPM", ScriptType::VehicleEngineRPM),
    ("getItemConfig", ScriptType::ItemConfig),
    ("getGameEntityScript", ScriptType::Entity),
    ("getXuiConfigScript", ScriptType::XuiConfig),
    ("getXuiLayout", ScriptType::XuiLayout),
    ("getXuiStyle", ScriptType::XuiStyle),
    ("getXuiDefaultStyle", ScriptType::XuiDefaultStyle),
    ("getXuiColor", ScriptType::XuiColor),
    ("getXuiSkinScript", ScriptType::XuiSkin),
    ("getItemFilter", ScriptType::ItemFilter),
    ("getFluidFilter", ScriptType::FluidFilter),
    ("getStringList", ScriptType::StringList),
    ("getEnergyDefinitionScript", ScriptType::EnergyDefinition),
    ("getFluidDefinitionScript", ScriptType::FluidDefinition),
    ("getTimedActionScript", ScriptType::TimedAction),
    ("getRagdollScript", ScriptType::Ragdoll),
    ("getPhysicsHitReactionScript", ScriptType::PhysicsHitReaction),
    ("getCharacterTraitScript", ScriptType::CharacterTraitDefinition),
    ("getCharacterProfessionScript", ScriptType::CharacterProfessionDefinition),
];

/// `ScriptManager.getAllX()` (`getAllScripts` on the type's collection).
const MANAGER_LISTS: &[(&str, ScriptType)] = &[
    ("getAllVehicleTemplates", ScriptType::VehicleTemplate),
    ("getAllGameEntityTemplates", ScriptType::EntityTemplate),
    ("getAllItems", ScriptType::Item),
    ("getAllRecipes", ScriptType::Recipe),
    ("getAllUniqueRecipes", ScriptType::UniqueRecipe),
    ("getAllEvolvedRecipesList", ScriptType::EvolvedRecipe),
    ("getAllEvolvedRecipes", ScriptType::EvolvedRecipe),
    ("getAllAnimationsMeshes", ScriptType::AnimationMesh),
    ("getAllClockScripts", ScriptType::Clock),
    ("getAllMannequinScripts", ScriptType::Mannequin),
    ("getAllModelScripts", ScriptType::Model),
    ("getAllPhysicsShapes", ScriptType::PhysicsShape),
    ("getAllGameSounds", ScriptType::Sound),
    ("getAllSoundTimelines", ScriptType::SoundTimeline),
    ("getAllSpriteModels", ScriptType::SpriteModel),
    ("getAllVehicleScripts", ScriptType::Vehicle),
    ("getAllRuntimeAnimationScripts", ScriptType::RuntimeAnimation),
    ("getAllVehicleEngineRPMs", ScriptType::VehicleEngineRPM),
    ("getAllItemConfigs", ScriptType::ItemConfig),
    ("getAllGameEntities", ScriptType::Entity),
    ("getAllXuiConfigScripts", ScriptType::XuiConfig),
    ("getAllXuiLayouts", ScriptType::XuiLayout),
    ("getAllXuiStyles", ScriptType::XuiStyle),
    ("getAllXuiDefaultStyles", ScriptType::XuiDefaultStyle),
    ("getAllXuiColors", ScriptType::XuiColor),
    ("getAllXuiSkinScripts", ScriptType::XuiSkin),
    ("getAllItemFilters", ScriptType::ItemFilter),
    ("getAllFluidFilters", ScriptType::FluidFilter),
    ("getAllCraftRecipes", ScriptType::CraftRecipe),
    ("getAllStringLists", ScriptType::StringList),
    ("getAllEnergyDefinitionScripts", ScriptType::EnergyDefinition),
    ("getAllFluidDefinitionScripts", ScriptType::FluidDefinition),
    ("getAllTimedActionScripts", ScriptType::TimedAction),
];

/// Entities whose `CraftRecipe` component holds a buildable recipe, with
/// the recipe's name (`getAllBuildableRecipes`' scan).
fn buildable_recipes(sm: &ScriptManager) -> Vec<(ScriptRef, Option<String>)> {
    let mut out = Vec::new();
    for e in sm.collection(ScriptType::Entity).get_all_scripts() {
        let Some(entity) = e.downcast::<GameEntityScript>() else { continue };
        let entity = entity.borrow();
        let recipe = entity
            .component::<CraftRecipeComponentScript>(ComponentType::CraftRecipe)
            .and_then(CraftRecipeComponentScript::craft_recipe);
        if let Some(r) = recipe.filter(|r| r.is_buildable_recipe()) {
            out.push((e.clone(), r.name().map(str::to_string)));
        }
    }
    out
}

fn entity_recipe_value(entity: &ScriptRef) -> Value {
    match script_value(Some(entity)) {
        Value::Object(o) => match o.as_any().downcast_ref::<ScriptHandle>() {
            Some(h) => entity_recipe(h),
            None => Value::Nil,
        },
        _ => Value::Nil,
    }
}

/// `getCraftRecipe`: the script, else the recipe of the entity so named.
pub fn get_craft_recipe(sm: &ScriptManager, name: &str) -> Value {
    if let Some(r) = sm.get_script(ScriptType::CraftRecipe, name) {
        return script_value(Some(&r));
    }
    let Some(e) = sm.get_script(ScriptType::Entity, name) else { return Value::Nil };
    let has = e.downcast::<GameEntityScript>().is_some_and(|g| {
        g.borrow()
            .component::<CraftRecipeComponentScript>(ComponentType::CraftRecipe)
            .is_some_and(|c| c.craft_recipe().is_some())
    });
    if has {
        entity_recipe_value(&e)
    } else {
        Value::Nil
    }
}

fn register_manager(thread: &mut Thread, sm: &Rc<ScriptManager>) -> Value {
    let instance = Value::Object(Rc::new(ScriptManagerObject(sm.clone())));
    let b = expose_class(thread, SCRIPT_MANAGER_CLASS, None);
    b.field("instance", instance.clone());
    b.field("Base", pz_scripts::manager::BASE);
    b.field("Base_Module", pz_scripts::manager::BASE_MODULE);
    b.field("VanillaID", pz_scripts::manager::VANILLA_ID);
    let inst = instance.clone();
    global(thread, "getScriptManager", move |c| Ok(c.push(inst.clone())));

    let s = sm.clone();
    b.static_fn("EnableDebug", move |c| {
        if let Some(t) = script_type_arg(c, 0) {
            s.enable_debug(t, bool_arg(c, 1));
        }
        Ok(0)
    });
    let s = sm.clone();
    b.static_fn("isDebugEnabled", move |c| {
        let r = script_type_arg(c, 0).is_some_and(|t| s.is_debug_enabled(t));
        Ok(c.push(r))
    });
    let s = sm.clone();
    b.static_fn("println", move |c| {
        let t = match handle_arg(c, 0) {
            Ok(Some(h)) => h.base(|b| b.script_object_type()).ok(),
            _ => script_type_arg(c, 0),
        };
        if let Some(t) = t {
            s.println(t, &opt_str(c, 1).unwrap_or_else(|| "null".into()));
        }
        Ok(0)
    });
    b.static_fn("getItemName", |c| {
        let n = req_str(c, 0)?;
        Ok(c.push(ScriptManager::get_item_name(&n)))
    });
    type LoadState = fn(&ScriptManager) -> Option<String>;
    let load_state: [(&str, LoadState); 3] = [
        ("getCurrentLoadFileMod", ScriptManager::current_load_file_mod),
        ("getCurrentLoadFileAbsPath", ScriptManager::current_load_file_abs_path),
        ("getCurrentLoadFileName", ScriptManager::current_load_file_name),
    ];
    for (name, f) in load_state {
        let s = sm.clone();
        b.static_fn(name, move |c| Ok(c.push(f(&s).into_lua())));
    }

    for &(name, t) in MANAGER_GETTERS {
        let s = sm.clone();
        b.method(name, move |c| {
            let name = req_str(c, 1)?;
            Ok(c.push(script_value(s.get_script(t, &name).as_ref())))
        });
    }
    for &(name, t) in MANAGER_LISTS {
        let s = sm.clone();
        b.method(name, move |c| Ok(c.push(script_list(&s.collection(t).get_all_scripts()))));
    }
    let s = sm.clone();
    b.method("getAllFixing", move |c| {
        let mut list = list_arg(c, 1);
        list.extend(s.collection(ScriptType::Fixing).get_all_scripts().iter().map(|f| script_value(Some(f))));
        Ok(c.push(list_value(list)))
    });
    let s = sm.clone();
    b.method("getScriptsForType", move |c| {
        let list = match script_type_arg(c, 1) {
            Some(t) => s.get_scripts_for_type(t),
            None => Vec::new(),
        };
        Ok(c.push(script_list(&list)))
    });
    let s = sm.clone();
    b.method("getCraftRecipe", move |c| {
        let name = req_str(c, 1)?;
        Ok(c.push(get_craft_recipe(&s, &name)))
    });
    let s = sm.clone();
    b.method("getAllBuildableRecipes", move |c| {
        let list: Vec<Value> = buildable_recipes(&s).iter().map(|(e, _)| entity_recipe_value(e)).collect();
        Ok(c.push(list_value(list)))
    });
    let s = sm.clone();
    b.method("getBuildableRecipe", move |c| {
        let name = opt_str(c, 1);
        let found = buildable_recipes(&s).into_iter().find(|(_, n)| *n == name);
        Ok(c.push(found.map(|(e, _)| entity_recipe_value(&e)).unwrap_or_default()))
    });
    let s = sm.clone();
    b.method("addModelScript", move |c| {
        let h = handle_arg(c, 1)?.ok_or_else(npe)?;
        s.add_runtime_script(h.root.clone()).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("addSpriteModel", move |c| {
        let h = handle_arg(c, 1)?.ok_or_else(npe)?;
        s.add_runtime_script(h.root.clone()).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("getRandomVehicleScript", move |c| {
        let list: Vec<ScriptRef> = s
            .collection(ScriptType::Vehicle)
            .get_all_scripts()
            .into_iter()
            .filter(|v| {
                let n = pz_scripts::java::to_lower(&v.name());
                !n.contains("burnt") && !n.contains("smashed")
            })
            .collect();
        let i = pz_core::random::next(list.len() as i32);
        match list.get(i as usize) {
            Some(v) => Ok(c.push(script_value(Some(v)))),
            None => Err(LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {}", list.len()),
                class: "java.lang.IndexOutOfBoundsException",
            }),
        }
    });
    let s = sm.clone();
    b.method("getModule", move |c| {
        let name = req_str(c, 1)?;
        let m = if c.nargs() > 2 { s.get_module_default(&name, bool_arg(c, 2)) } else { s.get_module(&name) };
        Ok(c.push(module_value(m.as_ref())))
    });
    let s = sm.clone();
    b.method("getModuleNoDisableCheck", move |c| {
        let name = req_str(c, 1)?;
        Ok(c.push(module_value(s.get_module_no_disable_check(&name).as_ref())))
    });
    let s = sm.clone();
    b.method("FindItem", move |c| {
        let name = req_str(c, 1)?;
        let base = if c.nargs() > 2 { bool_arg(c, 2) } else { true };
        Ok(c.push(script_value(s.find_item(&name, base).as_ref())))
    });
    let s = sm.clone();
    b.method("isDrainableItemType", move |c| {
        let name = req_str(c, 1)?;
        let r = s
            .find_item(&name, true)
            .and_then(|i| i.downcast::<Item>())
            .is_some_and(|i| i.borrow().is_item_type(&s, "DRAINABLE"));
        Ok(c.push(r))
    });
    let s = sm.clone();
    b.method("CheckExitPoints", move |_| {
        for m in s.module_list() {
            if !m.is_disabled() && m.check_exit_points() {
                break;
            }
        }
        Ok(0)
    });
    let s = sm.clone();
    b.method("getItemsTag", move |c| {
        let tag = key_arg(c, 1)?;
        let items: Vec<Value> = s
            .collection(ScriptType::Item)
            .get_all_scripts()
            .iter()
            .filter(|i| i.downcast::<Item>().is_some_and(|i| i.borrow().has_tag(tag.as_ref())))
            .map(|i| script_value(Some(i)))
            .collect();
        Ok(c.push(list_value(items)))
    });
    let s = sm.clone();
    b.method("getItemsByType", move |c| {
        let t = opt_str(c, 1).unwrap_or_else(|| "null".into());
        let items = s.get_items_by_type(&t).map_err(java_error)?;
        Ok(c.push(script_list(&items)))
    });
    let s = sm.clone();
    b.method("Reset", move |_| {
        s.reset();
        Ok(0)
    });
    b.method("update", |_| Ok(0));
    let s = sm.clone();
    b.method("getChecksum", move |c| Ok(c.push(s.checksum())));
    let s = sm.clone();
    b.method("LoadFile", move |c| {
        let mode = load_mode_arg(c, 1);
        let name = req_str(c, 2)?;
        s.load_file(mode, &name, bool_arg(c, 3)).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("ParseScript", move |c| {
        let mode = load_mode_arg(c, 1);
        let body = req_str(c, 2)?;
        s.parse_script(mode, &body).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("ReloadScripts", move |c| {
        let types: Vec<ScriptType> = match script_type_arg(c, 1) {
            Some(t) => vec![t],
            None => list_arg(c, 1)
                .iter()
                .filter_map(|v| match v {
                    Value::Object(o) => ScriptType::ALL.iter().copied().find(|t| t.name() == o.to_lua_string()),
                    _ => None,
                })
                .collect(),
        };
        s.reload_scripts(&types);
        Ok(0)
    });
    type Phase = fn(&ScriptManager);
    let phases: [(&str, Phase); 3] = [
        ("LoadedAfterLua", ScriptManager::loaded_after_lua),
        ("PostTileDefinitions", ScriptManager::post_tile_definitions),
        ("PostWorldDictionaryInit", ScriptManager::post_world_dictionary_init),
    ];
    for (name, f) in phases {
        let s = sm.clone();
        b.method(name, move |_| {
            f(&s);
            Ok(0)
        });
    }
    // Only runs with `Core.debug`. TODO(Translator): the debug report.
    b.method("VerifyAllCraftRecipesAreLearnable", |_| Ok(0));
    let s = sm.clone();
    b.method("hasLoadErrors", move |c| {
        let r = s.has_load_errors(c.nargs() > 1 && bool_arg(c, 1));
        Ok(c.push(r))
    });
    let s = sm.clone();
    b.method("getAllRecipesFor", move |c| {
        let result = opt_str(c, 1);
        let mut out = Vec::new();
        for r in s.collection(ScriptType::Recipe).get_all_scripts() {
            let Some(recipe) = r.downcast::<pz_scripts::objects::recipes::Recipe>() else { continue };
            let t = recipe.borrow().result().and_then(|res| res.type_.clone()).ok_or_else(npe)?;
            let t = match t.find('.') {
                Some(i) => t[i + 1..].to_string(),
                None => t,
            };
            if Some(t) == result {
                out.push(script_value(Some(&r)));
            }
        }
        Ok(c.push(list_value(out)))
    });
    let s = sm.clone();
    b.method("getItemTypeForClothingItem", move |c| {
        let name = opt_str(c, 1).unwrap_or_default();
        Ok(c.push(pz_scripts::objects::items::get_item_type_for_clothing_item(&s, &name).into_lua()))
    });
    let s = sm.clone();
    b.method("getItemForClothingItem", move |c| {
        let name = opt_str(c, 1).unwrap_or_default();
        let item =
            pz_scripts::objects::items::get_item_type_for_clothing_item(&s, &name).and_then(|t| s.find_item(&t, true));
        Ok(c.push(script_value(item.as_ref())))
    });
    let s = sm.clone();
    b.method("getZedDmgMap", move |c| Ok(c.push(string_list(pz_scripts::objects::items::get_zed_dmg_map(&s)))));
    let s = sm.clone();
    b.method("resolveItemType", move |c| {
        let m = module_arg(c, 1)?;
        let t = opt_str(c, 2);
        let r = match (m, t) {
            (_, None) => None,
            (Some(m), Some(t)) => s.resolve_item_type(&m, &t),
            (None, Some(t)) if pz_scripts::java::is_null_or_whitespace(Some(&t)) || t.contains('.') => Some(t),
            (None, Some(_)) => return Err(npe()),
        };
        Ok(c.push(r.into_lua()))
    });
    let s = sm.clone();
    b.method("resolveModelScript", move |c| {
        let m = module_arg(c, 1)?;
        let t = opt_str(c, 2);
        let r = match (m, t) {
            (_, None) => None,
            (Some(m), Some(t)) => s.resolve_model_script(&m, &t),
            (None, Some(t)) if pz_scripts::java::is_null_or_whitespace(Some(&t)) || t.contains('.') => Some(t),
            (None, Some(_)) => return Err(npe()),
        };
        Ok(c.push(r.into_lua()))
    });
    for (name, t) in [("getSpecificItem", ScriptType::Item), ("getSpecificEntity", ScriptType::Entity)] {
        let s = sm.clone();
        b.method(name, move |c| {
            let n = req_str(c, 1)?;
            if s.debug && !n.contains('.') {
                return Err(LuaError::Runtime {
                    message: format!("ScriptManager.{name} requires a full type name, cannot find: {n}"),
                    class: "java.lang.RuntimeException",
                });
            }
            Ok(c.push(script_value(s.get_specific(t, &n).as_ref())))
        });
    }
    instance
}

/// Register the scripting classes and `getScriptManager()` on `thread`,
/// bound to `sm`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>) {
    register_map(thread);
    register_script_type(thread);
    register_module(thread, &sm);
    register_base(thread, &sm);
    register_manager(thread, &sm);
    item::register(thread, &sm);
    recipes::register(thread, &sm);
    models::register(thread, &sm);
    vehicles::register(thread, &sm);
    crafting::register(thread, &sm);
}

/// A `Set` of registry keys as an `ArrayList` of key objects.
pub fn key_list(class: &'static str, keys: &pz_scripts::objects::items::KeySet) -> Value {
    list_value(keys.iter().map(|k| key_value(class, k)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn load(tag: &str, files: &[(&str, &str)]) -> Rc<ScriptManager> {
        let dir = std::env::temp_dir().join(format!("pz-bind-scripts-{}-{tag}", std::process::id()));
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let sm = ScriptManager::new();
        sm.builtin_tables.borrow_mut().perk_ids = ["Woodwork", "Cooking"].map(String::from).to_vec();
        sm.builtin_tables.borrow_mut().metabolics = vec![("Default".into(), 1.5), ("Napping".into(), 0.5)];
        for (reg, id) in [("item_tag", "t:shiny"), ("item_tag", "t:dull"), ("item_type", "t:normal")] {
            sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
        }
        sm.statics.items.borrow_mut().constants.insert("item_tag", "SHINY", "t:shiny");
        sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        Rc::new(sm)
    }

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))
    }

    fn thread(sm: Rc<ScriptManager>) -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end")
            .unwrap();
        crate::bind_core::register(&mut t, None, Rc::new(RefCell::new(pz_core::core::Core::default())));
        crate::bind_registry::register(&mut t, sm.clone());
        register(&mut t, sm);
        t
    }

    const SCRIPTS: &str = "module Base {
        item Apple { DisplayName = Red Apple, Weight = 0.5, Tags = t:shiny, ClothingItem = AppleHat, }
        item Stone { Weight = -2, Tags = t:dull, }
        recipe Mash { Apple, keep Stone=2, Result:Apple=2, Time:30.0, }
        timedAction Nap { metabolics = Napping, }
        model AppleModel { mesh = Apple, attachment world { offset = 1 2 3, rotate = 0 90 0, } }
        vehicle Cart {
            mass = 300, zombieType = Farmer,
            model Default { file = CartMesh, scale = 2, offset = 0 0.5 0, }
            area Back { xywh = 0 1 2 3, }
            part Wheel { model Left { file = W, } }
            passenger Driver { position inside { offset = 0 0 0.5, } }
        }
        craftRecipe Carve { time = 20, Tags = InHandCraft, inputs { item 1 [Base.Stone] flags[ToolLeft;Prop1] mode:keep, item 2 [Base.Apple], } outputs { item 1 Base.Apple, } }
    }";

    #[test]
    fn manager_items_and_recipes() {
        let sm = load("items", &[("media/scripts/test.txt", SCRIPTS)]);
        let mut t = thread(sm);
        run(
            &mut t,
            r#"
            local sm = getScriptManager()
            assert(sm == ScriptManager.instance)
            local apple = sm:getItem("Apple")
            assert(apple ~= nil and apple == sm:FindItem("Base.Apple") and apple == sm:getSpecificItem("Base.Apple"))
            assert(apple:getDisplayName() == "Red Apple" and apple:getFullName() == "Base.Apple")
            assert(apple:getActualWeight() == 0.5 and sm:getItem("Stone"):getActualWeight() == 0)
            assert(apple:hasTag(ItemTag.SHINY) and not sm:getItem("Stone"):hasTag(ItemTag.SHINY))
            assert(apple:getTags():size() == 1 and apple:getTags():get(0) == ItemTag.SHINY)
            assert(sm:getItemsTag(ItemTag.SHINY):size() == 1)
            assert(apple:getScriptObjectType() == ScriptType.Item)
            assert(apple:getModule():getName() == "Base" and apple:getModule():getItem("Apple") == apple)
            assert(sm:getItemTypeForClothingItem("AppleHat") == "Base.Apple")
            apple:setCount(3)
            assert(apple:getCount() == 3)
            assert(sm:getAllItems():size() == 2)
            assert(sm:getItemsByType("Apple"):get(0) == apple)
            assert(ScriptManager.getItemName("Base.Apple") == "Apple")
            assert(ScriptType.GetEnumListLua():contains(ScriptType.Vehicle))
            assert(ScriptType.Item:getScriptTag() == "item")

            local r = sm:getRecipe("Mash")
            assert(r:getName() == "Mash" and r:getSource():size() == 2)
            local keep = r:findSource("Stone")
            assert(keep:isKeep() and keep:getCount() == 2 and keep == r:getSource():get(1))
            keep:setCount(4)
            assert(r:getSource():get(1):getCount() == 4)
            r:DoResult("Apple=2")
            assert(r:getResult():getType() == "Apple" and r:getResult():getCount() == 2)
            assert(sm:getAllRecipesFor("Apple"):size() == 1)
            "#,
        )
        .unwrap();
        let e = run(&mut t, "getScriptManager():getItemsByType('')").unwrap_err();
        assert!(e.contains("invalid type"), "{e}");
    }

    #[test]
    fn models_vehicles_and_craft_recipes() {
        let sm = load("models", &[("media/scripts/test.txt", SCRIPTS)]);
        let mut t = thread(sm);
        run(
            &mut t,
            r#"
            local sm = getScriptManager()
            assert(sm:getTimedActionScript("Nap"):getMetabolics() == Metabolics.Napping)
            local m = sm:getModelScript("AppleModel")
            local w0 = m:getAttachmentById("world")
            assert(m:getMeshName() == "Apple" and m:getTextureName() == "Apple" and m:getAttachmentCount() == 1)
            local w = m:getAttachmentById("world")
            assert(w == m:getAttachment(0) and w:getOffset():y() == 2 and w:getRotate():y() == 90)
            w:getOffset():set(4, 5, 6)
            assert(m:getAttachment(0):getOffset():z() == 6)
            local a = ModelAttachment.new("hand")
            m:addAttachment(a)
            assert(m:getAttachmentById("hand") == a)
            a:setId("grip")
            assert(m:getAttachmentById("grip") == a and m:getAttachmentById("hand") == nil)

            local v = sm:getVehicle("Cart")
            assert(v:getMass() == 300 and v:getFullName() == "Base.Cart" and v:getModelScale() == 2)
            assert(v:getModel():getFile() == "CartMesh" and v:getModel():getOffset():y() == 1)
            local area = v:getAreaById("Back")
            assert(area == v:getArea(0) and area:getW() == 4)
            area:setW(5)
            assert(v:getArea(0):getW() == 5)
            local part = v:getPartById("Wheel")
            assert(part:getId() == "Wheel" and part:getModelCount() == 1 and part:getModel(0):getFile() == "W")
            assert(v:getPassenger(0):getPosition(0):getOffset():z() == 1)
            assert(v:getZombieType():get(0) == "Farmer" and v:getRandomZombieType() == "Farmer")
            assert(sm:getAllVehicleScripts():size() == 1)

            local c = sm:getCraftRecipe("Carve")
            assert(c:getTime() == 20 and c:hasTag("InHandCraft") and c:getInputCount() == 2)
            local tool = c:getToolLeft()
            assert(tool == c:getInputs():get(0) and tool == c:getProp1() and tool:isKeep() and tool:isToolLeft())
            assert(tool:getParentRecipe() == c and c:getIndexForIO(tool) == 0 and tool:getRecipeLineIndex() == 0)
            assert(tool:getItems():get(0) == "Base.Stone" and tool:getAmount() == 1)
            assert(tool:hasFlag(InputFlag.Prop1) and not tool:hasFlag(InputFlag.IsWorn))
            assert(c:getInputs():get(1):getIntAmount() == 2)
            assert(c:getOutputs():size() == 1 and c:getOutputs():get(0):getAmount() == 1)
            assert(sm:getAllCraftRecipes():contains(c))
            "#,
        )
        .unwrap();
    }
}
