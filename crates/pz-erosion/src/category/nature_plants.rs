//! `NaturePlants`.

use super::{clerp, parse_suffix, Base, Ctx, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::season::ErosionSeason;
use crate::tables::{ErosionTables, PlantInit};

#[derive(Clone, Debug, Default)]
pub struct NaturePlants {
    pub base: Base,
    soil_ref: Vec<Vec<i32>>,
    spawn_chance: Vec<i32>,
    pub objs: Vec<ErosionObj>,
    plants: Vec<PlantInit>,
}

const GROUNDCOVER: &str = "vegetation_groundcover_01_";

impl NaturePlants {
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes the groundcover objects below index `i`.
    fn remove_groundcover_below(host: &mut dyn ErosionHost, sq: Sq, mut i: usize) {
        loop {
            if i <= 1 {
                return;
            }
            i -= 1;
            let Some(&o) = host.objects(sq).get(i) else {
                continue;
            };
            if host.sprite_name(sq, o).is_some_and(|n| n.starts_with(GROUNDCOVER)) {
                host.remove_tile_object_erosion_no_recalc(sq, o);
            }
        }
    }

    pub fn replace_existing_object(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData, ctx: &mut Ctx<'_>) -> bool {
        let objs = host.objects(sq);
        for i in (1..objs.len()).rev() {
            let o = objs[i];
            let Some(name) = host.sprite_name(sq, o) else {
                continue;
            };
            if name.starts_with("d_plants_1_") {
                let Some(id) = parse_suffix(&name, "d_plants_1_") else {
                    continue;
                };
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = if id < 32 {
                    id % 8
                } else if id < 48 {
                    id % 8 + 8
                } else {
                    id % 8 + 16
                };
                data.stage = 0;
                data.spawn_time = 0;
                host.remove_tile_object_erosion_no_recalc(sq, o);
                return true;
            }
            let suffix = name.strip_prefix(GROUNDCOVER);
            if matches!(suffix, Some("16" | "17")) {
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = 21;
                data.stage = 0;
                data.spawn_time = 0;
                host.remove_tile_object_erosion_no_recalc(sq, o);
                Self::remove_groundcover_below(host, sq, i);
                return true;
            }
            if !matches!(suffix, Some("18" | "19" | "20" | "21" | "22" | "23")) {
                continue;
            }
            let pick = sd.rand(sq.0, sq.1, self.plants.len() as i32, &mut ctx.rand);
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = pick;
            data.stage = 0;
            data.spawn_time = 0;
            host.remove_tile_object_erosion_no_recalc(sq, o);
            Self::remove_groundcover_below(host, sq, i);
            return true;
        }
        false
    }

    pub fn validate_spawn(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData, has_wall: bool, ctx: &mut Ctx<'_>) -> bool {
        if host.objects(sq).len() > if has_wall { 2 } else { 1 } {
            return false;
        }
        if sd.soil < 0 || sd.soil >= self.soil_ref.len() as i32 {
            return false;
        }
        let row = self.soil_ref[sd.soil as usize].clone();
        let e = sd.noise_main_int;
        if sd.rand(sq.0, sq.1, 101, &mut ctx.rand) < self.spawn_chance.get(e as usize).copied().unwrap_or(0) {
            let pick = sd.rand(sq.0, sq.1, row.len() as i32, &mut ctx.rand);
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = row[pick as usize] - 1;
            data.stage = 0;
            data.spawn_time = 100 - e;
            return true;
        }
        false
    }

    pub fn update(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &SquareData, data: &mut CategoryData, e_tick: i32, season: &ErosionSeason) -> Keep {
        if e_tick < data.spawn_time || data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            return Keep::Clear;
        }
        let obj = &mut self.objs[data.game_obj as usize];
        let disp = self.base.current_season(sd.magic_num, season);
        let bloom = self.base.current_bloom(sd.magic_num, obj, season);
        self.base.update_obj(host, sq, data, obj, false, 0, disp, bloom)
    }

    pub fn init(&mut self, tables: &ErosionTables) {
        self.soil_ref = tables.plants_soil_ref.clone();
        self.plants = tables.plants.clone();
        self.spawn_chance = vec![0; 100];
        for i in 0..100usize {
            let f = i as i32;
            if (20..50).contains(&f) {
                self.spawn_chance[i] = clerp((f - 20) as f32 / 30.0, 0.0, 8.0) as i32;
            } else if (50..80).contains(&f) {
                self.spawn_chance[i] = clerp((f - 50) as f32 / 30.0, 8.0, 0.0) as i32;
            }
        }
        self.base.set_disp(5, 0, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 2, true);
        self.base.set_disp(4, 4, 0, true);
        let s = |n: usize| format!("d_plants_1_{n}");
        let spring: Vec<String> = (0..=7).map(s).collect();
        let autumn: Vec<String> = (8..=15).map(s).collect();
        let mut offset = 16;
        for (i, plant) in self.plants.iter().enumerate() {
            if i >= 8 {
                offset = 24;
            }
            if i >= 16 {
                offset = 32;
            }
            let mut spr = ErosionObjSprites::new(1, &plant.name, false, plant.has_flower, false);
            spr.set_base_list(0, &spring, 1);
            spr.set_base_list(0, &autumn, 4);
            spr.set_base(0, &s(offset + i), 2);
            spr.set_flower(0, &s(offset + i + 8));
            let (bs, be) = if plant.has_flower { (plant.bloomstart, plant.bloomend) } else { (0.0, 0.0) };
            self.objs.push(ErosionObj::new(spr, 30, bs, be, false));
        }
    }
}
