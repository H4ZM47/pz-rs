//! Load chunks with the full object creation path and print every square's
//! objects, for comparing against the original loader on a machine with the
//! game installed. Results are recorded as notes only.
//!
//! ```text
//! objects_digest <game dir> <fs root> <map> <sprite tables> <object tables>
//!                <seed> <locked houses> <out file> <wx0> <wy0> <wx1> <wy1>
//! objects_digest find <fs root> <map>
//! ```
//!
//! `<fs root>` holds `media/maps/<map>`. Room `lightsActive` flags are set
//! from room coordinates (not `Rand`) so the comparison does not depend on
//! header-loading draws; `Rand` is a `java.util.Random(seed)` reseeded after
//! the meta grid is built. The tables files are read from the installed game
//! and are not part of this repository.

use pz_world::ObjectSlot;
use std::fmt::Write as _;

use pz_fs::GameFs;
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectKind, ObjectLoadHooks,
    ObjectLoadSettings, ObjectTables, SpriteTables, WorldGenBounds,
};

/// `java.util.Random`.
struct JavaRandom(u64);

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut r = self.bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.bits(31);
        }
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

/// A fresh world with no zones or generators: the power grid is on except
/// in derelict and user-defined rooms (`IsoGridSquare.isNoPower`).
struct GridOn;
impl ObjectLoadHooks for GridOn {
    fn square_powered(&mut self, _sq: pz_world::SquareRef, room: &pz_world::RoomDef) -> bool {
        !room.name.to_lowercase().contains("derelict") && !room.user_defined
    }
}

fn grid(fs_root: &str, map: &str) -> IsoMetaGrid {
    IsoMetaGrid::create(
        &GameFs::new(fs_root),
        CreateParams {
            game_map: map,
            world_gen: WorldGenBounds::default(),
            rand: &mut JavaRandom::new(0),
            voronois: Vec::new(),
        },
    )
    .expect("create")
}

fn name(m: &IsoSpriteManager, s: Option<usize>) -> &str {
    s.map_or("null", |s| m.sprites[s].name.as_deref().unwrap_or("null"))
}

fn object_line(m: &IsoSpriteManager, o: &IsoObject) -> String {
    let mut l = format!(
        "  {} {} tile={} out={} tint={:x},{:x},{:x} ry={:x}",
        o.kind.class_name(),
        name(m, o.sprite.tile_projection()),
        o.tile.as_deref().unwrap_or("null"),
        o.outline_on_mouseover as u8,
        o.tint_r.to_bits(),
        o.tint_g.to_bits(),
        o.tint_b.to_bits(),
        o.render_y_offset.to_bits(),
    );
    if let Some(a) = &o.attached_anim_sprite {
        l.push_str(" att=");
        for s in a {
            write!(l, "{},", name(m, Some(s.sprite))).unwrap();
        }
    }
    for c in o.container.iter().chain(o.secondary_containers.iter()) {
        write!(
            l,
            " cont={}/{}/{}/{}",
            c.kind.as_deref().unwrap_or("null"),
            c.capacity,
            c.container_position.as_deref().unwrap_or("null"),
            c.freezer_position.as_deref().unwrap_or("null")
        )
        .unwrap();
    }
    let b = |v: bool| v as u8;
    match &o.kind {
        ObjectKind::Door(d) => write!(
            l,
            " door={}{}{}{} hp={}/{} {} {}",
            b(d.north),
            b(d.open),
            b(d.locked),
            b(d.locked_by_key),
            d.health,
            d.max_health,
            name(m, d.open_sprite),
            name(m, d.closed_sprite)
        ),
        ObjectKind::Window(w) => write!(
            l,
            " win={}{}{}{}{}{} hp={} {} {} {} {}",
            b(w.north),
            b(w.locked),
            b(w.perma_locked),
            b(w.open),
            b(w.destroyed),
            b(w.glass_removed),
            w.health,
            name(m, w.open_sprite),
            name(m, w.closed_sprite),
            name(m, w.smashed_sprite),
            name(m, w.glass_removed_sprite)
        ),
        ObjectKind::Curtain(c) => write!(
            l,
            " curtain={}{} {} {}",
            b(c.north),
            b(c.open),
            name(m, c.open_sprite),
            name(m, c.closed_sprite)
        ),
        ObjectKind::WindowFrame { north } => write!(l, " frame={}", b(*north)),
        ObjectKind::LightSwitch(s) => {
            write!(
                l,
                " switch={}{}{}{} room={} rgb={:x},{:x},{:x}",
                b(s.activated),
                b(s.light_room),
                b(s.street_light),
                b(s.can_be_modified),
                s.room_id,
                s.primary_r.to_bits(),
                s.primary_g.to_bits(),
                s.primary_b.to_bits()
            )
            .unwrap();
            for li in &s.lights {
                write!(
                    l,
                    " light={},{},{},{:x},{:x},{:x},{},{}{}",
                    li.x,
                    li.y,
                    li.z,
                    li.r.to_bits(),
                    li.g.to_bits(),
                    li.b.to_bits(),
                    li.radius,
                    b(li.active),
                    b(li.hydro_powered)
                )
                .unwrap();
            }
            Ok(())
        }
        ObjectKind::Barbecue(q) => write!(
            l,
            " bbq={} fuel={} {} {}",
            b(q.has_propane_tank),
            q.fuel_amount_minutes,
            name(m, q.normal_sprite),
            name(m, q.no_tank_sprite)
        ),
        ObjectKind::Tree(t) => write!(l, " tree={},{},{}", t.size, t.log_yield, t.damage),
        ObjectKind::Radio(d) | ObjectKind::Television(d) => write!(
            l,
            " device={} vol={:x} on={} pow={:x} presets={}",
            d.channel,
            d.device_volume.to_bits(),
            b(d.is_turned_on),
            d.power_delta.to_bits(),
            d.presets.len()
        ),
        _ => Ok(()),
    }
    .unwrap();
    l
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "find" {
        let g = grid(&a[2], &a[3]);
        for &bi in &g.buildings {
            let b = &g.building_defs[bi.0 as usize];
            if b.x / 256 != (b.x2 - 1) / 256 || b.y / 256 != (b.y2 - 1) / 256 {
                println!("building {} ({},{})-({},{})", b.id, b.x, b.y, b.x2, b.y2);
            }
        }
        let orphans = g.room_defs.iter().filter(|r| r.building.is_none()).count();
        println!("rooms without a building: {orphans}");
        return;
    }
    let game = std::path::Path::new(&a[1]);
    let mut g = grid(&a[2], &a[3]);
    g.assign_alarms(&mut JavaRandom::new(0), false, false, 1, || 0);
    for r in &g.room_defs {
        r.lights_active.set((r.x + r.y + r.level) & 1 == 0);
    }
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[4]).unwrap()).unwrap();
    let objects = ObjectTables::from_text(&std::fs::read_to_string(&a[5]).unwrap()).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut rand = JavaRandom::new(a[6].parse().unwrap());
    let settings = ObjectLoadSettings {
        locked_houses: a[7].parse().unwrap(),
        ..Default::default()
    };
    let fix2x = Fix2xTable::default();
    let (wx0, wy0, wx1, wy1): (i32, i32, i32, i32) = (
        a[9].parse().unwrap(),
        a[10].parse().unwrap(),
        a[11].parse().unwrap(),
        a[12].parse().unwrap(),
    );
    let blood_body_parts = pz_world::save::SaveTables::from_game_jar(&game.join("projectzomboid.jar")).expect("visual body tables").blood_body_parts;
    let mut cell = IsoCell::default();
    let mut out = String::new();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for wx in wx0..=wx1 {
        for wy in wy0..=wy1 {
            let mut env = ChunkLoadEnv {
                blood_body_parts: blood_body_parts,
                sprites: &mut sprites,
                fix2x: &fix2x,
                rand: &mut rand,
                tables: &objects,
                settings,
                hooks: &mut GridOn,
            };
            cell.load_chunk(&g, wx, wy, &mut env);
            let ch = cell.chunks.remove(&(wx, wy)).unwrap();
            writeln!(
                out,
                "chunk {wx},{wy} levels {}..{}",
                ch.min_level, ch.max_level
            )
            .unwrap();
            for sq in ch.squares() {
                let bid = |b: Option<pz_world::BuildingIdx>| {
                    b.map_or(-1, |b| g.building_defs[b.0 as usize].id)
                };
                writeln!(
                    out,
                    " sq {},{},{} room={} assoc={} roof={}",
                    sq.borrow().x,
                    sq.borrow().y,
                    sq.borrow().z,
                    sq.borrow().room_id,
                    bid(sq.borrow().associated_building),
                    bid(sq.borrow().roof_hide_building)
                )
                .unwrap();
                for o in sq.borrow().objects.iter().map(ObjectSlot::borrow) {
                    *counts.entry(o.kind.class_name()).or_default() += 1;
                    writeln!(out, "{}", object_line(&sprites, &o)).unwrap();
                }
            }
        }
    }
    std::fs::write(&a[8], out).unwrap();
    println!("{counts:?} next={}", rand.next(1 << 30));
}
