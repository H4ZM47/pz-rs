//! `NatureBush`.

use super::{clerp, grown_stage, parse_suffix, Base, Ctx, IceQueenSink, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::season::ErosionSeason;
use crate::tables::{BushInit, ErosionTables};

#[derive(Clone, Debug, Default)]
pub struct NatureBush {
    pub base: Base,
    soil_ref: Vec<Vec<i32>>,
    pub objs: Vec<ErosionObj>,
    spawn_chance: Vec<i32>,
    bush: Vec<BushInit>,
}

impl NatureBush {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_existing_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        ctx: &mut Ctx<'_>,
    ) -> bool {
        let mut replaced = false;
        let n = host.objects(sq).len();
        for i in (1..n).rev() {
            let objs = host.objects(sq);
            let Some(&o) = objs.get(i) else {
                continue;
            };
            let Some(name) = host.sprite_name(sq, o) else {
                continue;
            };
            if name.starts_with("vegetation_foliage") {
                let mut soil = sd.soil;
                if soil < 0 || soil >= self.soil_ref.len() as i32 {
                    soil = sd.rand(sq.0, sq.1, self.soil_ref.len() as i32, &mut ctx.rand);
                }
                let row = self.soil_ref[soil as usize].clone();
                let e = sd.noise_main_int;
                let pick = sd.rand(sq.0, sq.1, row.len() as i32, &mut ctx.rand);
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = row[pick as usize] - 1;
                data.max_stage = (e as f32 / 60.0).floor() as i32;
                data.stage = data.max_stage;
                data.spawn_time = 0;
                host.remove_tile_object(sq, o);
                replaced = true;
            }
            if !name.starts_with("f_bushes_1_") {
                continue;
            }
            let Some(id) = parse_suffix(&name, "f_bushes_1_") else {
                continue;
            };
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = id % 16;
            data.max_stage = 1;
            data.stage = 1;
            data.spawn_time = 0;
            host.remove_tile_object(sq, o);
            replaced = true;
        }
        replaced
    }

    pub fn validate_spawn(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        has_wall: bool,
        ctx: &mut Ctx<'_>,
    ) -> bool {
        if host.objects(sq).len() > if has_wall { 2 } else { 1 } {
            return false;
        }
        if sd.soil < 0 || sd.soil >= self.soil_ref.len() as i32 {
            return false;
        }
        let row = self.soil_ref[sd.soil as usize].clone();
        let e = sd.noise_main_int;
        let r = sd.rand(sq.0, sq.1, 101, &mut ctx.rand);
        if r < self.spawn_chance.get(e as usize).copied().unwrap_or(0) {
            let pick = sd.rand(sq.0, sq.1, row.len() as i32, &mut ctx.rand);
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = row[pick as usize] - 1;
            data.max_stage = (e as f32 / 60.0).floor() as i32;
            data.stage = 0;
            data.spawn_time = 100 - e;
            return true;
        }
        false
    }

    pub fn update(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &SquareData,
        data: &mut CategoryData,
        e_tick: i32,
        season: &ErosionSeason,
    ) -> Keep {
        if e_tick < data.spawn_time || data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            data.do_nothing = true;
            return Keep::Keep;
        }
        let obj = &mut self.objs[data.game_obj as usize];
        let max_stage = data.max_stage;
        let mut stage = grown_stage(e_tick, data.spawn_time, obj.cycle_time, max_stage);
        if stage < data.stage {
            stage = data.stage;
        }
        if stage > max_stage {
            stage = max_stage;
        }
        let disp = self.base.current_season(sd.magic_num, season);
        let bloom = self.base.current_bloom(sd.magic_num, obj, season);
        self.base
            .update_obj(host, sq, data, obj, false, stage, disp, bloom)
    }

    pub fn init(&mut self, tables: &ErosionTables, ice: &mut IceQueenSink) {
        self.soil_ref = tables.bush_soil_ref.clone();
        self.bush = tables.bush.clone();
        self.spawn_chance = vec![0; 100];
        for i in 0..100 {
            if (45..60).contains(&i) {
                self.spawn_chance[i] = clerp((i as i32 - 45) as f32 / 15.0, 0.0, 20.0) as i32;
            }
            if (60..90).contains(&i) {
                self.spawn_chance[i] = clerp((i as i32 - 60) as f32 / 30.0, 20.0, 0.0) as i32;
            }
        }
        self.base.set_disp(5, 0, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 2, true);
        self.base.set_disp(4, 4, 0, true);
        let s = |n: i32| format!("f_bushes_1_{n}");
        for (i, b) in self.bush.iter().enumerate() {
            let i = i as i32;
            let trunk = i - ((i as f32 / 8.0) as f64).floor() as i32 * 8;
            let mut spr = ErosionObjSprites::new(2, &b.name, true, b.has_flower, true);
            let base_id = trunk;
            let snow_id = base_id + 16;
            let spring_id = snow_id + 16;
            let autumn_id = spring_id + 16;
            let summer_id = 64 + i;
            let bloom_id = summer_id + 16;
            spr.set_base(0, &s(base_id), 0);
            spr.set_base(1, &s(base_id + 8), 0);
            ice.push((s(base_id), s(snow_id)));
            ice.push((s(base_id + 8), s(snow_id + 8)));
            spr.set_child_sprite(0, &s(spring_id), 1);
            spr.set_child_sprite(1, &s(spring_id + 8), 1);
            spr.set_child_sprite(0, &s(autumn_id), 4);
            spr.set_child_sprite(1, &s(autumn_id + 8), 4);
            spr.set_child_sprite(0, &s(summer_id), 2);
            spr.set_child_sprite(1, &s(summer_id + 32), 2);
            if b.has_flower {
                spr.set_flower(0, &s(bloom_id));
                spr.set_flower(1, &s(bloom_id + 32));
            }
            let (bs, be) = if b.has_flower {
                (b.bloomstart, b.bloomend)
            } else {
                (0.0, 0.0)
            };
            self.objs.push(ErosionObj::new(spr, 60, bs, be, true));
        }
    }
}
