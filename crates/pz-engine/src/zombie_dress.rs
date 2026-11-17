//! `ModelManager.dressInRandomOutfit`: a zombie dresses as it gets its
//! model (`ModelManager.Add`), before the body model is built.

use pz_characters::character::zombie::dress::{DressSettings, ZombieDressCx};
use pz_characters::population::zombie_defs::{ZoneView, ZoneWorld};
use pz_kahlua::Thread;
use pz_lua_api::bind_character_sim::LuaGameCharacter;
use pz_lua_api::bind_sandbox::{current_bool, current_double};
use pz_lua_api::bind_world::try_with_world_read;
use pz_lua_api::bind_zombie_outfits::{shared_zombie_outfits, LuaDressWorld};

/// The world `ZombiesZoneDefinition` reads. Each query reads the world on
/// its own: dressing creates items, and their Lua callbacks may take the
/// world in between.
struct WorldZones;

impl ZoneWorld for WorldZones {
    fn square_room(&self, x: i32, y: i32, z: i32) -> Option<Option<String>> {
        try_with_world_read(|w| {
            let Some(square) = w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z)) else {
                return Ok(None);
            };
            let id = square.borrow().room_id;
            let room = (id != -1)
                .then(|| w.meta_grid.as_ref()?.get_room_def_by_id(id).map(|r| r.name.clone()))
                .flatten();
            Ok(Some(room))
        })
        .ok()
        .flatten()
    }

    fn zones_at(&self, x: i32, y: i32, z: i32) -> Vec<ZoneView> {
        try_with_world_read(|w| {
            let Some(meta) = w.meta_grid.as_ref() else {
                return Ok(Vec::new());
            };
            Ok(meta
                .get_zones_at(x, y, z)
                .into_iter()
                .map(|id| {
                    let zone = meta.zone(id);
                    ZoneView {
                        key: id.0 as u64,
                        name: Some(zone.name.clone()),
                        zone_type: Some(zone.zone_type.clone()),
                        id: zone.id.map(|u| ((u.msb as u64 as u128) << 64) | u.lsb as u64 as u128),
                    }
                })
                .collect())
        })
        .unwrap_or_default()
    }
}

/// An integer (or enum) sandbox option, else its default.
fn sandbox_int(name: &str, default: i32) -> i32 {
    current_double(name).map_or(default, |v| v as i32)
}

/// `ModelManager.dressInRandomOutfit(chr)` for a zombie about to get its
/// model. Nothing for a zombie that is already dressed.
pub fn dress_zombie(thread: &Thread, chr: &mut LuaGameCharacter) -> Result<(), String> {
    if !chr.is_zombie() || !(chr.zombie.dress_in_random_outfit || !chr.zombie.persistent_outfit_init) {
        return Ok(());
    }
    let days = pz_lua_api::bind_climate::shared_game_time()
        .borrow()
        .world_age_days_since_begin(sandbox_int("TimeSinceApo", 1));
    let settings = DressSettings {
        clothing_degradation: sandbox_int("ClothingDegradation", 3),
        remove_zombie_loot: current_bool("RemoveZombieLoot").unwrap_or(false),
        tutorial: pz_lua_api::bind_core::shared_core().borrow().get_game_mode() == "Tutorial",
        // `IsoWorld.getWorldAgeDays()` is `(float) getWorldAgeDaysSinceBegin()`.
        world_age_days: days as f32,
        world_age_days_since_begin: days,
    };
    let globals = thread.globals().clone();
    let chars = pz_lua_api::bind_characters::shared_characters();
    let outfits = shared_zombie_outfits();
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let no_black_clothes = current_bool("NoBlackClothes").unwrap_or(true);
    let result = pz_lua_api::bind_appearance::with_dress_context_options(thread, no_black_clothes, |dress| {
        let chars = chars.borrow();
        let mut outfits = outfits.borrow_mut();
        let mut create = |t: &str| pz_lua_api::bind_items::create_item(t, 1.0).map(|i| i.borrow().clone());
        let loot = pz_lua_api::item_picker::loot_settings();
        let loot_modifier = |t: &str| pz_lua_api::item_picker::loot_modifier(&sm, &loot, t);
        let mut world = LuaDressWorld {
            globals,
            zones: &WorldZones,
            create_item: &mut create,
            loot_modifier: &loot_modifier,
            chance_of_attached_weapon: sandbox_int("ZombieLore.ChanceOfAttachedWeapon", 6),
        };
        let mut cx = ZombieDressCx { chars: &chars, dress, outfits: &mut outfits, world: &mut world, settings };
        chr.model_manager_dress_in_random_outfit(&mut cx)
    });
    match result {
        None => Err("no outfits or clothing assets to dress the zombie".into()),
        Some(r) => r.map_err(|e| e.to_string()),
    }
}
