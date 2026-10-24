//! `zombie.SandboxOptions`: the sandbox settings of a game, their
//! `SandboxVars` Lua table, preset files and derived values.
//!
//! The option list itself is read from the installed jar ([`defs`]); the
//! engine pieces the original pokes from here (`GameTime`, the world, the
//! save file) are outside this crate and are noted where they apply.

pub mod defs;

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pz_core::config::{java_split, ConfigFile, ConfigObject, ConfigOption, OptionKind as CfgKind};
use pz_kahlua::{TableRef, Value};
use pz_text::jfmt::JavaArg;
use pz_text::Translator;

pub use defs::{OptionDef, OptionKind};

/// An enum value index outside `1..=numValues` (the original's
/// `ArrayIndexOutOfBoundsException`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange;

/// `SandboxOptions.FIRST_YEAR`.
pub const FIRST_YEAR: i32 = 1993;
/// `SandboxOptions.SANDBOX_VERSION`.
pub const SANDBOX_VERSION: i32 = 6;

/// One `SandboxOptions.*SandboxOption`: a config option plus its place in
/// `SandboxVars` and its translation keys.
#[derive(Clone, Debug)]
pub struct SandboxOption {
    pub config: ConfigOption,
    /// `EnumSandboxOption` (an `IntegerConfigOption` from 1 to `numValues`).
    pub is_enum: bool,
    /// `StrongEnumSandboxOption`: the Java enum's constants.
    pub enum_constants: Option<Vec<String>>,
    pub translation: Option<String>,
    pub table_name: Option<String>,
    pub short_name: String,
    pub custom: bool,
    pub page_name: Option<String>,
    pub value_translation: Option<String>,
}

/// `SandboxOptions.parseName`: "Table.Name" splits into table and short name.
fn parse_name(name: &str) -> (Option<String>, String) {
    if name.contains('.') {
        let ss = java_split(name, ".");
        if ss.len() == 2 {
            return (Some(ss[0].to_string()), ss[1].to_string());
        }
    }
    (None, name.to_string())
}

/// A Lua value as the Java object Kahlua hands to `setValueFromObject`.
pub fn config_object(v: &Value) -> ConfigObject {
    match v {
        Value::Nil => ConfigObject::Null,
        Value::Bool(b) => ConfigObject::Bool(*b),
        Value::Num(n) => ConfigObject::Double(*n),
        Value::Str(s) => ConfigObject::Str(s.to_string()),
        other => ConfigObject::Other(pz_kahlua::exec::java_value_of(other)),
    }
}

/// A config value as the Lua value Kahlua converts it to.
pub fn lua_value(o: &ConfigObject) -> Value {
    match o {
        ConfigObject::Null => Value::Nil,
        ConfigObject::Bool(b) => Value::Bool(*b),
        ConfigObject::Double(d) => Value::Num(*d),
        ConfigObject::Str(s) | ConfigObject::Other(s) => Value::from(s.clone()),
    }
}

fn int_default(c: &ConfigOption) -> i32 {
    match &c.kind {
        CfgKind::Integer(i) => i.default_value,
        _ => 0,
    }
}

impl SandboxOption {
    pub fn from_def(def: &OptionDef) -> Self {
        let (table_name, short_name) = parse_name(&def.name);
        let (config, is_enum, enum_constants) = match &def.kind {
            OptionKind::Boolean { default } => {
                (ConfigOption::boolean(&def.name, *default), false, None)
            }
            OptionKind::Integer { min, max, default } => (
                ConfigOption::integer(&def.name, *min, *max, *default),
                false,
                None,
            ),
            OptionKind::Double { min, max, default } => (
                ConfigOption::double(&def.name, *min, *max, *default),
                false,
                None,
            ),
            OptionKind::Enum {
                num_values,
                default,
            } => (
                ConfigOption::integer(&def.name, 1, *num_values, *default),
                true,
                None,
            ),
            OptionKind::StrongEnum { constants, default } => {
                let ordinal = constants.iter().position(|c| c == default).unwrap_or(0) as i32;
                let n = constants.len() as i32;
                (
                    ConfigOption::integer(&def.name, 1, n, ordinal + 1),
                    true,
                    Some(constants.clone()),
                )
            }
            OptionKind::String {
                default,
                max_length,
            } => (
                ConfigOption::string_max(&def.name, default, *max_length),
                false,
                None,
            ),
        };
        SandboxOption {
            config,
            is_enum,
            enum_constants,
            translation: def.translation.clone(),
            table_name,
            short_name,
            custom: false,
            page_name: None,
            value_translation: def.value_translation.clone(),
        }
    }

    pub fn name(&self) -> &str {
        self.config.name()
    }

    /// `getType()`: "enum" for enum options, else the config type.
    pub fn get_type(&self) -> &'static str {
        if self.is_enum {
            "enum"
        } else {
            self.config.get_type()
        }
    }

    /// `getNumValues()` (`max` of the integer option).
    pub fn num_values(&self) -> i32 {
        match &self.config.kind {
            CfgKind::Integer(i) => i.max,
            _ => 0,
        }
    }

    fn translation_key(&self) -> &str {
        self.translation.as_deref().unwrap_or(&self.short_name)
    }

    /// `getTranslatedName()`.
    pub fn translated_name(&self, t: &Translator) -> String {
        t.get_text(&format!("Sandbox_{}", self.translation_key()), &[])
    }

    /// `getTooltip()`: the `_tooltip` text; for enum options also
    /// "Default: <value>", joined with a literal `\n`.
    pub fn tooltip(&self, t: &Translator) -> Option<String> {
        let s1 = t.get_text_or_null(&format!("Sandbox_{}_tooltip", self.translation_key()), &[]);
        if !self.is_enum {
            return s1;
        }
        let s2 = self
            .value_translation_by_index_or_null(t, int_default(&self.config))
            .ok()
            .flatten()
            .map(|v| t.get_text("Sandbox_Default", &[JavaArg::Str(v)]));
        match (s1, s2) {
            (None, s2) => s2,
            (s1, None) => s1,
            (Some(a), Some(b)) => Some(format!("{a}\\n{b}")),
        }
    }

    /// `getValueTranslation()`.
    pub fn value_translation(&self) -> &str {
        self.value_translation
            .as_deref()
            .unwrap_or_else(|| self.translation_key())
    }

    /// `getValueTranslationByIndex(index)`; `Err` where the original throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn value_translation_by_index(&self, t: &Translator, index: i32) -> Result<String, OutOfRange> {
        if index < 1 || index > self.num_values() {
            return Err(OutOfRange);
        }
        Ok(t.get_text(
            &format!("Sandbox_{}_option{}", self.value_translation(), index),
            &[],
        ))
    }

    /// `getValueTranslationByIndexOrNull(index)`.
    pub fn value_translation_by_index_or_null(
        &self,
        t: &Translator,
        index: i32,
    ) -> Result<Option<String>, OutOfRange> {
        if index < 1 || index > self.num_values() {
            return Err(OutOfRange);
        }
        Ok(t.get_text_or_null(
            &format!("Sandbox_{}_option{}", self.value_translation(), index),
            &[],
        ))
    }

    /// `StrongEnumSandboxOption.getEnumValue()`: the constant's name.
    pub fn enum_value(&self) -> Option<&str> {
        let constants = self.enum_constants.as_ref()?;
        let ordinal = self.config.get_int() - 1;
        usize::try_from(ordinal)
            .ok()
            .and_then(|i| constants.get(i))
            .map(String::as_str)
    }

    /// `fromTable(table)`: reads `table[short]` or `table[tableName][short]`.
    pub fn from_table(&mut self, table: &TableRef) {
        let table = match &self.table_name {
            Some(tn) => match table.rawget(&Value::str(tn)) {
                Value::Table(t) => t,
                _ => return,
            },
            None => table.clone(),
        };
        let o = table.rawget(&Value::str(&self.short_name));
        if !o.is_nil() {
            self.config.set_value_from_object(&config_object(&o));
        }
    }

    /// `toTable(table)`: writes the value, creating the sub-table if needed.
    pub fn to_table(&self, table: &TableRef) {
        let table = match &self.table_name {
            Some(tn) => match table.rawget(&Value::str(tn)) {
                Value::Table(t) => t,
                _ => {
                    let t = TableRef::new();
                    table.rawset(Value::str(tn), Value::Table(t.clone()));
                    t
                }
            },
            None => table.clone(),
        };
        table.rawset(
            Value::str(&self.short_name),
            lua_value(&self.config.get_value_as_object()),
        );
    }

    /// `StringConfigOption.getSplitCSVList()`: the value without whitespace,
    /// split on commas (as a set).
    pub fn split_csv_list(&self) -> Vec<String> {
        let v: String = self
            .config
            .get_string()
            .chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r'))
            .collect();
        if v.is_empty() {
            return vec![String::new()];
        }
        let mut parts: Vec<String> = v.split(',').map(str::to_string).collect();
        while parts.len() > 1 && parts.last().is_some_and(String::is_empty) {
            parts.pop();
        }
        parts.sort();
        parts.dedup();
        parts
    }
}

/// `Rand.Next(min, max)` for the shutoff randomisers (randomly seeded in
/// the original too).
fn rand_between(min: i32, max: i32) -> i32 {
    thread_local! {
        static SEED: Cell<u64> = Cell::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
                | 1,
        );
    }
    if max == min {
        return min;
    }
    let (lo, hi) = if min > max { (max, min) } else { (min, max) };
    let r = SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    });
    lo + (r % (hi - lo) as u64) as i32
}

/// `zombie.SandboxOptions`.
#[derive(Clone, Debug, Default)]
pub struct SandboxOptions {
    pub options: Vec<SandboxOption>,
    by_name: HashMap<String, usize>,
    /// Indices of options added through `newCustomOption`.
    custom: Vec<usize>,
    /// `speed`.
    pub speed: i32,
}

impl SandboxOptions {
    /// The option list with every option at its built-in default. The
    /// original constructor then runs `defines.lua`, loads the
    /// "Apocalypse" preset and makes it the defaults ([`Self::finish_init`]).
    pub fn new(defs: &[OptionDef]) -> Self {
        let mut s = SandboxOptions {
            speed: 3,
            ..Default::default()
        };
        for d in defs {
            s.add_option(SandboxOption::from_def(d));
        }
        s
    }

    fn add_option(&mut self, o: SandboxOption) {
        self.by_name
            .insert(o.name().to_string(), self.options.len());
        self.options.push(o);
    }

    /// The constructor's tail: apply the "Apocalypse" preset table (as
    /// `loadGameFile` does) and make the result the defaults.
    pub fn finish_init(&mut self, apocalypse: Option<&TableRef>) {
        if let Some(t) = apocalypse {
            self.apply_table(t);
        }
        self.set_defaults_to_current_values();
    }

    /// `fromTable` for every option (the body of `loadGameFile`).
    pub fn apply_table(&mut self, t: &TableRef) {
        for o in &mut self.options {
            o.from_table(t);
        }
    }

    pub fn num_options(&self) -> usize {
        self.options.len()
    }

    pub fn option_by_index(&self, i: usize) -> Option<&SandboxOption> {
        self.options.get(i)
    }

    pub fn option_by_name(&self, name: &str) -> Option<&SandboxOption> {
        self.by_name.get(name).map(|&i| &self.options[i])
    }

    pub fn option_by_name_mut(&mut self, name: &str) -> Option<&mut SandboxOption> {
        self.by_name
            .get(name)
            .copied()
            .map(move |i| &mut self.options[i])
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    fn int(&self, name: &str) -> i32 {
        self.option_by_name(name).map_or(0, |o| o.config.get_int())
    }

    fn bool(&self, name: &str) -> bool {
        self.option_by_name(name)
            .is_some_and(|o| o.config.get_bool())
    }

    /// `set(name, o)`; `Err` where the original throws
    /// `IllegalArgumentException`.
    pub fn set(&mut self, name: Option<&str>, o: &ConfigObject) -> Result<(), String> {
        let (Some(name), false) = (name, matches!(o, ConfigObject::Null)) else {
            return Err(String::new());
        };
        let option = self
            .option_by_name_mut(name)
            .ok_or_else(|| format!("unknown SandboxOption \"{name}\""))?;
        option.config.set_value_from_object(o);
        Ok(())
    }

    /// `copyValuesFrom(other)` (option by option, by index).
    pub fn copy_values_from(&mut self, other: &SandboxOptions) {
        for (i, o) in self.options.iter_mut().enumerate() {
            if let Some(src) = other.options.get(i) {
                o.config
                    .set_value_from_object(&src.config.get_value_as_object());
            }
        }
    }

    pub fn reset_to_default(&mut self) {
        self.options
            .iter_mut()
            .for_each(|o| o.config.reset_to_default());
    }

    pub fn set_defaults_to_current_values(&mut self) {
        self.options
            .iter_mut()
            .for_each(|o| o.config.set_default_to_current_value());
    }

    /// `initSandboxVars()`: every option reads its value from `SandboxVars`
    /// and writes it back, filling in missing entries.
    pub fn init_sandbox_vars(&mut self, vars: &TableRef) {
        for o in &mut self.options {
            o.from_table(vars);
            o.to_table(vars);
        }
    }

    /// `toLua()`.
    pub fn to_lua(&self, vars: &TableRef) {
        self.options.iter().for_each(|o| o.to_table(vars));
    }

    /// `updateFromLua()`: reads every option from `SandboxVars`. The
    /// original also sets `GameTime.multiplierBias`,
    /// `VirtualZombieManager.maxRealZombies` and calls `applySettings()`;
    /// those belong to the world and game-time ports.
    pub fn update_from_lua(&mut self, vars: &TableRef) {
        self.apply_table(vars);
    }

    /// `isValidPresetName(name)`.
    pub fn is_valid_preset_name(name: Option<&str>) -> bool {
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            return false;
        };
        !name.contains(['/', '\\', ':', ';', '"', '.'])
    }

    // ---- preset text files ----------------------------------------------------------

    /// `readTextFile(fileName, isPreset)`.
    pub fn read_text_file(&mut self, file: &Path, is_preset: bool) -> bool {
        let mut cf = ConfigFile::new();
        if !cf.read(file) {
            return false;
        }
        let version = cf.get_version();
        let fix_zombie_lore: Option<Vec<String>> = (is_preset && version == 1).then(|| {
            self.options
                .iter()
                .filter(|o| o.table_name.as_deref() == Some("ZombieLore"))
                .map(|o| o.short_name.clone())
                .collect()
        });
        let entries: Vec<(String, String)> = cf
            .get_options()
            .iter()
            .map(|c| (c.name().to_string(), c.get_value_as_string()))
            .collect();
        for (mut name, value) in entries {
            if fix_zombie_lore.as_ref().is_some_and(|f| f.contains(&name)) {
                name = format!("ZombieLore.{name}");
            }
            if is_preset && version == 1 {
                if name == "WaterShutModifier" {
                    name = "WaterShut".into();
                } else if name == "ElecShutModifier" {
                    name = "ElecShut".into();
                }
            }
            let value = self.upgrade_option_value(&name, &value, version);
            if let Some(o) = self.option_by_name_mut(&name) {
                o.config.parse(&value);
            }
        }
        true
    }

    /// `writeTextFile(fileName, version)`: `ConfigFile.write` with sandbox
    /// tooltips and enum value lists.
    pub fn write_text_file(&self, file: &Path, version: i32, t: &Translator) -> bool {
        let sep = if cfg!(windows) { "\r\n" } else { "\n" };
        let mut out = String::new();
        if version != 0 {
            out.push_str(&format!("Version={version}{sep}"));
        }
        let line_sep = format!("{sep}{sep}");
        let n = self.options.len();
        for (i, o) in self.options.iter().enumerate() {
            if let Some(tooltip) = o.tooltip(t) {
                let tooltip = tooltip
                    .replace("\\n", " ")
                    .replace("\\\"", "\"")
                    .replace('\n', &format!("{sep}# "));
                out.push_str(&format!("# {tooltip}{sep}"));
            }
            if o.is_enum {
                for j in 1..=o.num_values() {
                    if let Ok(Some(s)) = o.value_translation_by_index_or_null(t, j) {
                        out.push_str(&format!("    -- {j} = {}{sep}", s.replace("\\\"", "\"")));
                    }
                }
            }
            out.push_str(&format!(
                "{}={}{}",
                o.name(),
                o.config.get_value_as_string(),
                if i < n - 1 { line_sep.as_str() } else { "" }
            ));
        }
        std::fs::write(file, out).is_ok()
    }

    /// `loadPresetFile(presetName)` from `<cache>/Sandbox Presets`.
    pub fn load_preset_file(&mut self, sandbox_cache_dir: &Path, preset: &str) -> bool {
        self.read_text_file(&sandbox_cache_dir.join(format!("{preset}.cfg")), true)
    }

    /// `savePresetFile(presetName)`.
    pub fn save_preset_file(&self, sandbox_cache_dir: &Path, preset: &str, t: &Translator) -> bool {
        if !Self::is_valid_preset_name(Some(preset)) {
            return false;
        }
        self.write_text_file(
            &sandbox_cache_dir.join(format!("{preset}.cfg")),
            SANDBOX_VERSION,
            t,
        )
    }

    /// The text `writeLuaFile(fileName, isDeveloperFile)` writes: the
    /// `SandboxVars = {...}` (or `return {...}`) file, grouped by table.
    pub fn lua_file_text(&self, developer: bool, t: &Translator) -> String {
        let sep = if cfg!(windows) { "\r\n" } else { "\n" };
        let mut table_names: Vec<&str> = Vec::new();
        for o in &self.options {
            if let Some(tn) = o.table_name.as_deref() {
                if !table_names.contains(&tn) {
                    table_names.push(tn);
                }
            }
        }
        let comments = |o: &SandboxOption, indent: &str, out: &mut String, nested: bool| {
            if developer {
                return;
            }
            if let Some(tooltip) = o.tooltip(t) {
                let tooltip = tooltip
                    .replace("\\n", " ")
                    .replace("\\\"", "\"")
                    .replace('\n', &format!("{sep}{indent}-- "));
                out.push_str(&format!("{indent}-- {tooltip}{sep}"));
            }
            if o.is_enum {
                for i in 1..=o.num_values() {
                    if let Ok(Some(s)) = o.value_translation_by_index_or_null(t, i) {
                        // Only the top-level list unescapes quotes.
                        let s = if nested { s } else { s.replace("\\\"", "\"") };
                        out.push_str(&format!("{indent}-- {i} = {s}{sep}"));
                    }
                }
            }
        };
        let mut out = String::new();
        out.push_str(if developer {
            "return {"
        } else {
            "SandboxVars = {"
        });
        out.push_str(sep);
        out.push_str(&format!("    VERSION = {SANDBOX_VERSION},{sep}"));
        for o in self.options.iter().filter(|o| o.table_name.is_none()) {
            comments(o, "    ", &mut out, false);
            out.push_str(&format!(
                "    {} = {},{sep}",
                o.name(),
                o.config.get_value_as_lua_string()
            ));
        }
        for tn in table_names {
            out.push_str(&format!("    {tn} = {{{sep}"));
            for o in self
                .options
                .iter()
                .filter(|o| o.table_name.as_deref() == Some(tn))
            {
                comments(o, "        ", &mut out, true);
                out.push_str(&format!(
                    "        {} = {},{sep}",
                    o.short_name,
                    o.config.get_value_as_lua_string()
                ));
            }
            out.push_str(&format!("    }},{sep}"));
        }
        out.push('}');
        out.push_str(sep);
        out
    }

    /// `upgradeOptionValue(name, value, version)`.
    pub fn upgrade_option_value(&mut self, name: &str, value: &str, version: i32) -> String {
        let mut value = value.to_string();
        // `(int)Double.parseDouble(v)`.
        let parse_int = |v: &str| v.trim().parse::<f64>().ok().map(|d| d as i32);
        if version < 3 && name == "DayLength" {
            if let Some(o) = self.option_by_name_mut("DayLength") {
                o.config.parse(&value);
                match o.config.get_int() {
                    8 => o.config.set_int(14),
                    9 => o.config.set_int(26),
                    _ => {}
                }
                value = o.config.get_value_as_string();
            }
        }
        if version < 4 && name == "CarSpawnRate" {
            if let Some(v) = parse_int(&value).filter(|v| *v > 1) {
                value = (v + 1).to_string();
            }
        }
        if version < 5 {
            const OLD_LOOT: [&str; 10] = [
                "FoodLoot",
                "CannedFoodLoot",
                "LiteratureLoot",
                "SurvivalGearsLoot",
                "MedicalLoot",
                "WeaponLoot",
                "RangedWeaponLoot",
                "AmmoLoot",
                "MechanicsLoot",
                "OtherLoot",
            ];
            if OLD_LOOT.contains(&name) {
                if let Some(v) = parse_int(&value).filter(|v| *v > 0) {
                    value = (v + 2).to_string();
                }
            }
            if name == "RecentlySurvivorVehicles" {
                if let Some(v) = parse_int(&value).filter(|v| *v > 0) {
                    value = (v + 1).to_string();
                }
            }
        }
        // Version 6 shifts DayLength on the option itself; the returned
        // value is the original's unchanged string, as in the original.
        if version < 6 && name == "DayLength" {
            if let Some(o) = self.option_by_name_mut("DayLength") {
                o.config.parse(&value);
                if o.config.get_int() > 3 {
                    let v = o.config.get_int() + 1;
                    o.config.set_int(v);
                }
            }
        }
        value
    }

    /// `upgradeLuaTable(prefix, table, version)` for a `SandboxVars` file
    /// read by `readLuaFile`: values become strings, sub-tables recurse.
    pub fn upgrade_lua_table(
        &mut self,
        prefix: &str,
        table: &TableRef,
        version: i32,
    ) -> Result<TableRef, String> {
        let out = TableRef::new();
        for (k, v) in table.entries() {
            let Value::Str(key) = &k else {
                return Err("expected a String key".into());
            };
            if let Value::Table(sub) = &v {
                let t = self.upgrade_lua_table(&format!("{prefix}{key}."), sub, version)?;
                out.rawset(k.clone(), Value::Table(t));
                continue;
            }
            let name = format!("{prefix}{key}");
            let value =
                self.upgrade_option_value(&name, &pz_kahlua::exec::java_value_of(&v), version);
            out.rawset(Value::from(name.replace(prefix, "")), Value::from(value));
        }
        Ok(out)
    }

    // ---- derived values -------------------------------------------------------------

    pub fn first_year(&self) -> i32 {
        FIRST_YEAR
    }

    pub fn temperature_modifier(&self) -> i32 {
        self.int("Temperature")
    }

    pub fn rain_modifier(&self) -> i32 {
        self.int("Rain")
    }

    pub fn erosion_speed(&self) -> i32 {
        self.int("ErosionSpeed")
    }

    pub fn water_shut_modifier(&self) -> i32 {
        self.int("WaterShutModifier")
    }

    pub fn elec_shut_modifier(&self) -> i32 {
        self.int("ElecShutModifier")
    }

    pub fn time_since_apo(&self) -> i32 {
        self.int("TimeSinceApo")
    }

    pub fn all_clothes_unlocked(&self) -> bool {
        self.bool("AllClothesUnlocked")
    }

    pub fn endurance_regen_multiplier(&self) -> f64 {
        match self.int("EndRegen") {
            1 => 1.8,
            2 => 1.3,
            4 => 0.7,
            5 => 0.4,
            _ => 1.0,
        }
    }

    pub fn stats_decrease_multiplier(&self) -> f64 {
        match self.int("StatsDecrease") {
            1 => 2.0,
            2 => 1.6,
            4 => 0.8,
            5 => 0.65,
            _ => 1.0,
        }
    }

    fn day_length_minutes_for(value: i32) -> i32 {
        match value {
            1 => 15,
            2 => 30,
            3 => 60,
            4 => 90,
            v => (v - 3) * 60,
        }
    }

    pub fn day_length_minutes(&self) -> i32 {
        Self::day_length_minutes_for(self.int("DayLength"))
    }

    pub fn day_length_minutes_default(&self) -> i32 {
        Self::day_length_minutes_for(
            self.option_by_name("DayLength")
                .map_or(0, |o| int_default(&o.config)),
        )
    }

    pub fn compost_hours(&self) -> i32 {
        match self.int("CompostTime") {
            1 => 168,
            3 => 504,
            4 => 672,
            5 => 1008,
            6 => 1344,
            7 => 1680,
            8 => 2016,
            _ => 336,
        }
    }

    pub fn random_water_shut(&self, modifier: i32) -> i32 {
        match modifier {
            2 => rand_between(0, 30),
            3 => rand_between(0, 60),
            4 => rand_between(0, 180),
            5 => rand_between(0, 360),
            6 => rand_between(0, 1800),
            7 => rand_between(60, 180),
            8 => rand_between(180, 360),
            9 => i32::MAX,
            _ => -1,
        }
    }

    pub fn random_electricity_shut(&self, modifier: i32) -> i32 {
        match modifier {
            2 => rand_between(14, 30),
            3 => rand_between(14, 60),
            4 => rand_between(14, 180),
            5 => rand_between(14, 360),
            6 => rand_between(14, 1800),
            7 => rand_between(60, 180),
            8 => rand_between(180, 360),
            9 => i32::MAX,
            _ => -1,
        }
    }

    pub fn random_alarm_decay(&self, modifier: i32) -> i32 {
        match modifier {
            2 => rand_between(0, 30),
            3 => rand_between(0, 60),
            4 => rand_between(0, 180),
            5 => rand_between(0, 360),
            6 => rand_between(0, 1800),
            _ => 0,
        }
    }

    fn ramp(max: i32, until: i32, world_age_days: f64) -> Option<i32> {
        if max <= 0 {
            return Some(0);
        }
        if until <= 0 {
            return Some(max);
        }
        let _ = world_age_days;
        None
    }

    /// `getCurrentRatIndex()` for a world `world_age_days` old.
    pub fn current_rat_index(&self, world_age_days: f64) -> i32 {
        let (max, until) = (
            self.int("MaximumRatIndex"),
            self.int("DaysUntilMaximumRatIndex"),
        );
        if let Some(v) = Self::ramp(max, until, world_age_days) {
            return v;
        }
        let days = (world_age_days as i32).max(1).min(until);
        (max * days / until).max(1)
    }

    /// `getCurrentLootedChance()` (no square).
    pub fn current_looted_chance(&self, world_age_days: f64) -> i32 {
        let (max, until) = (
            self.int("MaximumLooted"),
            self.int("DaysUntilMaximumLooted"),
        );
        if let Some(v) = Self::ramp(max, until, world_age_days) {
            return v;
        }
        let days = (world_age_days as i32).max(1).min(until);
        (max * days / until).max(1)
    }

    /// `getCurrentDiminishedLootPercentage()` (no square).
    pub fn current_diminished_loot_percentage(&self, world_age_days: f64) -> i32 {
        let (max, until) = (
            self.int("MaximumDiminishedLoot"),
            self.int("DaysUntilMaximumDiminishedLoot"),
        );
        if let Some(v) = Self::ramp(max, until, world_age_days) {
            return v;
        }
        let days = (world_age_days as i32).max(1).min(until);
        (max * days / until).clamp(0, 100)
    }

    /// `getCurrentLootMultiplier()` (no square).
    pub fn current_loot_multiplier(&self, world_age_days: f64) -> f32 {
        1.0 - self.current_diminished_loot_percentage(world_age_days) as f32 / 100.0
    }

    /// `lootItemRemovalListContains(itemType)`.
    pub fn loot_item_removal_list_contains(&self, item_type: &str) -> bool {
        self.option_by_name("LootItemRemovalList")
            .is_some_and(|o| o.split_csv_list().iter().any(|s| s == item_type))
    }

    /// `worldItemRemovalListContains(itemType)`.
    pub fn world_item_removal_list_contains(&self, item_type: &str) -> bool {
        self.option_by_name("WorldItemRemovalList")
            .is_some_and(|o| o.split_csv_list().iter().any(|s| s == item_type))
    }

    /// `doesPowerGridExist(offset)` for a world `world_age_days` old.
    pub fn does_power_grid_exist(&self, world_age_days: f32, offset: i32) -> bool {
        world_age_days <= (self.elec_shut_modifier() + offset) as f32
    }

    /// `newCustomOption`: a mod option, added after the built-in ones.
    pub fn new_custom_option(&mut self, def: &OptionDef, page: Option<&str>) {
        let mut o = SandboxOption::from_def(def);
        o.custom = true;
        if let Some(p) = page {
            o.page_name = Some(p.to_string());
        }
        self.custom.push(self.options.len());
        self.add_option(o);
    }

    /// `Reset()` / `removeCustomOptions()`.
    pub fn remove_custom_options(&mut self) {
        let custom = std::mem::take(&mut self.custom);
        let mut i = 0;
        self.options.retain(|_| {
            let keep = !custom.contains(&i);
            i += 1;
            keep
        });
        self.by_name = self
            .options
            .iter()
            .enumerate()
            .map(|(i, o)| (o.name().to_string(), i))
            .collect();
    }
}

/// `<cache>/Sandbox Presets`.
pub fn sandbox_cache_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("Sandbox Presets")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Vec<OptionDef> {
        let d = |name: &str, kind| OptionDef {
            name: name.into(),
            kind,
            translation: None,
            value_translation: None,
        };
        vec![
            d(
                "A",
                OptionKind::Enum {
                    num_values: 5,
                    default: 3,
                },
            ),
            d("T.B", OptionKind::Boolean { default: true }),
            d(
                "C",
                OptionKind::Double {
                    min: 0.0,
                    max: 4.0,
                    default: 0.5,
                },
            ),
            d(
                "DayLength",
                OptionKind::Enum {
                    num_values: 27,
                    default: 4,
                },
            ),
        ]
    }

    #[test]
    fn sandbox_vars_round_trip() {
        let mut s = SandboxOptions::new(&defs());
        let vars = TableRef::new();
        vars.rawset(Value::str("A"), Value::Num(5.0));
        s.init_sandbox_vars(&vars);
        assert_eq!(s.option_by_name("A").unwrap().config.get_int(), 5);
        let Value::Table(t) = vars.rawget(&Value::str("T")) else {
            panic!("sub-table")
        };
        assert!(t.rawget(&Value::str("B")).raw_equals(&Value::Bool(true)));
        assert!(vars.rawget(&Value::str("C")).raw_equals(&Value::Num(0.5)));
        assert_eq!(s.option_by_name("A").unwrap().get_type(), "enum");
    }

    #[test]
    fn upgrades_and_names() {
        let mut s = SandboxOptions::new(&defs());
        assert_eq!(s.day_length_minutes(), 90);
        // The upgrade also moves the option itself (8 -> 14, then +1 for v6).
        assert_eq!(s.upgrade_option_value("DayLength", "8", 2), "14");
        assert_eq!(s.option_by_name("DayLength").unwrap().config.get_int(), 15);
        assert!(SandboxOptions::is_valid_preset_name(Some("My Preset")));
        assert!(!SandboxOptions::is_valid_preset_name(Some("a.b")));
    }
}
