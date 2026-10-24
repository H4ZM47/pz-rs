//! `NatureTrees`.

use super::{clerp, grown_stage, Base, Ctx, IceQueenSink, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::season::ErosionSeason;
use crate::tables::{ErosionTables, TreeInit};

#[derive(Clone, Debug, Default)]
pub struct NatureTrees {
    pub base: Base,
    soil_ref: Vec<Vec<i32>>,
    trees: Vec<TreeInit>,
    spawn_chance: Vec<i32>,
    pub objs: Vec<ErosionObj>,
}

/// `PZMath.tryParseInt`.
fn try_parse_int(s: &str, default: i32) -> i32 {
    let t = crate::java::trim(s);
    if t.is_empty() {
        return default;
    }
    t.parse().unwrap_or(default)
}

impl NatureTrees {
    pub fn new() -> Self {
        Self::default()
    }

    /// The `soilRef` row for the square, drawing a random row for a soil
    /// outside the table.
    fn soil_row(&self, sq: Sq, sd: &mut SquareData, ctx: &mut Ctx<'_>) -> Vec<i32> {
        let mut soil = sd.soil;
        if soil < 0 || soil >= self.soil_ref.len() as i32 {
            soil = sd.rand(sq.0, sq.1, self.soil_ref.len() as i32, &mut ctx.rand);
        }
        self.soil_ref[soil as usize].clone()
    }

    pub fn replace_existing_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        ctx: &mut Ctx<'_>,
    ) -> bool {
        let objs = host.objects(sq);
        for i in (1..objs.len()).rev() {
            let o = objs[i];
            let Some(name) = host.sprite_name(sq, o) else {
                continue;
            };
            for (prefix, base_stage) in [("jumbo_tree_01", 5), ("vegetation_trees", 3)] {
                if !name.starts_with(prefix) {
                    continue;
                }
                let row = self.soil_row(sq, sd, ctx);
                let e = sd.noise_main_int;
                let pick = sd.rand(sq.0, sq.1, row.len() as i32, &mut ctx.rand);
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = row[pick as usize] - 1;
                data.max_stage = base_stage + (e as f32 / 51.0).floor() as i32 - 1;
                data.stage = data.max_stage;
                data.spawn_time = 0;
                data.disp_season = -1;
                let obj_name = self.objs[data.game_obj as usize].name.clone();
                host.set_object_name(sq, o, Some(&obj_name));
                data.has_spawned = true;
                return true;
            }
            for t in 0..self.trees.len() {
                let tree = &self.trees[t];
                let (max_stage, stage) = if name.starts_with(&tree.tileset_name) {
                    (3, 3)
                } else if name.starts_with(&tree.jumbo_tileset_name) {
                    let p = name.rfind('_').map_or(0, |p| p + 1);
                    let index = try_parse_int(&name[p..], 0);
                    (5, if index % 2 == 0 { 4 } else { 5 })
                } else if name.starts_with(&tree.jumbo_xl_tileset_name) {
                    (6, 6)
                } else if name.starts_with(&tree.jumbo_xxl_tileset_name) {
                    (7, 7)
                } else {
                    continue;
                };
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = t as i32;
                data.max_stage = max_stage;
                data.stage = stage;
                data.spawn_time = 0;
                host.remove_tile_object(sq, o);
                return true;
            }
        }
        false
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
        let chance = self.spawn_chance.get(e as usize).copied().unwrap_or(0);
        if chance > 0 && sd.rand(sq.0, sq.1, 101, &mut ctx.rand) < chance {
            let pick = sd.rand(sq.0, sq.1, row.len() as i32, &mut ctx.rand);
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = row[pick as usize] - 1;
            data.max_stage = 2 + (e - 50) / 17 - 1;
            data.stage = 0;
            data.spawn_time = 130 - e;
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
            return Keep::Clear;
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
        self.base
            .update_obj(host, sq, data, obj, true, stage, disp, false)
    }

    pub fn init(&mut self, tables: &ErosionTables, ice: &mut IceQueenSink) {
        self.soil_ref = tables.trees_soil_ref.clone();
        self.trees = tables.trees.clone();
        self.spawn_chance = (0..100)
            .map(|i| {
                if i >= 50 {
                    clerp((i - 50) as f32 / 50.0, 0.0, 90.0) as i32
                } else {
                    0
                }
            })
            .collect();
        let snames = [0, 5, 1, 2, 3, 4];
        self.base.set_disp(5, 0, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 3, true);
        self.base.set_disp(4, 4, 0, true);
        let mut base_sprite = String::new();
        for tree in &self.trees {
            let seasonal = !tree.evergreen;
            let sheet = &tree.tileset_name;
            let mut spr = ErosionObjSprites::new(8, &tree.name, true, false, seasonal);
            for stage in 0..8 {
                for (season, &sname) in snames.iter().enumerate() {
                    let season = season as i32;
                    let sheet_id = match stage {
                        4 => season * 2,
                        5 => season * 2 + 1,
                        6 | 7 => season,
                        _ => season * 4 + stage,
                    };
                    let repl = match stage {
                        4 | 5 => sheet.replace("_1", "JUMBO_1"),
                        6 => sheet.replace("_1", "JUMBOXL_1"),
                        7 => sheet.replace("_1", "JUMBOXXL_1"),
                        _ => sheet.clone(),
                    };
                    let name = format!("{repl}_{sheet_id}");
                    if season == 0 {
                        base_sprite = name;
                        spr.set_base(stage, &base_sprite, 0);
                    } else if season == 1 {
                        ice.push((base_sprite.clone(), name));
                    } else if seasonal {
                        spr.set_child_sprite(stage, &name, sname);
                    }
                }
            }
            self.objs.push(ErosionObj::new(spr, 60, 0.0, 0.0, true));
        }
    }
}
