//! Digests of decoded lot data, for comparing against the original loader
//! on a machine with the game installed. Results are recorded as notes only.
//!
//! `world_digest <dir containing media/> <map> header|lot <cellX> <cellY>...`

use pz_fs::GameFs;
use pz_world::{CreateParams, GameRand, IsoLot, IsoMetaGrid, WorldGenBounds};

struct Fnv(u64);
impl Fnv {
    fn b(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }
    fn i(&mut self, v: i32) {
        v.to_le_bytes().iter().for_each(|&b| self.b(b));
    }
    fn l(&mut self, v: i64) {
        v.to_le_bytes().iter().for_each(|&b| self.b(b));
    }
    fn s(&mut self, s: &str) {
        self.i(s.len() as i32);
        s.bytes().for_each(|b| self.b(b));
    }
}

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fs = GameFs::new(&a[1]);
    let grid = IsoMetaGrid::create(
        &fs,
        CreateParams {
            game_map: &a[2],
            world_gen: WorldGenBounds::default(),
            rand: &mut Zero,
            voronois: Vec::new(),
        },
    )
    .expect("create");
    for pair in a[4..].chunks(2) {
        let (cx, cy): (i32, i32) = (pair[0].parse().unwrap(), pair[1].parse().unwrap());
        let mf = 0;
        let header = grid.map_files[mf].get_lot_header(cx, cy).expect("header");
        let mut h = Fnv(0xcbf29ce484222325);
        if a[3] == "header" {
            let cell = grid.get_cell_data(cx, cy).expect("cell");
            h.i(header.version);
            h.i(header.min_level);
            h.i(header.max_level);
            h.i(header.tiles_used.len() as i32);
            header.tiles_used.iter().for_each(|t| h.s(t));
            h.i(cell.room_list.len() as i32);
            for r in &cell.room_list {
                let r = &grid.room_defs[r.0 as usize];
                h.s(&r.name);
                h.i(r.level);
                h.l(r.id);
                h.l(r.meta_id);
                for v in [r.x, r.y, r.x2, r.y2, r.area] {
                    h.i(v);
                }
                h.i(r.rects.len() as i32);
                for rc in &r.rects {
                    for v in [rc.x, rc.y, rc.w, rc.h] {
                        h.i(v);
                    }
                }
                h.i(r.objects.len() as i32);
                for o in r.objects.iter() {
                    for v in [o.kind, o.x, o.y] {
                        h.i(v);
                    }
                }
                h.l(r
                    .building
                    .map_or(-1, |b| grid.building_defs[b.0 as usize].id));
            }
            h.i(cell.buildings.len() as i32);
            for b in &cell.buildings {
                let b = &grid.building_defs[b.0 as usize];
                h.l(b.id);
                h.l(b.meta_id);
                for v in [
                    b.x,
                    b.y,
                    b.x2,
                    b.y2,
                    b.rooms.len() as i32,
                    b.emptyoutside.len() as i32,
                ] {
                    h.i(v);
                }
            }
            (0..1024).for_each(|i| h.b(header.get_zombie_intensity(i)));
            println!(
                "header {cx},{cy} rooms={} buildings={} tiles={} levels={}..{} fnv={:x}",
                cell.room_list.len(),
                cell.buildings.len(),
                header.tiles_used.len(),
                header.min_level,
                header.max_level,
                h.0
            );
        } else {
            let mut lot = IsoLot::default();
            let mut tiles = 0i64;
            for x in 0..32 {
                for y in 0..32 {
                    lot.load(&grid.map_files, mf, cx, cy, cx * 32 + x, cy * 32 + y);
                    h.i(x);
                    h.i(y);
                    h.i(lot.min_level);
                    h.i(lot.max_level);
                    for z in header.min_level..=header.max_level {
                        for sx in 0..8 {
                            for sy in 0..8 {
                                let o = lot.offset_in_data
                                    [(sx + sy * 8 + (z - header.min_level) * 64) as usize];
                                let count = if o < 0 { 0 } else { lot.data[o as usize] };
                                h.i(count);
                                for n in 0..count {
                                    h.i(lot.data[(o + 1 + n) as usize]);
                                }
                                tiles += count as i64;
                            }
                        }
                    }
                }
            }
            println!("lot {cx},{cy} tiles={tiles} fnv={:x}", h.0);
        }
    }
}
