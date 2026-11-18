//! `IsoObject.table` (`getModData()`) as a live Lua table.
//!
//! A loaded object keeps its mod data as a codec value. The first
//! `getModData()` from Lua adopts it into a live table (or makes an empty
//! one, as the original does). From then on the live table is the only
//! state: the codec field is empty, and save and copy read the live table.

use std::borrow::Cow;

use pz_inventory::io::{SaveError, SaveResult};
use pz_inventory::moddata::{adopt_archive, live_to_archive};
use pz_kahlua::TableRef;

use crate::objects::IsoObject;
use crate::save::lua_table::LuaTable;

/// The live mod-data table. Equality is value equality of the saved form,
/// as for the codec value.
#[derive(Debug)]
pub struct LiveModData(pub TableRef);

impl PartialEq for LiveModData {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
            || matches!(
                (live_to_archive(&self.0), live_to_archive(&other.0)),
                (Ok(a), Ok(b)) if a == b
            )
    }
}

impl IsoObject {
    /// `getModData()`: the live table, made on first use.
    pub fn install_runtime_mod_data(&mut self) -> SaveResult<TableRef> {
        if let Some(LiveModData(table)) = &self.runtime_mod_data {
            return Ok(table.clone());
        }
        let table = match &self.mod_data {
            Some(archive) => adopt_archive(archive)?,
            None => TableRef::new(),
        };
        self.mod_data = None;
        self.runtime_mod_data = Some(LiveModData(table.clone()));
        Ok(table)
    }

    /// `hasModData()`: a table that is not empty.
    pub fn has_mod_data(&self) -> SaveResult<bool> {
        match &self.runtime_mod_data {
            Some(LiveModData(table)) => Ok(!table
                .try_is_empty()
                .map_err(|_| SaveError("IsoObject ModData: BorrowedTable".into()))?),
            None => Ok(self.mod_data.as_ref().is_some_and(|t| !t.entries.is_empty())),
        }
    }

    /// The mod data as a saved value: the codec value, or a snapshot of the
    /// live table.
    pub fn saved_mod_data(&self) -> SaveResult<Option<Cow<'_, LuaTable>>> {
        match &self.runtime_mod_data {
            Some(LiveModData(table)) => Ok(Some(Cow::Owned(live_to_archive(table)?))),
            None => Ok(self.mod_data.as_ref().map(Cow::Borrowed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::objects::{IsoObject, ObjectKind};
    use crate::save::lua_table::{LuaKey, LuaTable, LuaValue};
    use pz_kahlua::Value;

    #[test]
    fn install_adopts_the_codec_table_and_save_reads_the_live_one() {
        let mut o = IsoObject::new(1, ObjectKind::Object, None);
        let mut saved = LuaTable::default();
        saved.entries.push((LuaKey::Str("crate_customContainerName".into()), LuaValue::Str("Mine".into())));
        o.mod_data = Some(saved);
        let table = o.install_runtime_mod_data().unwrap();
        assert!(o.mod_data.is_none());
        assert_eq!(table.get_str("crate_customContainerName").as_str(), Some("Mine"));
        assert!(table.ptr_eq(&o.install_runtime_mod_data().unwrap()));
        table.set_str("n", Value::Num(2.0));
        let saved = o.saved_mod_data().unwrap().unwrap().into_owned();
        assert_eq!(saved.entries.len(), 2);
        assert!(o.has_mod_data().unwrap());
        let copy = o.clone();
        assert!(copy.runtime_mod_data.is_none());
        assert_eq!(copy.mod_data.as_ref().unwrap().entries.len(), 2);
    }

    #[test]
    fn get_mod_data_on_an_object_without_one_makes_an_empty_table() {
        let mut o = IsoObject::new(1, ObjectKind::Object, None);
        assert!(!o.has_mod_data().unwrap());
        o.install_runtime_mod_data().unwrap();
        assert!(!o.has_mod_data().unwrap());
        assert_eq!(o.saved_mod_data().unwrap().unwrap().entries.len(), 0);
    }
}
