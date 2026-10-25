//! `IsoPlayer.load`/`save` and the `IsoGameCharacter` and `IsoMovingObject`
//! parts they chain (`IsoLivingCharacter` adds nothing): the player record
//! of `players.db` (`PlayerDB.loadPlayer`) and `map_p.bin`.
//!
//! The parts each class owns are loaded by that class's own code here:
//! [`SurvivorDesc::load`], `HumanVisual` (`pz-world`'s save form),
//! `ItemContainer` (`pz-inventory`), [`Stats::load`](super::Stats::load),
//! `BodyDamage` with its [`Thermoregulator::load`](super::Thermoregulator::load),
//! [`GameCharacter::load_xp`] (with `CharacterTraits.load`),
//! [`Nutrition::load`](super::Nutrition::load),
//! [`Fitness::load`](super::Fitness::load) and [`PlayerCheats::load`](super::PlayerCheats::load).
//!
//! Hand and worn items are the ids of inventory items (see
//! [`super::items`]); the saved indices refer to `savedInventoryItems`,
//! the list `CompressIdenticalItems` reads and writes, which can hold items
//! that could not be loaded.
//!
//! Not modelled (rendering, sound and network state the load sets up):
//! `InitSpriteParts`/`initSpritePartsEmpty`, `setEquipParent`,
//! `setUseHandWeapon`, `onWornItemsChanged`'s ragdoll flag and model reset,
//! `NetworkCharacterAI.updateXpChecker`, the `IsoWorld.survivorDescriptors`
//! registration and the client's `MpTextColor`. Attached (hotbar) items are
//! not part of the record: the items carry their `attachedSlot` and the Lua
//! hotbar re-attaches them.

use std::rc::Rc;

use pz_inventory::env::ItemEnv;
use pz_inventory::io::{Reader, SaveError, Writer};
use pz_inventory::java::sort_hash_map;
use pz_scripts::java::JHashMap;
use pz_scripts::objects::items::ItemBodyLocation;
use pz_scripts::registry::ResourceLocation;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};
use pz_world::save::visual::{HumanVisual, MovingBase};
use pz_world::save::SaveTables;
use pz_world::square::IsoDirections;

use super::body_damage::SaveReader;
use super::context::SimContext;
use super::thermoregulator::Thermoregulator;
use super::GameCharacter;
use crate::survivor::SurvivorDesc;
use crate::worn_items::{ItemIdentity, WornItems};
use crate::{npe, Characters};

/// A save buffer error as the `ScriptError` the character code uses.
pub(crate) fn io_error(e: SaveError) -> ScriptError {
    ScriptError::new(format!("java.nio.BufferUnderflowException: {}", e.0))
}

/// `IsoGameCharacter.ReadBook`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadBook {
    pub full_type: String,
    pub already_read_pages: i32,
}

/// `PlayerCraftHistory.CraftHistoryEntry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CraftHistoryEntry {
    pub craft_count: i32,
    pub last_craft_time: f64,
}

/// `savedVehicleX`, `savedVehicleY`, `savedVehicleSeat` and
/// `savedVehicleRunning`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedVehicle {
    pub x: f32,
    pub y: f32,
    pub seat: i16,
    pub running: bool,
}

/// `IsoPlayer` fields the record holds that the simulation does not use.
#[derive(Clone, Debug)]
pub struct PlayerFields {
    /// `speakColour` (r, g, b, a in 0..1).
    pub speak_colour: [f32; 4],
    /// `allChatMuted`.
    pub all_chat_muted: bool,
    /// `tagPrefix`.
    pub tag_prefix: String,
    /// `tagColor` (r, g, b; alpha 1).
    pub tag_color: [f32; 3],
    /// `displayName`.
    pub display_name: Option<String>,
    /// `showTag`.
    pub show_tag: bool,
    /// `factionPvp`.
    pub faction_pvp: bool,
    /// `autoDrink`.
    pub auto_drink: bool,
    /// `mechanicsItem` (a `HashMap<Long, Long>`, in insertion order).
    pub mechanics_item: Vec<(i64, i64)>,
    /// `alreadyReadBook`: full types.
    pub already_read_book: Vec<String>,
    /// `craftHistory.craftHistory`.
    pub craft_history: JHashMap<CraftHistoryEntry>,
}

impl Default for PlayerFields {
    fn default() -> Self {
        PlayerFields {
            speak_colour: [1.0; 4],
            all_chat_muted: false,
            tag_prefix: String::new(),
            tag_color: [1.0; 3],
            display_name: None,
            show_tag: true,
            faction_pvp: false,
            auto_drink: true,
            mechanics_item: Vec::new(),
            already_read_book: Vec::new(),
            craft_history: JHashMap::new(),
        }
    }
}

/// What loading and saving a character record read besides the character:
/// the item lookups (`WorldDictionary`, scripts) and the jar tables.
pub struct SaveEnv<'a> {
    pub items: &'a dyn ItemEnv,
    pub tables: &'a SaveTables,
}

/// The `IsoGameCharacter` cheat setters `load` calls, in order: the
/// `CheatType`, and the world version that added it (0: always saved).
const LOADED_CHEATS: [(&str, i32); 13] = [
    ("UNLIMITED_CARRY", 0),
    ("BUILD", 0),
    ("HEALTH", 0),
    ("MECHANICS", 0),
    ("MOVABLES", 0),
    ("FARMING", 0),
    ("FISHING", 202),
    ("BRUSH_TOOL", 217),
    ("FAST_MOVE", 217),
    ("TIMED_ACTION_INSTANT", 0),
    ("UNLIMITED_ENDURANCE", 0),
    ("UNLIMITED_AMMO", 230),
    ("KNOW_ALL_RECIPES", 230),
];

/// `Long.hashCode`.
fn long_hash(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

fn e<T>(r: Result<T, SaveError>) -> ScriptResult<T> {
    r.map_err(io_error)
}

impl<I: ItemIdentity + 'static> GameCharacter<I> {
    /// The id of `savedInventoryItems[index]` (`None` out of range or for
    /// an item that could not be loaded).
    fn saved_item(&self, index: i32) -> Option<Option<i32>> {
        usize::try_from(index).ok().and_then(|i| self.saved_inventory_items.get(i)).copied()
    }

    /// `Role.hasCapability(this, capability)`: only in single-player debug
    /// mode (a player without a role has none).
    fn has_cheat_capability(ctx: &dyn SimContext) -> bool {
        ctx.is_debug() && !ctx.is_client() && !ctx.is_server()
    }

    /// `IsoPlayer.load(input, worldVersion, isDebugSave)` into this
    /// character (a `new IsoPlayer(cell)`, as `PlayerDB.loadPlayer` uses):
    /// the serialise and class bytes, `IsoGameCharacter.load`, then the
    /// player fields. `set_on_fire` is `SetOnFire()` for a burning
    /// character (the fire manager is the world's); it is called where the
    /// original calls it.
    #[allow(clippy::too_many_arguments)]
    pub fn load_player(
        &mut self,
        sm: &ScriptManager,
        chars: &mut Characters,
        ctx: &dyn SimContext,
        env: &SaveEnv<'_>,
        r: &mut Reader<'_>,
        world_version: i32,
        set_on_fire: &mut dyn FnMut(&mut Self),
    ) -> ScriptResult {
        let _serialise = e(r.i8())?;
        let _object_id = e(r.i8())?;
        self.load_game_character(sm, chars, ctx, env, r, world_version, set_on_fire)?;
        self.hours_survived = e(r.f64())?;
        // setFemale(descriptor.isFemale()) sets the descriptor's own gender;
        // a missing descriptor is a NullPointerException.
        if self.descriptor.is_none() {
            return Err(npe());
        }
        // speakColour: three draws (a client then takes MpTextColor).
        let mut c = [0.0f32; 4];
        for v in c.iter_mut().take(3) {
            *v = (ctx.rand_next(135) + 120) as f32 / 255.0;
        }
        c[3] = 1.0;
        self.player_fields.speak_colour = c;
        self.zombie_kills = e(r.i32())?;
        let worn_count = e(r.i8())?;
        // The constructor's `initWornItems("Human")`, unless already made.
        if self.items.worn_items.is_none() {
            self.items.worn_items = Some(WornItems::new(chars.body_locations.get_group("Human")?));
        }
        for _ in 0..worn_count {
            let name = e(r.string())?;
            let location = ItemBodyLocation::get(sm, &ResourceLocation::of(&name)?);
            let index = e(r.i16())?;
            let Some(item) = self.saved_item(index as i32) else { continue };
            let worn = self.items.worn_items.as_mut().ok_or_else(npe)?;
            if worn.body_location_group().borrow().location(location.as_ref())?.is_none() {
                continue;
            }
            worn.set_item(location, item)?;
        }
        self.on_worn_items_changed(sm);
        let index = e(r.i16())?;
        if let Some(item) = self.saved_item(index as i32) {
            self.items.primary_hand_item = item;
        }
        let index = e(r.i16())?;
        if let Some(item) = self.saved_item(index as i32) {
            self.items.secondary_hand_item = item;
        }
        let weapon = self.weapon_type(sm);
        self.set_var_str("Weapon", weapon.type_name());
        self.survivor_kills = e(r.i32())?;
        let nutrition = self.nutrition.as_mut().ok_or_else(npe)?;
        // A weight below 35 also damages the owner (see `nutrition`'s TODO).
        e(nutrition.load(r))?;
        let p = &mut self.player_fields;
        p.all_chat_muted = e(r.bool())?;
        p.tag_prefix = e(r.string())?;
        p.tag_color = [e(r.f32())?, e(r.f32())?, e(r.f32())?];
        p.display_name = Some(e(r.string())?);
        p.show_tag = e(r.bool())?;
        p.faction_pvp = e(r.bool())?;
        if world_version >= 239 {
            p.auto_drink = e(r.bool())?;
        }
        // The extra info flags (god mode, ghost, ...) are read and dropped.
        let _extra_info = e(r.i8())?;
        if e(r.bool())? {
            self.saved_vehicle = Some(SavedVehicle {
                x: e(r.f32())?,
                y: e(r.f32())?,
                seat: e(r.i8())? as i16,
                running: e(r.bool())?,
            });
            self.vars.z = 0.0;
        }
        let n = e(r.i32())?;
        for _ in 0..n {
            let k = e(r.i64())?;
            let v = e(r.i64())?;
            let m = &mut self.player_fields.mechanics_item;
            match m.iter_mut().find(|(key, _)| *key == k) {
                Some(entry) => entry.1 = v,
                None => m.push((k, v)),
            }
        }
        e(self.fitness.as_mut().ok_or_else(npe)?.load(r, world_version))?;
        let n = e(r.i16())?;
        for _ in 0..n {
            let id = e(r.i16())?;
            if let Some(t) = env.items.full_type(id) {
                self.player_fields.already_read_book.push(t);
            }
        }
        // loadKnownMediaLines.
        self.known_media_lines.clear();
        let n = e(r.i16())?;
        for _ in 0..n {
            self.known_media_lines.insert(e(r.string())?, ());
        }
        if world_version >= 203 {
            // setVoiceType: clamped to 0..3 on the descriptor.
            let v = (e(r.i8())? as i32).clamp(0, 3);
            self.descriptor.as_ref().ok_or_else(npe)?.borrow_mut().voice_type = v;
        }
        if world_version >= 228 {
            let history = &mut self.player_fields.craft_history;
            history.clear();
            let n = e(r.i32())?;
            for _ in 0..n {
                let len = e(r.i32())?;
                let mut units = Vec::with_capacity(len.max(0) as usize);
                for _ in 0..len {
                    units.push(e(r.i16())? as u16);
                }
                let key = String::from_utf16_lossy(&units);
                let craft_count = e(r.i32())?;
                let last_craft_time = e(r.f64())?;
                history.insert(key, CraftHistoryEntry { craft_count, last_craft_time });
            }
        }
        Ok(())
    }

    /// `IsoGameCharacter.load` with `IsoMovingObject.load` (after the
    /// serialise and class bytes, which `IsoPlayer.load` reads).
    #[allow(clippy::too_many_arguments)]
    fn load_game_character(
        &mut self,
        sm: &ScriptManager,
        chars: &mut Characters,
        ctx: &dyn SimContext,
        env: &SaveEnv<'_>,
        r: &mut Reader<'_>,
        world_version: i32,
        set_on_fire: &mut dyn FnMut(&mut Self),
    ) -> ScriptResult {
        // IsoMovingObject.load: the saved offsets are dropped (a player's
        // are 0), IsoWorld.saveoffsetx/y are 0 outside the editor.
        let base = e(MovingBase::load(r, world_version, (0.0, 0.0)))?;
        self.vars.x = base.x;
        self.vars.last_x = base.x;
        self.vars.y = base.y;
        self.vars.last_y = base.y;
        self.vars.z = base.z;
        self.vars.last_z = base.z;
        // setForwardIsoDirection: the direction, then the forward vector
        // from it (which sets the direction from its angle).
        let dir = IsoDirections::ALL[(base.dir & 7) as usize];
        self.vars.dir = dir;
        let (fx, fy) = super::zombie::math::vector_from_direction(dir);
        self.set_forward_direction(fx, fy);
        if base.mod_data.is_some() {
            self.mod_data = base.mod_data;
        }
        if e(r.bool())? {
            let mut d = SurvivorDesc::new_unregistered(chars, sm)?;
            d.load(sm, chars, r, world_version)?;
            self.descriptor = Some(Rc::new(std::cell::RefCell::new(d)));
        }
        self.human_visual = Some(e(HumanVisual::load(r, env.tables.blood_body_parts))?);
        // `inventory.load(input, worldVersion)`: into the same (shared)
        // container object.
        let mut loaded = self.items.inventory.borrow().to_container();
        let saved = e(loaded.load_saved(r, world_version, env.items))?;
        self.saved_inventory_items = saved.iter().map(|s| s.map(|i| loaded.items[i].id())).collect();
        pz_inventory::shared::SharedContainer::fill_from(&self.items.inventory, loaded);
        self.asleep = e(r.bool())?;
        self.force_wake_up_time = e(r.f32())?;
        if !self.is_zombie() {
            let n = self.stats.load(&chars.sim.character_stats, &r.buf[r.pos.min(r.buf.len())..])?;
            r.pos += n;
            let mut sr = SaveReader { data: r.buf, pos: r.pos };
            self.load_body_damage(sm, chars, ctx, &mut sr, world_version)?;
            r.pos = sr.pos;
            if e(r.bool())? {
                match self.body_damage.thermoregulator.as_mut() {
                    Some(t) => e(t.load(r, world_version))?,
                    // Loaded into a throwaway one (logged in the original).
                    None => e(Thermoregulator::new(sm).load(r, world_version))?,
                }
            }
            self.load_xp(sm, chars, ctx, r, world_version)?;
            // Hand items: indices into includingObsoleteItems, which is
            // the saved list.
            let n = e(r.i32())?;
            if let Some(item) = self.saved_item(n) {
                self.items.primary_hand_item = item;
            }
            let n = e(r.i32())?;
            if let Some(item) = self.saved_item(n) {
                self.items.secondary_hand_item = item;
            }
        }
        if e(r.bool())? {
            set_on_fire(self);
        }
        self.depress_effect = e(r.f32())?;
        self.depress_first_take_time = e(r.f32())?;
        self.beta_effect = e(r.f32())?;
        self.beta_delta = e(r.f32())?;
        self.pain_effect = e(r.f32())?;
        self.pain_delta = e(r.f32())?;
        self.sleeping_tablet_effect = e(r.f32())?;
        self.sleeping_tablet_delta = e(r.f32())?;
        let n = e(r.i32())?;
        for _ in 0..n {
            let full_type = e(r.string())?;
            let already_read_pages = e(r.i32())?;
            self.read_books.push(ReadBook { full_type, already_read_pages });
        }
        self.reduce_infection_power = e(r.f32())?;
        let n = e(r.i32())?;
        for _ in 0..n {
            self.known_recipes.push(e(r.string())?);
        }
        self.last_hour_sleeped = e(r.i32())?;
        self.time_since_last_smoke = e(r.f32())?;
        self.beard_grow_timing = e(r.f32())?;
        self.hair_grow_timing = e(r.f32())?;
        // The cheat setters: without the capability they clear the cheat;
        // either way `PlayerCheats.set` ignores it unless cheats are allowed.
        let capable = Self::has_cheat_capability(ctx);
        for (cheat, since) in LOADED_CHEATS {
            if world_version < since {
                continue;
            }
            let b = e(r.bool())?;
            self.cheats.set(ctx, cheat, capable && b);
        }
        self.environment.sneaking = e(r.bool())?;
        self.death_drag_down = e(r.bool())?;
        let n = e(r.i32())?;
        for _ in 0..n {
            let title = e(r.string())?;
            let day = e(r.i32())?;
            // addReadLiterature(title, day): not for a title still read.
            // TODO(GameTime): `isLiteratureRead`'s cooldown; an entry here
            // is taken as still read (a fresh character has none).
            if !self.read_literature.contains_key(&title) {
                self.read_literature.insert(title, day);
            }
        }
        if world_version >= 222 {
            self.read_print_media.clear();
            let n = e(r.i32())?;
            for _ in 0..n {
                self.read_print_media.insert(e(r.string())?, ());
            }
        }
        self.last_animal_pet = e(r.i64())?;
        if world_version >= 231 {
            let cheat_types = chars.sim.cheat_types.clone();
            self.cheats.load(&cheat_types, r)?;
        }
        Ok(())
    }

    /// `IsoPlayer.save(output, isDebugSave)`: the record
    /// [`GameCharacter::load_player`] reads (the current world version's
    /// layout). The saved vehicle is [`GameCharacter::saved_vehicle`] (the
    /// original writes the vehicle the player sits in).
    pub fn save_player(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, env: &SaveEnv<'_>, w: &mut Writer) -> ScriptResult {
        self.save_game_character(sm, chars, ctx, env, w)?;
        w.f64(self.hours_survived);
        w.i32(self.zombie_kills);
        let worn = self.items.worn_items.as_ref().ok_or_else(npe)?;
        if worn.size() > 127 {
            return Err(ScriptError::new("java.lang.RuntimeException: too many worn items"));
        }
        w.i8(worn.size() as i8);
        let saved_index = |id: Option<i32>| -> i16 {
            id.and_then(|id| self.saved_inventory_items.iter().position(|s| *s == Some(id))).map_or(-1, |i| i as i16)
        };
        for worn in worn.items() {
            let location = worn.location.as_ref().ok_or_else(npe)?;
            w.string(&location.location.id());
            w.i16(saved_index(Some(worn.item)));
        }
        w.i16(saved_index(self.items.primary_hand_item));
        w.i16(saved_index(self.items.secondary_hand_item));
        w.i32(self.survivor_kills);
        self.nutrition.as_ref().ok_or_else(npe)?.save(w);
        let p = &self.player_fields;
        w.bool(p.all_chat_muted);
        w.string(&p.tag_prefix);
        for c in p.tag_color {
            w.f32(c);
        }
        w.opt_string(p.display_name.as_deref());
        w.bool(p.show_tag);
        w.bool(p.faction_pvp);
        w.bool(p.auto_drink);
        w.i8(self.extra_info_flags(ctx));
        match &self.saved_vehicle {
            Some(v) => {
                w.i8(1);
                w.f32(v.x);
                w.f32(v.y);
                w.i8(v.seat as i8);
                w.bool(v.running);
            }
            None => w.i8(0),
        }
        let mut mechanics = p.mechanics_item.clone();
        sort_hash_map(&mut mechanics, |(k, _)| long_hash(*k));
        w.i32(mechanics.len() as i32);
        for (k, v) in mechanics {
            w.i64(k);
            w.i64(v);
        }
        self.fitness.as_ref().ok_or_else(npe)?.save(w);
        w.i16(p.already_read_book.len() as i16);
        for t in &p.already_read_book {
            w.i16(env.items.registry_id(t));
        }
        w.i16(self.known_media_lines.len() as i16);
        for k in self.known_media_lines.keys() {
            w.string(k);
        }
        w.i8(self.descriptor.as_ref().ok_or_else(npe)?.borrow().voice_type as i8);
        w.i32(p.craft_history.len() as i32);
        for (k, v) in p.craft_history.iter() {
            let units: Vec<u16> = k.encode_utf16().collect();
            w.i32(units.len() as i32);
            for u in units {
                w.i16(u as i16);
            }
            w.i32(v.craft_count);
            w.f64(v.last_craft_time);
        }
        Ok(())
    }

    /// `IsoPlayer.getExtraInfoFlags`.
    fn extra_info_flags(&self, ctx: &dyn SimContext) -> i8 {
        let set = |c: &str| self.cheats.is_set(ctx, c);
        let show_admin_tag = [
            "INVISIBLE",
            "GOD_MODE",
            "NO_CLIP",
            "TIMED_ACTION_INSTANT",
            "UNLIMITED_CARRY",
            "UNLIMITED_ENDURANCE",
            "BUILD",
            "FARMING",
            "FISHING",
            "HEALTH",
            "MECHANICS",
            "MOVABLES",
            "CAN_SEE_EVERYONE",
            "CAN_HEAR_EVERYONE",
            "ZOMBIES_DONT_ATTACK",
        ]
        .iter()
        .any(|c| set(c));
        (set("GOD_MODE") as i8)
            | (set("INVISIBLE") as i8) << 1
            | (set("INVISIBLE") as i8) << 2
            | (set("NO_CLIP") as i8) << 3
            | (show_admin_tag as i8) << 4
            | (set("CAN_HEAR_EVERYONE") as i8) << 5
    }

    /// `IsoGameCharacter.save` with `IsoMovingObject.save`. Refreshes
    /// [`GameCharacter::saved_inventory_items`] to the written order, as the
    /// original does.
    fn save_game_character(&mut self, _sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, env: &SaveEnv<'_>, w: &mut Writer) -> ScriptResult {
        let base = MovingBase {
            offset_x: 0.0,
            offset_y: 0.0,
            x: self.vars.x,
            y: self.vars.y,
            z: self.vars.z,
            dir: self.vars.dir as i32,
            mod_data: self.mod_data.clone(),
        };
        base.save(w, env.tables.class_id("Player"));
        match &self.descriptor {
            Some(d) => {
                w.i8(1);
                d.borrow().save(w)?;
            }
            None => w.i8(0),
        }
        match &self.human_visual {
            Some(v) => v.save(w),
            None => HumanVisual::new(env.tables.blood_body_parts).save(w),
        }
        // `inventory.save(output, this)`: equipped items start no run.
        let equipped = |i: &pz_inventory::saved::SavedItem| self.items.is_equipped(i.id());
        let inventory = self.items.inventory.borrow().to_container();
        let order = e(inventory.save_saved(w, env.items, Some(&equipped)))?;
        self.saved_inventory_items = order.iter().map(|&i| Some(inventory.items[i].id())).collect();
        w.bool(self.asleep);
        w.f32(self.force_wake_up_time);
        if !self.is_zombie() {
            self.stats.save(&chars.sim.character_stats, &mut w.buf);
            self.body_damage.save_parts_and_main_fields(&mut w.buf);
            match self.body_damage.thermoregulator() {
                Some(t) => {
                    w.i8(1);
                    t.save(w);
                }
                None => w.i8(0),
            }
            self.save_xp(w);
            let index = |id: Option<i32>| {
                id.and_then(|id| inventory.items.iter().position(|i| i.id() == id)).map_or(-1, |i| i as i32)
            };
            w.i32(index(self.items.primary_hand_item));
            w.i32(index(self.items.secondary_hand_item));
        }
        w.bool(self.vars.on_fire);
        for f in [
            self.depress_effect,
            self.depress_first_take_time,
            self.beta_effect,
            self.beta_delta,
            self.pain_effect,
            self.pain_delta,
            self.sleeping_tablet_effect,
            self.sleeping_tablet_delta,
        ] {
            w.f32(f);
        }
        w.i32(self.read_books.len() as i32);
        for b in &self.read_books {
            w.string(&b.full_type);
            w.i32(b.already_read_pages);
        }
        w.f32(self.reduce_infection_power);
        w.i32(self.known_recipes.len() as i32);
        for k in &self.known_recipes {
            w.string(k);
        }
        w.i32(self.last_hour_sleeped);
        w.f32(self.time_since_last_smoke);
        w.f32(self.beard_grow_timing);
        w.f32(self.hair_grow_timing);
        for (cheat, _) in LOADED_CHEATS {
            w.bool(self.cheats.is_set(ctx, cheat));
        }
        w.bool(self.environment.sneaking);
        w.bool(self.death_drag_down);
        w.i32(self.read_literature.len() as i32);
        for (k, v) in self.read_literature.iter() {
            w.string(k);
            w.i32(*v);
        }
        w.i32(self.read_print_media.len() as i32);
        for k in self.read_print_media.keys() {
            w.string(k);
        }
        w.i64(self.last_animal_pet);
        self.cheats.save(&chars.sim.cheat_types, w);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
