//! Tables the `ItemCodeOnCreate` callbacks draw from, read from the
//! installed jar.
//!
//! The original keeps them in static initialisers: the script-object
//! registries (`Brochure`, `Photo`, `Book`, ...), the lists of
//! `ItemGenerationConstants` that point into them, and the string ids of key
//! constants (`ModelKey`, `ItemKey$Drainable`, `FluidKey`, `ItemTag`). A
//! registry's values keep registration order, which is the order in which
//! its class initialiser registers them, so every `Rand.Next(List)` draw
//! lands on the same entry as in the original.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_classfile::{Effect, Jar, Value as J};
use pz_kahlua::{LuaError, LuaResult};

const OBJECTS: &str = "zombie/scripting/objects/";
const GENERATION: &str = "zombie/inventory/ItemGenerationConstants";
const WEIGHTED_LIST: &str = "zombie/util/list/WeightedList";
const CRAFT_RECIPE_KEY: &str = "zombie/scripting/objects/CraftRecipeKey";

/// Registries whose entries are a registered name plus translation keys
/// built by the constructor.
const REGISTRIES: [&str; 20] = [
    "BookSubject",
    "MagazineSubject",
    "Book",
    "Magazine",
    "ComicBook",
    "Brochure",
    "Flier",
    "Newspaper",
    "OldNewspaper",
    "Catalogue",
    "RpgManual",
    "Locket",
    "Doodle",
    "DoodleKids",
    "Postcard",
    "Job",
    "Letter",
    "Business",
    "PetName",
    "Photo",
];

/// One registered object.
#[derive(Debug, Default)]
pub(crate) struct Entry {
    /// The registered name (first constructor argument).
    pub name: String,
    /// Instance string fields the constructor derives from the name, in
    /// constructor order (`translationKey`, `translationInfoKey`, ...).
    pub keys: Vec<(String, String)>,
    /// `Newspaper.issues`.
    pub issues: Vec<String>,
    /// `Magazine.firstYear` or `ComicBook.issues`.
    pub number: i32,
    /// `ComicBook.inPrint`.
    pub in_print: bool,
    /// `Book.cover` (`HARDCOVER`, `SOFTCOVER` or `BOTH`).
    pub cover: String,
    /// Lower-cased names of the `Book`/`Magazine` subjects.
    pub subjects: Vec<String>,
}

impl Entry {
    /// An instance string field, such as `translationKey`.
    pub fn key(&self, field: &str) -> &str {
        self.keys
            .iter()
            .find(|(f, _)| f == field)
            .map_or("", |(_, v)| v)
    }

    /// `Registry.getLocation(this).getPath()`: the lower-cased name.
    pub fn path(&self) -> String {
        self.name.to_lowercase()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Registry {
    pub entries: Vec<Rc<Entry>>,
    by_field: HashMap<String, usize>,
}

impl Registry {
    pub fn field(&self, field: &str) -> Option<&Rc<Entry>> {
        self.by_field.get(field).map(|&i| &self.entries[i])
    }
}

/// `WeightedList.getRandom()` over registered objects.
#[derive(Debug, Default)]
pub(crate) struct Weighted {
    entries: Vec<(Rc<Entry>, i32)>,
    total: i64,
}

impl Weighted {
    /// One `Rand.Next(totalChance)` draw, then the first entry whose running
    /// sum exceeds the roll.
    pub fn get_random(&self) -> LuaResult<Rc<Entry>> {
        let roll = i64::from(pz_core::random::next(self.total as i32));
        let mut sum = 0i64;
        for (entry, weight) in &self.entries {
            sum += i64::from(*weight);
            if roll < sum {
                return Ok(entry.clone());
            }
        }
        Err(LuaError::Runtime {
            message: "Weighted list was empty.".into(),
            class: "java.lang.RuntimeException",
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Tables {
    registries: HashMap<&'static str, Registry>,
    /// `ItemGenerationConstants` plain object lists.
    lists: HashMap<String, Vec<Rc<Entry>>>,
    weighted: HashMap<String, Weighted>,
    /// `ItemGenerationConstants` lists of `CraftRecipeKey` ids.
    recipes: HashMap<String, Vec<String>>,
    /// Ids of key constants, by `(class, field)`.
    keys: HashMap<(&'static str, &'static str), String>,
}

thread_local! {
    static TABLES: RefCell<Option<Rc<Tables>>> = const { RefCell::new(None) };
}

fn unsupported(message: String) -> LuaError {
    LuaError::Runtime {
        message,
        class: "pz.host.Unsupported",
    }
}

impl Tables {
    pub fn registry(&self, class: &'static str) -> LuaResult<&Registry> {
        self.registries.get(class).ok_or_else(|| {
            unsupported(format!(
                "{class} registry not loaded from the installed jar"
            ))
        })
    }

    /// `Registries.<X>.values()`.
    pub fn values(&self, class: &'static str) -> LuaResult<&[Rc<Entry>]> {
        Ok(&self.registry(class)?.entries)
    }

    pub fn list(&self, name: &str) -> LuaResult<&[Rc<Entry>]> {
        self.lists.get(name).map(Vec::as_slice).ok_or_else(|| {
            unsupported(format!(
                "ItemGenerationConstants.{name} not loaded from the installed jar"
            ))
        })
    }

    pub fn weighted(&self, name: &str) -> LuaResult<&Weighted> {
        self.weighted.get(name).ok_or_else(|| {
            unsupported(format!(
                "ItemGenerationConstants.{name} not loaded from the installed jar"
            ))
        })
    }

    pub fn recipes(&self, name: &str) -> LuaResult<&[String]> {
        self.recipes.get(name).map(Vec::as_slice).ok_or_else(|| {
            unsupported(format!(
                "ItemGenerationConstants.{name} not loaded from the installed jar"
            ))
        })
    }

    /// The id of a key constant such as `ModelKey.PAPERWORK_1`.
    pub fn key(&self, class: &'static str, field: &'static str) -> LuaResult<&str> {
        self.keys
            .get(&(class, field))
            .map(String::as_str)
            .ok_or_else(|| {
                unsupported(format!("{class}.{field} not loaded from the installed jar"))
            })
    }
}

/// The tables read at start-up, if an installed jar was loaded.
pub(crate) fn maybe_tables() -> Option<Rc<Tables>> {
    TABLES.with(|t| t.borrow().clone())
}

/// The tables read at start-up, or a host capability error.
pub(crate) fn tables() -> LuaResult<Rc<Tables>> {
    TABLES.with(|t| t.borrow().clone()).ok_or_else(|| {
        unsupported("ItemCodeOnCreate tables not loaded from the installed jar".into())
    })
}

/// Key constants the callbacks read: `(class, field, how the id is built)`.
const KEYS: [(&str, &str); 25] = [
    ("ModelKey", "PAPERWORK_1"),
    ("ModelKey", "PAPERWORK_2"),
    ("ModelKey", "PAPERWORK_3"),
    ("ModelKey", "PAPERWORK_4"),
    ("ModelKey", "PAPERWORK_5"),
    ("ModelKey", "PAPERWORK_6"),
    ("ModelKey", "STOCK_CERTIFICATE_2"),
    ("ModelKey", "HOTTIE_ZGROUND_2"),
    ("ModelKey", "HOTTIE_Z2"),
    ("ModelKey", "SCRATCH_TICKET_WINNER"),
    ("ItemKey$Drainable", "GASMASK_FILTER"),
    ("ItemKey$Drainable", "RESPIRATOR_FILTERS"),
    ("ItemKey$Drainable", "RAG_FILTER"),
    ("ItemKey$Drainable", "OXYGEN_TANK"),
    ("FluidKey", "SODA_POP"),
    ("FluidKey", "COLA"),
    ("FluidKey", "GINGER_ALE"),
    ("FluidKey", "SODA_LIME"),
    ("ItemTag", "NEW"),
    ("ItemTag", "NEWSPAPER_NEW"),
    ("ItemTag", "GAS_MASK"),
    ("ItemTag", "RESPIRATOR"),
    ("ItemTag", "IMPROVISED_GAS_MASK"),
    ("ItemTag", "SCBA"),
    ("ItemTag", "HARDCOVER"),
];

/// More item tags, kept apart so the array above stays a plain list.
const MORE_TAGS: [&str; 3] = ["HOLLOW_BOOK", "FANCY_BOOK", "SOFTCOVER"];

/// Reads the tables from the jar. A table that cannot be read leaves the
/// callbacks that need it reporting a host capability error.
pub(crate) fn register_from_jar(jar: Option<&Jar>) {
    let Some(jar) = jar else { return };
    let tables = load(jar);
    TABLES.with(|t| *t.borrow_mut() = Some(Rc::new(tables)));
}

fn log(message: &str) {
    pz_core::debug::with(|d| d.log(pz_core::debug::DebugType::General, message));
}

fn load(jar: &Jar) -> Tables {
    let mut tables = Tables::default();
    for class in REGISTRIES {
        match load_registry(jar, class, &tables) {
            Ok(registry) => {
                tables.registries.insert(class, registry);
            }
            Err(e) => log(&format!(
                "ItemCodeOnCreate: {class} registry not loaded: {e}"
            )),
        }
    }
    if let Err(e) = load_generation(jar, &mut tables) {
        log(&format!(
            "ItemCodeOnCreate: ItemGenerationConstants not loaded: {e}"
        ));
    }
    for (class, field) in KEYS
        .iter()
        .copied()
        .chain(MORE_TAGS.iter().map(|f| ("ItemTag", *f)))
    {
        match load_key(jar, class, field) {
            Ok(id) => {
                tables.keys.insert((class, field), id);
            }
            Err(e) => log(&format!(
                "ItemCodeOnCreate: {class}.{field} not loaded: {e}"
            )),
        }
    }
    tables
}

fn err(e: pz_classfile::Error) -> String {
    format!("{e:?}")
}

/// `name` of a `registerBase`/factory/constructor value whose first
/// argument is the name.
fn first_string(value: &J) -> Option<&str> {
    match value {
        J::Call { args, .. } | J::New { args, .. } => args.first().and_then(J::as_str),
        _ => None,
    }
}

/// The id string of a key constant. `ItemTag` ids are the registered
/// location of the name, in the `base` namespace.
fn load_key(jar: &Jar, class: &str, field: &str) -> Result<String, String> {
    let full = format!("{OBJECTS}{class}");
    let value = pz_classfile::static_value(jar, &full, field)
        .map_err(err)?
        .ok_or_else(|| "no static initialiser".to_string())?;
    let name = first_string(&value).ok_or_else(|| format!("unexpected initialiser {value:?}"))?;
    Ok(if class == "ItemTag" {
        format!("base:{}", name.to_lowercase())
    } else {
        name.to_string()
    })
}

fn static_names(args: &[J]) -> Vec<(&str, &str)> {
    args.iter()
        .filter_map(|a| match a {
            J::Static { class, name } => Some((class.as_str(), name.as_str())),
            _ => None,
        })
        .collect()
}

/// The elements of `List.of(...)`: the arguments, or the elements of the
/// single array argument.
fn list_items(value: &J) -> Option<Vec<J>> {
    let J::Call {
        class, name, args, ..
    } = value
    else {
        return None;
    };
    if class != "java/util/List" || name != "of" {
        return None;
    }
    match args.as_slice() {
        [J::Array(items)] => Some(items.clone()),
        items => Some(items.to_vec()),
    }
}

fn load_registry(jar: &Jar, simple: &str, loaded: &Tables) -> Result<Registry, String> {
    let class = format!("{OBJECTS}{simple}");
    let cf = pz_classfile::load_class(jar, &class).map_err(err)?;
    // The constructor's `field = "Prefix_" + name + "_suffix"` stores.
    let templates: Vec<(String, String)> = pz_classfile::method_effects(&cf, "<init>")
        .map_err(err)?
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::PutField {
                field,
                value: J::Call {
                    name, descriptor, ..
                },
                ..
            } if name == "makeConcatWithConstants" => Some((field, descriptor)),
            _ => None,
        })
        .collect();
    let mut registry = Registry::default();
    for effect in pz_classfile::method_effects(&cf, "<clinit>").map_err(err)? {
        let Effect::PutStatic(init) = effect else {
            continue;
        };
        if init.class != class {
            continue;
        }
        let J::Call {
            class: owner,
            name: factory,
            args,
            ..
        } = &init.value
        else {
            continue;
        };
        if *owner != class || (factory != "registerBase" && factory != "register") {
            continue;
        }
        let name = args
            .first()
            .and_then(J::as_str)
            .ok_or_else(|| format!("{}: no name", init.field))?;
        let mut entry = Entry {
            name: name.to_string(),
            keys: templates
                .iter()
                .map(|(f, recipe)| (f.clone(), recipe.replace('\u{1}', name)))
                .collect(),
            ..Entry::default()
        };
        match simple {
            "Newspaper" => {
                let issues = args
                    .get(1)
                    .and_then(list_items)
                    .ok_or_else(|| format!("{}: no issues", init.field))?;
                entry.issues = issues
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect();
            }
            "ComicBook" => {
                entry.number = int_arg(args, 1)?;
                entry.in_print = int_arg(args, 2)? != 0;
            }
            "Magazine" => {
                entry.number = int_arg(args, 1)?;
                entry.subjects = subject_names(loaded, "MagazineSubject", args.get(2))?;
            }
            "Book" => {
                entry.cover = match args.get(1) {
                    Some(J::Static { name, .. }) => name.clone(),
                    other => return Err(format!("{}: cover {other:?}", init.field)),
                };
                entry.subjects = subject_names(loaded, "BookSubject", args.get(2))?;
            }
            _ => {}
        }
        registry.by_field.insert(init.field, registry.entries.len());
        registry.entries.push(Rc::new(entry));
    }
    if registry.entries.is_empty() {
        return Err("no registered entries".into());
    }
    Ok(registry)
}

fn int_arg(args: &[J], index: usize) -> Result<i32, String> {
    match args.get(index) {
        Some(J::Int(n)) => i32::try_from(*n).map_err(|_| format!("argument {index}: {n}")),
        other => Err(format!("argument {index}: {other:?}")),
    }
}

/// The lower-cased names of the subjects in a `register` varargs array.
fn subject_names(
    loaded: &Tables,
    class: &'static str,
    array: Option<&J>,
) -> Result<Vec<String>, String> {
    let Some(J::Array(items)) = array else {
        return Err(format!("{class} subjects: {array:?}"));
    };
    let registry = loaded
        .registries
        .get(class)
        .ok_or_else(|| format!("{class} not loaded"))?;
    items
        .iter()
        .map(|item| match item {
            J::Static { name, .. } => registry
                .field(name)
                .map(|e| e.path())
                .ok_or_else(|| format!("{class}.{name} is not registered")),
            other => Err(format!("{class} subject {other:?}")),
        })
        .collect()
}

/// `ItemGenerationConstants.<clinit>`: the lists that point into the
/// registries and the `CraftRecipeKey` constants.
fn load_generation(jar: &Jar, tables: &mut Tables) -> Result<(), String> {
    let recipe_ids = load_recipe_ids(jar)?;
    let cf = pz_classfile::load_class(jar, GENERATION).map_err(err)?;
    let effects = pz_classfile::method_effects(&cf, "<clinit>").map_err(err)?;
    let resolve = |tables: &Tables, class: &str, field: &str| -> Result<Rc<Entry>, String> {
        let simple = class
            .strip_prefix(OBJECTS)
            .ok_or_else(|| format!("unexpected class {class}"))?;
        tables
            .registries
            .get(simple)
            .and_then(|r| r.field(field))
            .cloned()
            .ok_or_else(|| format!("{class}.{field} is not registered"))
    };
    for effect in effects {
        match effect {
            Effect::PutStatic(init) if init.class == GENERATION => match &init.value {
                J::New { class, .. } if class == WEIGHTED_LIST => {
                    tables.weighted.insert(init.field, Weighted::default());
                }
                value => {
                    let Some(items) = list_items(value) else {
                        continue;
                    };
                    let statics = static_names(&items);
                    if statics.len() != items.len() {
                        continue;
                    }
                    // A list of `CraftRecipeKey` constants or of registered objects.
                    if statics.iter().all(|(class, _)| *class == CRAFT_RECIPE_KEY) {
                        let ids: Result<Vec<String>, String> = statics
                            .iter()
                            .map(|(_, f)| {
                                recipe_ids
                                    .get(*f)
                                    .cloned()
                                    .ok_or_else(|| format!("CraftRecipeKey.{f}"))
                            })
                            .collect();
                        match ids {
                            Ok(ids) => {
                                tables.recipes.insert(init.field, ids);
                            }
                            Err(e) => log(&format!("ItemCodeOnCreate: {}: {e}", init.field)),
                        }
                    } else {
                        let entries: Result<Vec<Rc<Entry>>, String> =
                            statics.iter().map(|(c, f)| resolve(tables, c, f)).collect();
                        match entries {
                            Ok(entries) => {
                                tables.lists.insert(init.field, entries);
                            }
                            Err(e) => log(&format!("ItemCodeOnCreate: {}: {e}", init.field)),
                        }
                    }
                }
            },
            Effect::Call(J::Call {
                class, name, args, ..
            }) if class == WEIGHTED_LIST && name == "add" => {
                let [J::Static {
                    class: owner,
                    name: list,
                }, J::Static {
                    class: object,
                    name: field,
                }, J::Int(chance)] = args.as_slice()
                else {
                    continue;
                };
                if owner != GENERATION || !object.starts_with(OBJECTS) {
                    continue;
                }
                if !tables.weighted.contains_key(list.as_str()) {
                    continue;
                }
                let entry = resolve(tables, object, field)?;
                let chance =
                    i32::try_from(*chance).map_err(|_| format!("{list}: chance {chance}"))?;
                if let Some(target) = tables.weighted.get_mut(list.as_str()) {
                    target.entries.push((entry, chance));
                    target.total += i64::from(chance);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `CraftRecipeKey.<FIELD>` to its `id`.
fn load_recipe_ids(jar: &Jar) -> Result<HashMap<String, String>, String> {
    let cf = pz_classfile::load_class(jar, CRAFT_RECIPE_KEY).map_err(err)?;
    Ok(pz_classfile::method_effects(&cf, "<clinit>")
        .map_err(err)?
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::PutStatic(init) if init.class == CRAFT_RECIPE_KEY => {
                first_string(&init.value).map(|id| (init.field.clone(), id.to_string()))
            }
            _ => None,
        })
        .collect())
}

/// Installs tables for tests.
#[cfg(test)]
pub(crate) fn install_for_test(tables: Tables) {
    TABLES.with(|t| *t.borrow_mut() = Some(Rc::new(tables)));
}

#[cfg(test)]
impl Tables {
    pub fn with_registry(
        mut self,
        class: &'static str,
        names: &[&str],
        keys: &[(&str, &str)],
    ) -> Self {
        let mut registry = Registry::default();
        for name in names {
            let entry = Entry {
                name: (*name).to_string(),
                keys: keys
                    .iter()
                    .map(|(f, r)| ((*f).to_string(), r.replace('\u{1}', name)))
                    .collect(),
                ..Entry::default()
            };
            registry
                .by_field
                .insert(name.to_uppercase(), registry.entries.len());
            registry.entries.push(Rc::new(entry));
        }
        self.registries.insert(class, registry);
        self
    }

    pub fn with_entries(mut self, class: &'static str, entries: Vec<Entry>) -> Self {
        let mut registry = Registry::default();
        for entry in entries {
            registry
                .by_field
                .insert(entry.name.to_uppercase(), registry.entries.len());
            registry.entries.push(Rc::new(entry));
        }
        self.registries.insert(class, registry);
        self
    }

    pub fn with_list(mut self, name: &str, class: &'static str, names: &[&str]) -> Self {
        let registry = &self.registries[class];
        let entries = names
            .iter()
            .map(|n| registry.field(&n.to_uppercase()).unwrap().clone())
            .collect();
        self.lists.insert(name.to_string(), entries);
        self
    }

    pub fn with_weighted(
        mut self,
        name: &str,
        class: &'static str,
        entries: &[(&str, i32)],
    ) -> Self {
        let registry = &self.registries[class];
        let mut list = Weighted::default();
        for (n, w) in entries {
            list.entries
                .push((registry.field(&n.to_uppercase()).unwrap().clone(), *w));
            list.total += i64::from(*w);
        }
        self.weighted.insert(name.to_string(), list);
        self
    }

    pub fn with_recipes(mut self, name: &str, ids: &[&str]) -> Self {
        self.recipes.insert(
            name.to_string(),
            ids.iter().map(|s| (*s).to_string()).collect(),
        );
        self
    }

    pub fn with_key(mut self, class: &'static str, field: &'static str, id: &str) -> Self {
        self.keys.insert((class, field), id.to_string());
        self
    }
}
