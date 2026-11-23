//! The character's live `IsoObject.table`. Saved `MovingBase` records
//! carry a detached table; the runtime owner keeps Lua table identity.

use super::GameCharacter;
use pz_inventory::lua_table::LuaTable;
use pz_kahlua::java_io::JavaReader;
use pz_kahlua::table_save::{load_table, save_table, Flavor};
use pz_kahlua::{TableRef, Value};
use pz_scripts::{ScriptError, ScriptResult};

// The modern table key/value format has been unchanged since version 25.
const TABLE_FORMAT_VERSION: i32 = 25;

fn table_error(error: impl std::fmt::Display) -> ScriptError {
    ScriptError::new(format!("KahluaTableImpl: {error}"))
}

impl<I: 'static> GameCharacter<I> {
    /// `getModData()`: create once and return the same mutable Lua table.
    pub fn get_mod_data(&mut self) -> TableRef {
        self.mod_data.get_or_insert_with(TableRef::new).clone()
    }

    /// `hasModData()`: allocating an empty table does not count.
    pub fn has_mod_data(&self) -> bool {
        self.mod_data.as_ref().is_some_and(|table| !table.is_empty())
    }

    pub fn music_intensity_event_data(&self, key: &str) -> Value {
        self.mod_data.as_ref().and_then(|data| {
            data.get_str("MusicIntensityEvent").as_table().map(|events| events.get_str(key))
        }).unwrap_or_default()
    }

    pub fn set_music_intensity_event_data(&mut self, key: &str, value: Value) {
        if key.trim().is_empty() { return }
        let table = self.mod_data.as_ref().map(|data| data.get_str("MusicIntensityEvent"));
        let events = match table {
            Some(Value::Table(table)) => table,
            _ if value.is_nil() => return,
            _ => {
                let events = TableRef::new();
                self.get_mod_data().set_str("MusicIntensityEvent", events.clone());
                events
            }
        };
        events.set_str(key, value);
    }

    pub fn clear_health_panel_bite_event(&mut self) {
        if self.is_player() || self.is_animal() {
            self.set_music_intensity_event_data("HealthPanel_SeeBite", Value::Nil);
        }
    }

    /// Import a detached save table into the current live table. Loading
    /// preserves a retained top-level Lua reference, as `table.load` does.
    pub fn load_mod_data(&mut self, saved: &LuaTable) -> ScriptResult {
        let mut output = pz_inventory::io::Writer::new();
        saved.save(&mut output);
        load_table(
            &self.get_mod_data(),
            &mut JavaReader::new(&output.buf),
            TABLE_FORMAT_VERSION,
            Flavor::ByteBuffer,
        )
        .map_err(table_error)
    }

    /// A detached view for save-data callers. Lua mutations remain owned
    /// by the live table, including edits through retained nested tables.
    pub fn saved_mod_data(&self) -> ScriptResult<Option<LuaTable>> {
        let Some(table) = self.mod_data.as_ref() else { return Ok(None) };
        let bytes = serialize(table)?;
        LuaTable::load(&mut pz_inventory::io::Reader::new(&bytes), TABLE_FORMAT_VERSION)
            .map(Some)
            .map_err(table_error)
    }

    /// A nonempty live table still writes a presence flag when every
    /// entry is filtered out by the original table serializer.
    pub(crate) fn mod_data_bytes(&self) -> ScriptResult<Option<Vec<u8>>> {
        self.mod_data.as_ref().filter(|table| !table.is_empty()).map(serialize).transpose()
    }
}

fn serialize(table: &TableRef) -> ScriptResult<Vec<u8>> {
    let mut output = Vec::new();
    save_table(table, &mut output, Flavor::ByteBuffer).map_err(table_error)?;
    Ok(output)
}
