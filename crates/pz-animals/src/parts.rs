//! `AnimalPartsDefinitions` and `AnimalPart`: read straight from the Lua
//! global `AnimalPartsDefinitions.animals[<type>]`
//! (`media/lua/shared/Definitions/animal/AnimalPartsDefinitions.lua`) on
//! every call, as the original does.

use crate::java::{self, as_table, entries, rawget_int, rawget_str, JResult};
use pz_kahlua::{TableRef, Value};

/// `AnimalPart`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalPart {
    pub item: Option<String>,
    pub min_nb: i32,
    pub max_nb: i32,
    pub nb: i32,
}

impl Default for AnimalPart {
    fn default() -> Self {
        AnimalPart {
            item: None,
            min_nb: -1,
            max_nb: -1,
            nb: -1,
        }
    }
}

impl AnimalPart {
    /// `new AnimalPart(KahluaTableImpl def)`.
    pub fn from_table(def: &TableRef) -> JResult<AnimalPart> {
        Ok(AnimalPart {
            item: rawget_str(def, "item")?,
            min_nb: rawget_int(def, "minNb"),
            max_nb: rawget_int(def, "maxNb"),
            nb: rawget_int(def, "nb"),
        })
    }
}

/// `getAnimalDef(animalType)`: the first entry of
/// `AnimalPartsDefinitions.animals` whose key prints as `animal_type`.
pub fn animal_def(env: &TableRef, animal_type: &str) -> JResult<Option<TableRef>> {
    let Some(definitions) = env.get_str("AnimalPartsDefinitions").as_table().cloned() else {
        return Ok(None);
    };
    let animals = match definitions.rawget(&Value::str("animals")) {
        Value::Nil => return Ok(None),
        v => as_table(&v)?,
    };
    for (k, v) in entries(&animals) {
        if java::to_string(&k) == animal_type {
            return as_table(&v).map(Some);
        }
    }
    Ok(None)
}

/// `getLeather(animalType)`.
pub fn leather(env: &TableRef, animal_type: &str) -> JResult<Option<String>> {
    match animal_def(env, animal_type)? {
        Some(d) => rawget_str(&d, "leather"),
        None => Ok(None),
    }
}

/// `getDef(def, type)`: every entry of `def[type]`, in table order.
pub fn parts_of(def: &TableRef, kind: &str) -> JResult<Vec<AnimalPart>> {
    let parts = match def.rawget(&Value::str(kind)) {
        Value::Nil => return Ok(Vec::new()),
        v => as_table(&v)?,
    };
    entries(&parts)
        .iter()
        .map(|(_, v)| AnimalPart::from_table(&as_table(v)?))
        .collect()
}

/// `getAllPartsDef(animalType)`.
pub fn all_parts_def(env: &TableRef, animal_type: &str) -> JResult<Vec<AnimalPart>> {
    match animal_def(env, animal_type)? {
        Some(d) => parts_of(&d, "parts"),
        None => Ok(Vec::new()),
    }
}

/// `getAllBonesDef(animalType)`.
pub fn all_bones_def(env: &TableRef, animal_type: &str) -> JResult<Vec<AnimalPart>> {
    match animal_def(env, animal_type)? {
        Some(d) => parts_of(&d, "bones"),
        None => Ok(Vec::new()),
    }
}
