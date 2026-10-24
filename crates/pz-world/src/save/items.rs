//! Inventory items in chunk files: the item runtime lives in
//! `pz-inventory`; this re-exports what the chunk code uses.
//!
//! `InventoryItem.saveWithSize` writes a 4-byte length before the item,
//! and `loadItem` uses it to skip what it did not read. Items whose class
//! the codec supports are decoded into `InventoryItem`s (so re-saves apply
//! the original's load-time changes); others are kept as their bytes.

pub use pz_inventory::container::{load_items, save_items, save_single};
pub use pz_inventory::env::WorldSpriteInfo;
pub use pz_inventory::env::{ItemEnv as ItemRegistry, NoItemEnv as AllItems};
pub use pz_inventory::saved::{read_record, SavedItem as ItemBlob};

use crate::sprite::IsoSpriteManager;
use std::collections::HashMap;

/// The `Moveable.ReadFromWorldSprite` properties of every `IsMoveAble`
/// sprite, for [`pz_inventory::factory::ScriptItems::world_sprites`].
pub fn moveable_sprites(m: &IsoSpriteManager) -> HashMap<String, WorldSpriteInfo> {
    let mut out = HashMap::new();
    for (name, &idx) in &m.named_map {
        let s = &m.sprites[idx];
        if !m.has_prop(s, "IsMoveAble") {
            continue;
        }
        let prop = |p: &str| m.prop(s, p).map(str::to_string);
        let sprite_grid = s.sprite_grid.map(|g| {
            let g = &m.grids[g];
            let index = g
                .sprites
                .iter()
                .position(|&x| x == Some(idx))
                .map_or(-1, |i| i as i32);
            (index, g.sprites.len() as i32)
        });
        out.insert(
            name.clone(),
            WorldSpriteInfo {
                custom_item: prop("CustomItem"),
                pick_up_weight: prop("PickUpWeight").and_then(|w| w.trim().parse().ok()),
                custom_name: prop("CustomName"),
                group_name: prop("GroupName"),
                sprite_grid,
                force_single_item: m.has_prop(s, "ForceSingleItem"),
            },
        );
    }
    out
}
