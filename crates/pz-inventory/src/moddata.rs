//! Exclusive item ModData authority. Archive data is an explicit, unowned DTO;
//! runtime access adopts it once into the actual VM table. No editable mirror.
//!
//! Rust archive copies clone table allocations (including table keys and
//! metatables), preserving table aliases. Opaque non-table values retain their
//! references. This is neither native copyModData nor native debug item clone.
use std::collections::{HashMap, VecDeque};

use pz_entity::runtime_load::RuntimeHostError;
use pz_entity::{EntityError, RetiredComponents};
use pz_kahlua::table::{RetiredTableValues, TableAccessError};
use pz_kahlua::table_save::{self, Flavor, TableCodecError};
use pz_kahlua::{LuaError, TableRef, Value};

use crate::io::{Reader, SaveError, Writer};
use crate::lua_table::{LuaKey, LuaTable, LuaValue};
use crate::shared::{self, ItemRef};
use crate::InventoryItem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModDataError {
    Borrowed(&'static str),
    Unsupported(&'static str),
    Native {
        class: &'static str,
        message: String,
    },
}
pub type ModDataResult<T> = Result<T, ModDataError>;
impl std::fmt::Display for ModDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Borrowed(operation) => write!(f, "host borrow conflict: {operation}"),
            Self::Unsupported(operation) => write!(f, "host unsupported: {operation}"),
            Self::Native { class, message } => write!(f, "{class}: {message}"),
        }
    }
}
impl std::error::Error for ModDataError {}
impl ModDataError {
    pub fn to_lua_error(&self) -> LuaError {
        let (class, message) = match self {
            Self::Native { class, message } => (*class, message.clone()),
            Self::Borrowed(operation) => ("pz.host.Borrowed", (*operation).to_owned()),
            Self::Unsupported(operation) => ("pz.host.Unsupported", (*operation).to_owned()),
        };
        LuaError::Runtime { class, message }
    }
    pub fn into_host(self) -> RuntimeHostError {
        match self {
            Self::Borrowed(operation) => RuntimeHostError::Borrowed(operation),
            Self::Unsupported(operation) => RuntimeHostError::Unsupported {
                operation,
                kind: None,
            },
            Self::Native { class, message } => {
                RuntimeHostError::NativeException(EntityError(format!("{class}: {message}")))
            }
        }
    }
    pub fn cast(expected: &'static str) -> Self {
        Self::Native {
            class: "java.lang.ClassCastException",
            message: format!("ModData value cannot be cast to {expected}"),
        }
    }
}
impl From<TableAccessError> for ModDataError {
    fn from(error: TableAccessError) -> Self {
        match error {
            TableAccessError::Borrowed => Self::Borrowed("item ModData table"),
            TableAccessError::TraversalLimit => {
                Self::Unsupported("ModData metatable traversal limit")
            }
        }
    }
}
impl From<TableCodecError> for ModDataError {
    fn from(error: TableCodecError) -> Self {
        match error {
            TableCodecError::Access(error) => error.into(),
            TableCodecError::Native { class, message } => Self::Native { class, message },
            TableCodecError::Unsupported { capability } => Self::Unsupported(capability),
        }
    }
}
impl From<SaveError> for ModDataError {
    fn from(error: SaveError) -> Self {
        Self::Native {
            class: "java.lang.RuntimeException",
            message: error.0,
        }
    }
}
impl From<ModDataError> for SaveError {
    fn from(error: ModDataError) -> Self {
        Self(error.to_string())
    }
}

#[derive(Debug)]
pub enum ItemModData {
    Archive(LuaTable),
    Live(TableRef),
}
impl From<LuaTable> for ItemModData {
    fn from(value: LuaTable) -> Self {
        Self::Archive(value)
    }
}
impl ItemModData {
    /// Physical collector edge, without reading or materializing any body.
    pub fn as_live_slot(&self) -> Option<&TableRef> {
        match self {
            Self::Live(table) => Some(table),
            Self::Archive(_) => None,
        }
    }
    pub fn try_is_empty(&self) -> ModDataResult<bool> {
        match self {
            Self::Archive(table) => Ok(table.entries.is_empty()),
            Self::Live(table) => Ok(table.try_is_empty()?),
        }
    }
    pub fn try_archive_copy(&self) -> ModDataResult<Self> {
        match self {
            Self::Archive(table) => Ok(Self::Archive(table.clone())),
            Self::Live(table) => Ok(Self::Live(copy_table_graph(table)?)),
        }
    }
    pub(crate) fn save(&self, writer: &mut Writer) -> ModDataResult<()> {
        match self {
            Self::Archive(table) => {
                table.save(writer);
                Ok(())
            }
            Self::Live(table) => Ok(table_save::try_save_table(
                table,
                &mut writer.buf,
                Flavor::ByteBuffer,
            )?),
        }
    }
}
impl Clone for ItemModData {
    /// Legacy synchronous archive copy; production callers preflight or use
    /// the fallible item archive operation before invoking derived Clone.
    fn clone(&self) -> Self {
        self.try_archive_copy()
            .expect("item ModData archive copy requires unborrowed tables")
    }
}
impl PartialEq for ItemModData {
    /// Bounded Rust data comparison, never the native ModDataMatches method.
    /// Opaque values compare by the VM's reference/value policy.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Archive(a), Self::Archive(b)) => a == b,
            (Self::Live(a), Self::Live(b)) => table_data_eq(a, b).unwrap_or(false),
            _ => false,
        }
    }
}

fn key_value(key: &LuaKey) -> Value {
    match key {
        LuaKey::Str(value) => Value::str(value),
        LuaKey::Num(value) => Value::Num(*value),
    }
}
fn archive_value(value: &LuaValue, depth: usize) -> ModDataResult<Value> {
    Ok(match value {
        LuaValue::Str(value) => Value::str(value),
        LuaValue::Num(value) => Value::Num(*value),
        LuaValue::Bool(value) => Value::Bool(*value),
        LuaValue::Table(value) => Value::Table(adopt_archive_at(value, depth + 1)?),
        LuaValue::Unsaveable => {
            return Err(ModDataError::Unsupported(
                "archive Unsaveable sentinel has no actual native value",
            ))
        }
    })
}
fn adopt_archive_at(archive: &LuaTable, depth: usize) -> ModDataResult<TableRef> {
    if depth > 64 {
        return Err(ModDataError::Unsupported(
            "archive ModData adoption depth limit",
        ));
    }
    let table = TableRef::new();
    let mut retired = RetiredTableValues::new();
    for (key, value) in &archive.entries {
        table.try_rawset(&key_value(key), &archive_value(value, depth)?, &mut retired)?;
    }
    retire_values(retired);
    Ok(table)
}

/// Deep-copy only mutable table allocations. Key, value and metatable table
/// edges share one memo; opaque values remain actual reference values.
fn copy_table_graph(source: &TableRef) -> ModDataResult<TableRef> {
    let target = TableRef::new();
    let mut memo = HashMap::from([(source.ptr_id(), target.clone())]);
    let mut queue = VecDeque::from([(source.clone(), target.clone())]);
    let mut retired = RetiredTableValues::new();
    let result = (|| {
        while let Some((source, target)) = queue.pop_front() {
            let entries = source.try_entries()?;
            let meta = source.try_metatable()?;
            let mut copy_value = |value: Value| -> ModDataResult<Value> {
                let Value::Table(table) = value else {
                    return Ok(value);
                };
                if let Some(copy) = memo.get(&table.ptr_id()) {
                    return Ok(Value::Table(copy.clone()));
                }
                if memo.len() >= 16_384 {
                    return Err(ModDataError::Unsupported(
                        "archive ModData table graph size limit",
                    ));
                }
                let copy = TableRef::new();
                memo.insert(table.ptr_id(), copy.clone());
                queue.push_back((table, copy.clone()));
                Ok(Value::Table(copy))
            };
            for (key, value) in entries {
                let key = copy_value(key)?;
                let value = copy_value(value)?;
                target.try_rawset(&key, &value, &mut retired)?;
            }
            let meta = match meta {
                Some(meta) => copy_value(Value::Table(meta))?.as_table().cloned(),
                None => None,
            };
            target.try_set_metatable(&meta, &mut retired)?;
        }
        Ok(target)
    })();
    // A failed partial graph may own userdata backedges; never destroy it under
    // an enclosing item/WORLD guard. Memo pins are outer-retired as well.
    let mut pins = RetiredComponents::new();
    pins.push_payload(Box::new(memo));
    pins.push_payload(Box::new(queue));
    if !retired.is_empty() {
        pins.push_payload(Box::new(retired));
    }
    crate::component_host::retire_components(pins);
    result
}
fn table_data_eq(a: &TableRef, b: &TableRef) -> ModDataResult<bool> {
    let mut queue = VecDeque::from([(a.clone(), b.clone())]);
    let mut left = HashMap::new();
    let mut right = HashMap::new();
    while let Some((a, b)) = queue.pop_front() {
        if let Some(id) = left.get(&a.ptr_id()) {
            if *id != b.ptr_id() {
                return Ok(false);
            }
            continue;
        }
        if right.contains_key(&b.ptr_id()) {
            return Ok(false);
        }
        if left.len() >= 16_384 {
            return Err(ModDataError::Unsupported(
                "ModData data comparison graph size limit",
            ));
        }
        left.insert(a.ptr_id(), b.ptr_id());
        right.insert(b.ptr_id(), a.ptr_id());
        let aa = a.try_entries()?;
        let bb = b.try_entries()?;
        if aa.len() != bb.len() {
            return Ok(false);
        }
        for ((ak, av), (bk, bv)) in aa.into_iter().zip(bb) {
            for (x, y) in [(ak, bk), (av, bv)] {
                match (x, y) {
                    (Value::Table(x), Value::Table(y)) => queue.push_back((x, y)),
                    (x, y) if x.raw_equals(&y) => {}
                    _ => return Ok(false),
                }
            }
        }
        match (a.try_metatable()?, b.try_metatable()?) {
            (Some(a), Some(b)) => queue.push_back((a, b)),
            (None, None) => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}
fn retire_values(values: RetiredTableValues) {
    if !values.is_empty() {
        let mut retired = RetiredComponents::new();
        retired.push_payload(Box::new(values));
        crate::component_host::retire_components(retired);
    }
}

impl InventoryItem {
    pub fn mod_data_slot(&self) -> &Option<ItemModData> {
        &self.table
    }
    pub fn take_mod_data_for_gc(&mut self) -> Option<ItemModData> {
        self.table.take()
    }
    pub(crate) fn ensure_live_mod_data(&mut self) -> ModDataResult<TableRef> {
        match self.table.as_ref() {
            Some(ItemModData::Live(table)) => return Ok(table.clone()),
            Some(ItemModData::Archive(table)) => {
                let table = adopt_archive_at(table, 0)?;
                self.table = Some(ItemModData::Live(table.clone()));
                return Ok(table);
            }
            None => {}
        }
        let table = TableRef::new();
        self.table = Some(ItemModData::Live(table.clone()));
        Ok(table)
    }
    pub fn mod_data_value(&self, key: &str) -> ModDataResult<Option<Value>> {
        match &self.table {
            None => Ok(None),
            Some(ItemModData::Live(table)) => {
                let value = table.try_rawget(&Value::str(key))?;
                Ok((!value.is_nil()).then_some(value))
            }
            Some(ItemModData::Archive(table)) => match table.rawget(&LuaKey::Str(key.into())) {
                None => Ok(None),
                Some(LuaValue::Table(_)) => Err(ModDataError::Unsupported(
                    "archive nested table read requires actual adoption",
                )),
                Some(value) => Ok(Some(archive_value(value, 0)?)),
            },
        }
    }
    pub fn set_mod_data_value(&mut self, key: &str, value: Option<Value>) -> ModDataResult<()> {
        let value = value.unwrap_or(Value::Nil);
        let mut retired = RetiredTableValues::new();
        let result = self.ensure_live_mod_data().and_then(|table| {
            table
                .try_rawset(&Value::str(key), &value, &mut retired)
                .map_err(Into::into)
        });
        // Also retain caller-owned input on a failed table borrow/adoption.
        retired.push(value);
        retire_values(retired);
        result
    }
    pub fn mod_data_own_entries(&self) -> ModDataResult<Vec<(Value, Value)>> {
        match &self.table {
            None => Ok(Vec::new()),
            Some(ItemModData::Live(table)) => Ok(table.try_entries()?),
            Some(ItemModData::Archive(table)) => table
                .entries
                .iter()
                .map(|(key, value)| Ok((key_value(key), archive_value(value, 0)?)))
                .collect(),
        }
    }
    pub fn mod_data_string(&self, key: &str) -> ModDataResult<Option<String>> {
        match self.mod_data_value(key)? {
            None => Ok(None),
            Some(Value::Str(value)) => Ok(Some(value.to_string())),
            Some(_) => Err(ModDataError::cast("java.lang.String")),
        }
    }
    pub(crate) fn load_mod_data(
        &mut self,
        reader: &mut Reader<'_>,
        version: i32,
        retired: &mut RetiredComponents,
        notices: &mut Vec<pz_kahlua::java_io::JavaIoNotice>,
    ) -> ModDataResult<()> {
        let table = TableRef::new();
        if let Some(old) = self.table.replace(ItemModData::Live(table.clone())) {
            retired.push_payload(Box::new(old));
        }
        let mut input = pz_kahlua::java_io::JavaReader::new(reader.buf);
        input
            .set_position(reader.pos)
            .map_err(|error| ModDataError::Native {
                class: "java.lang.IllegalArgumentException",
                message: error.to_string(),
            })?;
        let mut values = RetiredTableValues::new();
        let result = table_save::try_load_table(
            &table,
            &mut input,
            version,
            Flavor::ByteBuffer,
            &mut values,
        )
        .map_err(Into::into);
        reader.pos = input.position();
        notices.extend(input.take_notices());
        if !values.is_empty() {
            retired.push_payload(Box::new(values));
        }
        result
    }
}

pub fn get_mod_data_ref(item: &ItemRef) -> ModDataResult<TableRef> {
    let result = item
        .try_borrow_mut()
        .map_err(|_| ModDataError::Borrowed("get item ModData"))
        .and_then(|mut body| body.ensure_live_mod_data());
    shared::observe_item(item);
    result
}
pub fn has_mod_data_ref(item: &ItemRef) -> ModDataResult<bool> {
    let result = (|| {
        let body = item
            .try_borrow()
            .map_err(|_| ModDataError::Borrowed("has item ModData"))?;
        match &body.table {
            Some(table) => Ok(!table.try_is_empty()?),
            None => Ok(false),
        }
    })();
    shared::observe_item(item);
    result
}
pub fn set_mod_data_value_ref(
    item: &ItemRef,
    key: &str,
    value: Option<Value>,
) -> ModDataResult<()> {
    let result = match item.try_borrow_mut() {
        Ok(mut body) => body.set_mod_data_value(key, value),
        Err(_) => {
            let mut retired = RetiredTableValues::new();
            if let Some(value) = value {
                retired.push(value);
            }
            retire_values(retired);
            Err(ModDataError::Borrowed("set item ModData"))
        }
    };
    shared::observe_item(item);
    result
}
pub fn copy_mod_data_ref(item: &ItemRef, source: Option<&TableRef>) -> ModDataResult<()> {
    let mut retired = RetiredTableValues::new();
    let result = (|| {
        let destination = {
            let mut body = item
                .try_borrow_mut()
                .map_err(|_| ModDataError::Borrowed("copy item ModData"))?;
            if let Some(table) = &body.table {
                match table {
                    ItemModData::Live(table) => table.try_wipe(&mut retired)?,
                    ItemModData::Archive(_) => {
                        body.ensure_live_mod_data()?.try_wipe(&mut retired)?;
                    }
                }
            }
            if source.is_none() {
                return Ok(());
            }
            body.ensure_live_mod_data()?
        };
        table_save::copy_table_values(&destination, source, &mut retired)?;
        Ok(())
    })();
    retire_values(retired);
    shared::observe_item(item);
    result
}

#[cfg(test)]
mod tests;

/// Adopt an existing explicit archive slot without creating an absent native
/// slot. The observer runs after this function's item guard ends.
pub fn prepare_existing_mod_data_ref(item: &ItemRef) -> ModDataResult<()> {
    let present = item
        .try_borrow()
        .map_err(|_| ModDataError::Borrowed("prepare item ModData"))?
        .table
        .is_some();
    if present {
        get_mod_data_ref(item)?;
    }
    Ok(())
}
/// Native clothing model selection invokes hasFilter/hasTank only on these
/// clothing subclasses and tags. Call this before the item guard when the
/// caller actually reaches getAlternateModelName (getClothingItem/getVisual
/// return before that phase when their clothing assets are absent).
pub fn prepare_clothing_mod_data_ref(item: &ItemRef) -> ModDataResult<()> {
    let allocate = {
        let body = item
            .try_borrow()
            .map_err(|_| ModDataError::Borrowed("prepare clothing ModData"))?;
        matches!(
            body.kind,
            crate::ItemKind::Clothing(_) | crate::ItemKind::AlarmClockClothing(..)
        ) && (body.is_gas_mask() || body.has_tag(crate::gameplay::tags::SCBA))
    };
    if allocate {
        get_mod_data_ref(item)?;
    } else {
        prepare_existing_mod_data_ref(item)?;
    }
    Ok(())
}

/// Preparation for the static-model/cache pair: the native base and Food
/// getters reach getModData; HandWeapon's sprite override does not. Its
/// static-model exception can still route through the world-model getter.
pub fn prepare_static_model_mod_data_ref(item: &ItemRef) -> ModDataResult<()> {
    let allocate = {
        let body = item
            .try_borrow()
            .map_err(|_| ModDataError::Borrowed("prepare model ModData"))?;
        !matches!(body.kind, crate::ItemKind::Weapon(_))
            || body.has_tag(crate::model::TAG_USE_WORLD_STATIC_MODEL)
    };
    if allocate {
        get_mod_data_ref(item)?;
    } else {
        prepare_existing_mod_data_ref(item)?;
    }
    Ok(())
}
