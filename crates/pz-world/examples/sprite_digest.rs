//! Load the installed game's tile definitions into an `IsoSpriteManager` and
//! print a digest of every sprite in `namedMap`, for parity checks.
//!
//! `cargo run --release -p pz-world --example sprite_digest -- <game dir> <tables file> [lines out]`
//!
//! The tables file holds the `IsoFlagType`, `IsoObjectType` and `FasciaEdge`
//! constant names (three comma-separated lines) read from the installed game;
//! it is not part of this repository.

use std::fmt::Write as _;

use pz_world::sprite::{IsoSpriteManager, SpriteTables};

fn fnv1a(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= *b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = std::path::PathBuf::from(args.next().expect("game dir"));
    let tables_text = std::fs::read_to_string(args.next().expect("tables file")).unwrap();
    let out = args.next();
    let tables = SpriteTables::from_text(&tables_text).unwrap();
    let m = IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();

    let mut names: Vec<&String> = m.named_map.keys().collect();
    names.sort();
    let mut all = String::new();
    let mut h = 0xcbf29ce484222325u64;
    for name in names {
        let s = &m.sprites[m.named_map[name]];
        let (f1, f2) = s.properties.flags();
        let mut line = format!(
            "{name}\t{}\t{}\t{f1:x}\t{f2:x}\t",
            s.id,
            m.tables.object_type_name(s.tile_type)
        );
        for (k, v) in s.properties.property_pairs(&m.alias) {
            write!(line, "{k}={v};").unwrap();
        }
        write!(
            line,
            "\t{}|{}|{}{}{}{}{}{}{}|{}|{}|{}|{}",
            s.tileset_name.as_deref().unwrap_or("null"),
            s.tile_sheet_index,
            s.cut_n as u8,
            s.cut_w as u8,
            s.attached_floor as u8,
            s.is_bush as u8,
            s.solid as u8,
            s.solid_trans as u8,
            s.move_with_wind as u8,
            s.firerequirement,
            s.wind_type,
            s.render_layer,
            s.parent_object_name.as_deref().unwrap_or("null"),
        )
        .unwrap();
        match s.sprite_grid {
            Some(g) => {
                let g = &m.grids[g];
                let (x, y, z) = g.sprite_pos(m.named_map[name]).unwrap_or((-1, -1, -1));
                write!(
                    line,
                    "\tgrid {}x{}x{} @{x},{y},{z}",
                    g.width, g.height, g.levels
                )
                .unwrap();
            }
            None => line.push_str("\t-"),
        }
        line.push('\n');
        fnv1a(&mut h, line.as_bytes());
        all.push_str(&line);
    }
    println!(
        "sprites {} (intMap {}), properties {}, digest {h:016x}",
        m.named_map.len(),
        m.int_map.len(),
        m.alias.properties.len()
    );
    if let Some(out) = out {
        std::fs::write(out, all).unwrap();
    }
}
