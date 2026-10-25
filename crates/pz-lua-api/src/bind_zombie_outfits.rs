//! The outfit singletons zombies dress from
//! ([`pz_characters::character::zombie::dress::ZombieOutfits`]:
//! `PersistentOutfits`, `ZombiesZoneDefinition`, `UnderwearDefinition`,
//! `AttachedWeaponDefinitions`), shared like the other population state,
//! with their `checkDirty` reads of Lua's `ZombiesZoneDefinition`,
//! `UnderwearDefinition` and `AttachedWeaponDefinitions` tables (marked
//! dirty by every `RunLua`), and the world-load step
//! `PersistentOutfits.instance.init()` ([`init_persistent_outfits`]), whose
//! outfit names `ZombiePopulationManager.init` hands the native library.
//!
//! None of these classes is exposed to Lua. A table the original would
//! throw on while reading (a non-string where it casts to `String`, a bad
//! style string) is logged; the entries read before it are kept.

use pz_characters::character::zombie::dress::{ZombieDressWorld, ZombieOutfits};
use pz_characters::population::zombie_defs::{
    blood_part_from_string, AttachedWeaponCustomOutfit, AttachedWeaponDefinition, AttachedWeaponDefinitions,
    OutfitUnderwearDefinition, StringChance, UnderwearDefinition, ZombiesZoneDefinition, ZoneView, ZoneWorld, ZzdOutfit, ZzdZone,
};
use pz_characters::population::persistent_outfits::OutfitsFile;
use pz_inventory::InventoryItem;
use pz_kahlua::{TableRef, Thread, Value};
use pz_scripts::java::compare;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<ZombieOutfits>>>> = const { RefCell::new(None) };
}

/// Make `o` the shared outfit state.
pub fn install_zombie_outfits(o: Rc<RefCell<ZombieOutfits>>) {
    SHARED.with(|s| *s.borrow_mut() = Some(o));
}

/// The shared outfit state, created empty on first use.
pub fn shared_zombie_outfits() -> Rc<RefCell<ZombieOutfits>> {
    SHARED.with(|s| s.borrow_mut().get_or_insert_with(Default::default).clone())
}

/// `LuaManager.RunLua`'s `AttachedWeaponDefinitions.instance.dirty = true;
/// UnderwearDefinition.instance.dirty = true; ZombiesZoneDefinition.dirty =
/// true`.
pub fn mark_dirty() {
    let o = shared_zombie_outfits();
    let Ok(mut o) = o.try_borrow_mut() else {
        return;
    };
    o.weapons.dirty = true;
    o.underwear.dirty = true;
    o.zones.dirty = true;
}

/// `PersistentOutfits.instance.init()` at world load (single player:
/// seeds drawn, `z_outfits.bin` at `file` loaded then saved unless
/// `no_save`), over the shared population's `OutfitManager` and the
/// `ZombiesZoneDefinition` custom outfits (`checkDirty` reads Lua's table
/// first). Returns `getOutfitNames()`, what `WorldPopulation::init` takes.
pub fn init_persistent_outfits(thread: &Thread, file: Option<(&Path, bool)>) -> Vec<String> {
    let pop = crate::bind_appearance::shared_population();
    let pop = pop.borrow();
    let empty = Default::default();
    let om = pop.outfit_manager.as_ref().unwrap_or(&empty);
    let o = shared_zombie_outfits();
    let mut o = o.borrow_mut();
    if o.zones.dirty {
        load_zones(&thread.globals().clone(), &mut o.zones);
    }
    let custom = o.zones.register_custom_outfits();
    let file = file.map(|(path, no_save)| OutfitsFile { path, no_save });
    o.persistent.init(om, &custom, false, false, file);
    o.persistent.outfit_names().to_vec()
}

fn to_string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

/// `rawgetStr(key)`: `Err` is the `ClassCastException` for a non-string.
fn raw_str(t: &TableRef, k: &str) -> Result<Option<String>, String> {
    match t.rawget(&Value::str(k)) {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s.to_string())),
        v => Err(format!("java.lang.ClassCastException: {} cannot be cast to java.lang.String", class_of(&v))),
    }
}

fn class_of(v: &Value) -> &'static str {
    match v {
        Value::Num(_) => "java.lang.Double",
        Value::Bool(_) => "java.lang.Boolean",
        Value::Table(_) => "se.krka.kahlua.j2se.KahluaTableImpl",
        _ => "java.lang.Object",
    }
}

/// `rawgetInt(key)`: -1 unless a number.
fn raw_int(t: &TableRef, k: &str) -> i32 {
    match t.rawget(&Value::str(k)) {
        Value::Num(n) => n as i32,
        _ => -1,
    }
}

/// `rawgetFloat(key)`: -1 unless a number.
fn raw_float(t: &TableRef, k: &str) -> f32 {
    match t.rawget(&Value::str(k)) {
        Value::Num(n) => n as f32,
        _ => -1.0,
    }
}

fn raw_table(t: &TableRef, k: &str) -> Option<TableRef> {
    match t.rawget(&Value::str(k)) {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

fn zzd_outfit(t: &TableRef) -> Result<ZzdOutfit, String> {
    ZzdOutfit::new(
        raw_str(t, "name")?,
        raw_float(t, "chance"),
        raw_str(t, "gender")?,
        raw_int(t, "toSpawn"),
        raw_str(t, "mandatory")?,
        raw_str(t, "room")?,
        raw_str(t, "femaleHairStyles")?.as_deref(),
        raw_str(t, "maleHairStyles")?.as_deref(),
        raw_str(t, "beardStyles")?.as_deref(),
    )
}

/// `ZombiesZoneDefinition.init()` over Lua's table (`dirty` cleared first,
/// as `checkDirty` does).
pub fn load_zones(globals: &TableRef, zones: &mut ZombiesZoneDefinition) {
    let mut list = Vec::new();
    let r = (|| -> Result<(), String> {
        let Value::Table(def) = globals.get_str("ZombiesZoneDefinition") else {
            return Ok(());
        };
        for (k, v) in def.entries() {
            let Value::Table(zt) = v else { continue };
            let mut outfits = Vec::new();
            for (_, ov) in zt.entries() {
                if let Value::Table(ot) = ov {
                    outfits.push(zzd_outfit(&ot)?);
                }
            }
            list.push(ZzdZone::new(
                to_string(&k),
                raw_int(&zt, "femaleChance"),
                raw_int(&zt, "maleChance"),
                raw_int(&zt, "chanceToSpawn"),
                raw_int(&zt, "toSpawn"),
                outfits,
            ));
        }
        Ok(())
    })();
    if let Err(e) = r {
        eprintln!("ZombiesZoneDefinition: {e}");
    }
    zones.set_zones(list);
}

/// `UnderwearDefinition.init()` over Lua's table.
pub fn load_underwear(globals: &TableRef, u: &mut UnderwearDefinition) {
    u.dirty = false;
    u.outfit_definition.clear();
    let Value::Table(defs) = globals.get_str("UnderwearDefinition") else {
        return;
    };
    u.base_chance = raw_int(&defs, "baseChance");
    let r = (|| -> Result<(), String> {
        for (_, v) in defs.entries() {
            let Value::Table(d) = v else { continue };
            let tops = match d.rawget(&Value::str("top")) {
                Value::Table(tops) => {
                    let mut all = Vec::new();
                    for (_, tv) in tops.entries() {
                        if let Value::Table(t) = tv {
                            all.push(StringChance { str: raw_str(&t, "name")?.unwrap_or_else(|| "null".into()), chance: raw_float(&t, "chance") });
                        }
                    }
                    Some(all)
                }
                _ => None,
            };
            let gender = raw_str(&d, "gender")?;
            u.outfit_definition.push(OutfitUnderwearDefinition::new(tops, raw_str(&d, "bottom")?, raw_int(&d, "chanceToSpawn"), gender.as_deref()));
        }
        Ok(())
    })();
    if let Err(e) = r {
        eprintln!("UnderwearDefinition: {e}");
    }
}

/// `tableToArrayList(table, key, list)`: `table[key][1..#]` as strings.
fn string_list(t: &TableRef, k: &str) -> Vec<String> {
    let Some(l) = raw_table(t, k) else {
        return Vec::new();
    };
    (1..=l.len()).filter_map(|i| match l.get_int(i) {
        Value::Nil => None,
        v => Some(to_string(&v)),
    })
    .collect()
}

fn weapon_def(id: String, t: &TableRef) -> Result<AttachedWeaponDefinition, String> {
    let mut d = AttachedWeaponDefinition {
        id,
        chance: raw_int(t, "chance"),
        outfit: string_list(t, "outfit"),
        weapon_location: string_list(t, "weaponLocation"),
        ..Default::default()
    };
    if let Some(b) = raw_table(t, "bloodLocations") {
        for (_, v) in b.entries() {
            let p = blood_part_from_string(&to_string(&v));
            if p != pz_characters::character::items::blood_part::MAX {
                d.blood_locations.push(p);
            }
        }
    }
    d.add_holes = matches!(t.rawget(&Value::str("addHoles")), Value::Bool(true));
    d.day_survived = raw_int(t, "daySurvived");
    d.ensure_item = raw_str(t, "ensureItem")?;
    d.weapons = string_list(t, "weapons");
    Ok(d.sorted())
}

/// `AttachedWeaponDefinitions.init()` over Lua's table, with the sandbox
/// `ChanceOfAttachedWeapon`.
pub fn load_weapons(globals: &TableRef, w: &mut AttachedWeaponDefinitions, chance_of_attached_weapon: i32) {
    w.dirty = false;
    w.definitions.clear();
    w.outfit_definitions.clear();
    let Value::Table(def) = globals.get_str("AttachedWeaponDefinitions") else {
        return;
    };
    w.chance_of_attached_weapon = chance_of_attached_weapon;
    let key_str = |k: &Value| match k {
        Value::Str(s) => Ok(s.to_string()),
        v => Err(format!("java.lang.ClassCastException: {} cannot be cast to java.lang.String", class_of(v))),
    };
    let r = (|| -> Result<(), String> {
        for (k, v) in def.entries() {
            let Value::Table(t) = v else { continue };
            if matches!(&k, Value::Str(s) if &**s == "attachedWeaponCustomOutfit") {
                for (ck, cv) in t.entries() {
                    let Value::Table(ct) = cv else {
                        return Err("java.lang.ClassCastException: cannot be cast to se.krka.kahlua.j2se.KahluaTableImpl".into());
                    };
                    let mut c = AttachedWeaponCustomOutfit {
                        outfit: key_str(&ck)?,
                        chance: raw_int(&ct, "chance"),
                        maxitem: raw_int(&ct, "maxitem"),
                        weapons: Vec::new(),
                    };
                    let weapons = raw_table(&ct, "weapons").ok_or("java.lang.NullPointerException")?;
                    for (_, wv) in weapons.entries() {
                        let Value::Table(wt) = wv else {
                            return Err("java.lang.ClassCastException: cannot be cast to se.krka.kahlua.j2se.KahluaTableImpl".into());
                        };
                        let id = raw_str(&wt, "id")?.unwrap_or_else(|| "null".into());
                        c.weapons.push(weapon_def(id, &wt)?);
                    }
                    w.outfit_definitions.push(c);
                }
                continue;
            }
            let d = weapon_def(key_str(&k)?, &t)?;
            w.definitions.push(d);
        }
        Ok(())
    })();
    if let Err(e) = r {
        eprintln!("AttachedWeaponDefinitions: {e}");
    }
    w.definitions.sort_by(|a, b| compare(&a.id, &b.id));
}

/// A [`ZombieDressWorld`] over Lua's tables and the engine's world: the
/// zone lookups, item creation and loot modifier come from the engine.
pub struct LuaDressWorld<'a> {
    pub globals: TableRef,
    pub zones: &'a dyn ZoneWorld,
    /// `InventoryItemFactory.CreateItem(type)`.
    pub create_item: &'a mut dyn FnMut(&str) -> Option<InventoryItem>,
    /// `ItemPickerJava.getLootModifier(type)`.
    pub loot_modifier: &'a dyn Fn(&str) -> f32,
    /// `SandboxOptions.lore.chanceOfAttachedWeapon`.
    pub chance_of_attached_weapon: i32,
}

impl ZoneWorld for LuaDressWorld<'_> {
    fn square_room(&self, x: i32, y: i32, z: i32) -> Option<Option<String>> {
        self.zones.square_room(x, y, z)
    }
    fn zones_at(&self, x: i32, y: i32, z: i32) -> Vec<ZoneView> {
        self.zones.zones_at(x, y, z)
    }
}

impl ZombieDressWorld for LuaDressWorld<'_> {
    fn check_dirty_zones(&mut self, zones: &mut ZombiesZoneDefinition) {
        if zones.dirty {
            load_zones(&self.globals, zones);
        }
    }
    fn check_dirty_underwear(&mut self, underwear: &mut UnderwearDefinition) {
        if underwear.dirty {
            load_underwear(&self.globals, underwear);
        }
    }
    fn check_dirty_weapons(&mut self, weapons: &mut AttachedWeaponDefinitions) {
        if weapons.dirty {
            load_weapons(&self.globals, weapons, self.chance_of_attached_weapon);
        }
    }
    fn create_item(&mut self, item_type: &str) -> Option<InventoryItem> {
        (self.create_item)(item_type)
    }
    fn loot_modifier(&self, item_type: &str) -> f32 {
        (self.loot_modifier)(item_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> TableRef {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();
        t.globals().clone()
    }

    #[test]
    fn tables_from_lua() {
        let g = run(r#"
            ZombiesZoneDefinition = {}
            ZombiesZoneDefinition.Army = { chanceToSpawn = 20, toSpawn = 2,
                { name = "ArmyCamoGreen", chance = 90, maleHairStyles = "Bald:50;Buzzcut:50" },
                { name = "Doctor", chance = 10, gender = "female", room = "medical", mandatory = "true", toSpawn = 1 } }
            ZombiesZoneDefinition.Default = { { name = "Generic01", chance = 100 } }
            UnderwearDefinition = { baseChance = 40,
                FrillyRed = { chanceToSpawn = 10, gender = "female", bottom = "Base.Underpants_Frilly",
                    top = { { name = "Base.Bra_Frilly", chance = 70 }, { name = "Base.Bra_Strapless", chance = 30 } } } }
            AttachedWeaponDefinitions = { chanceOfAttachedWeapon = 6,
                knife = { chance = 10, weaponLocation = { "Knife Stomach", "Knife Back" }, bloodLocations = { "Torso_Upper", "Nope" },
                    addHoles = true, daySurvived = 0, weapons = { "Base.KitchenKnife", "Base.HuntingKnife" } },
                axe = { chance = 5, outfit = { "Lumberjack" }, weaponLocation = { "Back" }, weapons = { "Base.Axe" } },
                attachedWeaponCustomOutfit = { Police = { chance = 50, maxitem = 2,
                    weapons = { { id = "holster", chance = 5, weaponLocation = { "Holster Right" }, weapons = { "Base.Pistol" }, ensureItem = "Base.HolsterSimple" } } } } }
        "#);
        let mut z = ZombiesZoneDefinition::default();
        load_zones(&g, &mut z);
        assert!(!z.dirty);
        assert_eq!(z.zones().len(), 2);
        let army = &z.zones()[0];
        assert_eq!((army.name.as_str(), army.chance_to_spawn, army.to_spawn, army.female_chance), ("Army", 20, 2, -1));
        assert_eq!(army.outfits[0].custom_name, "ZZD.Army.ArmyCamoGreen");
        assert_eq!(army.outfits[0].male_hair_styles.as_ref().unwrap().len(), 2);
        assert_eq!((army.outfits[1].room.as_deref(), army.outfits[1].to_spawn, army.outfits[1].chance), (Some("medical"), 1, 10.0));
        let mut u = UnderwearDefinition::default();
        load_underwear(&g, &mut u);
        assert_eq!((u.base_chance, u.outfit_definition.len()), (40, 1));
        assert_eq!((u.outfit_definition[0].top_total_chance, u.outfit_definition[0].female), (100, true));
        let mut w = AttachedWeaponDefinitions::default();
        load_weapons(&g, &mut w, 6);
        assert_eq!(w.definitions.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["axe", "knife"]);
        let k = &w.definitions[1];
        assert_eq!(k.weapon_location, ["Knife Back", "Knife Stomach"]);
        assert_eq!(k.weapons, ["Base.HuntingKnife", "Base.KitchenKnife"]);
        assert_eq!((k.blood_locations.as_slice(), k.add_holes), ([6usize].as_slice(), true));
        assert_eq!(w.outfit_definitions[0].outfit, "Police");
        assert_eq!(w.outfit_definitions[0].weapons[0].ensure_item.as_deref(), Some("Base.HolsterSimple"));
    }
}
