//! Creates items the way Lua's `instanceItem` does
//! (`InventoryItemFactory.CreateItem`), each with `Rand` seeded as
//! `new java.util.Random(1000 + i)`, and prints the fields a local harness
//! prints for the original classes: class, id, name, colour, model index,
//! body location, `canBeEquipped`, the visual (clothing item, tint, base
//! texture, texture choice, hue, decal) and the next `Rand.Next(1000000)`.
//! No game data or harness output is part of this repository.
//!
//! ```text
//! instance_item_parity <game dir> <item types file> <out tsv>
//! ```

use pz_core::config::float_to_string as f;
use pz_lua_api::bind_items::{class_of, create_item, install_item_env, GameItemEnv};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

fn col(c: &[f32]) -> String {
    format!("{},{},{}", f(c[0]), f(c[1]), f(c[2]))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let game = PathBuf::from(&args[1]);
    let types = std::fs::read_to_string(&args[2]).map_err(|e| e.to_string())?;
    let mut fs = pz_fs::GameFs::new(&game);
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .map_err(|e| e.to_string())?;
    sm.load(&fs, &[]).map_err(|e| e.to_string())?;
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));
    {
        let core = pz_lua_api::bind_core::shared_core();
        pz_lua_api::bind_text::boot(&fs, &core.borrow())?;
    }
    pz_lua_api::bind_appearance::shared_population()
        .borrow_mut()
        .init(&mut fs);
    let assets = pz_inventory::ItemAssets::load(&game)?;
    let mut out = String::new();
    for (i, t) in types.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let mut env = GameItemEnv::new(Some(assets.clone()));
        env.rand = Some(pz_core::random::Rand::new(1000 + i as i64));
        let env = Rc::new(env);
        install_item_env(env.clone());
        let Some(item) = create_item(t, 1.0) else {
            writeln!(out, "{t}\tnull").unwrap();
            continue;
        };
        let it = item.borrow();
        let _ = write!(
            out,
            "{t}\tclass={}\tfull={}\tid={}\tname={}\tcol={}\tmodelIndex={}\tbody={}\tequip={}",
            class_of(&it),
            it.get_full_type(),
            it.id,
            it.name,
            col(&it.col),
            it.model_index,
            it.get_body_location().unwrap_or("null"),
            pz_lua_api::bind_item_visual::can_be_equipped(&it)
                .map_or("null".to_string(), |k| k.location.id()),
        );
        match &it.visual {
            Some(v) => {
                let _ = write!(
                    out,
                    "\tvis={}\ttint={}\tbase={}\ttc={}\thue={}\tdecal={}",
                    v.clothing_item_name,
                    v.tint.map_or("null".to_string(), |t| col(&t)),
                    v.base_texture,
                    v.texture_choice,
                    f(v.hue),
                    v.decal.as_deref().unwrap_or("null"),
                );
            }
            None => out.push_str("\tvis=null"),
        }
        writeln!(out, "\tnext={}", env.rand.as_ref().unwrap().next(1000000)).unwrap();
    }
    std::fs::write(&args[3], out).map_err(|e| e.to_string())
}
