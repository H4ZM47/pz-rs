//! `WallVines`: vine overlays on exterior walls, optionally climbing to the
//! level above.

use std::collections::HashMap;

use super::{grown_stage, valid_wall, Base, Ctx, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObjOverlay, ErosionObjOverlaySprites};
use crate::season::ErosionSeason;

#[derive(Clone, Debug, Default)]
pub struct WallVines {
    pub base: Base,
    pub objs: Vec<ErosionObjOverlay>,
    objs_ref: [[i32; 2]; 3],
    sprite_to_obj: HashMap<String, i32>,
    sprite_to_stage: HashMap<String, i32>,
    spawn_chance: Vec<i32>,
}

const PREFIX: &str = "f_wallvines_1_";

/// The overlay target: the north wall when both are valid.
fn wall_target(host: &dyn ErosionHost, sq: Sq) -> Option<u32> {
    let north = valid_wall(host, sq, true, true);
    let west = valid_wall(host, sq, false, true);
    north.or(west)
}

impl WallVines {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_existing_object(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData) -> bool {
        let objs = host.objects(sq);
        for i in (1..objs.len()).rev() {
            let o = objs[i];
            let Some(list) = host.attached(sq, o) else {
                continue;
            };
            for (j, a) in list.iter().enumerate() {
                let Some(name) = a.name.as_deref() else {
                    continue;
                };
                if !name.starts_with(PREFIX) {
                    continue;
                }
                let Some(&obj) = self.sprite_to_obj.get(name) else {
                    continue;
                };
                let stage = self.sprite_to_stage[name];
                let data = self.base.set_cat_mod_data(sd);
                data.game_obj = obj;
                data.stage = stage;
                data.max_stage = 2;
                data.spawn_time = 0;
                host.attached_remove(sq, o, j);
                if host.attached(sq, o).is_some_and(|l| j < l.len()) {
                    host.attached_remove(sq, o, j);
                }
                return true;
            }
        }
        false
    }

    pub fn validate_spawn(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData, is_exterior: bool, ctx: &mut Ctx<'_>) -> bool {
        if !is_exterior {
            return false;
        }
        let e = sd.noise_main_int;
        let chance = self.spawn_chance.get(e as usize).copied().unwrap_or(0);
        if chance == 0 {
            return false;
        }
        if sd.rand(sq.0, sq.1, 101, &mut ctx.rand) >= chance {
            return false;
        }
        let north = valid_wall(host, sq, true, true);
        let west = valid_wall(host, sq, false, true);
        let dir = match (north, west) {
            (Some(_), Some(_)) => 0,
            (Some(_), None) => 1,
            (None, Some(_)) => 2,
            (None, None) => return false,
        };
        let pick = sd.rand(sq.0, sq.1, self.objs_ref[dir].len() as i32, &mut ctx.rand);
        let mut data = sd.set_cat_mod_data(self.base.region_id, self.base.id).clone();
        data.game_obj = self.objs_ref[dir][pick as usize];
        data.max_stage = if e > 65 {
            3
        } else if e > 60 {
            2
        } else if e > 55 {
            1
        } else {
            0
        };
        data.stage = 0;
        data.spawn_time = 100 - e;
        if data.max_stage == 3 {
            let top = (sq.0, sq.1, sq.2 + 1);
            if host.square_exists(top) {
                let obj = valid_wall(host, top, dir == 1, true);
                let overlay = self.objs.get(data.game_obj as usize);
                if let (Some(_), Some(overlay)) = (obj, overlay) {
                    let pick = sd.rand(sq.0, sq.1, self.objs_ref[dir].len() as i32, &mut ctx.rand);
                    let t = CategoryData {
                        game_obj: self.objs_ref[dir][pick as usize],
                        max_stage: if e > 75 {
                            2
                        } else if e > 70 {
                            1
                        } else {
                            0
                        },
                        stage: 0,
                        spawn_time: data.spawn_time
                            + (overlay.cycle_time as f32 / (data.max_stage as f32 + 1.0) * 4.0) as i32,
                        ..Default::default()
                    };
                    data.has_top = Some(Box::new(t));
                } else {
                    data.max_stage = 2;
                }
            } else {
                data.max_stage = 2;
            }
        }
        *sd.set_cat_mod_data(self.base.region_id, self.base.id) = data;
        true
    }

    pub fn update(&mut self, host: &mut dyn ErosionHost, sq: Sq, data: &mut CategoryData, e_tick: i32, season: &ErosionSeason) -> Keep {
        if e_tick < data.spawn_time || data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            data.do_nothing = true;
            return Keep::Keep;
        }
        let overlay = &self.objs[data.game_obj as usize];
        let max_stage = data.max_stage;
        let mut stage = grown_stage(e_tick, data.spawn_time, overlay.cycle_time, max_stage);
        if stage < data.stage {
            stage = data.stage;
        }
        if stage > max_stage {
            stage = max_stage;
        }
        if stage > overlay.stages {
            stage = overlay.stages;
        }
        if stage == 3 && data.has_top.as_ref().is_some_and(|t| t.spawn_time > e_tick) {
            stage = 2;
        }
        let disp = season.get_season();
        if stage != data.stage || data.disp_season != disp {
            let object = wall_target(host, sq);
            data.disp_season = disp;
            match object {
                Some(o) => {
                    let id = overlay.set_overlay(host, sq, o, data.cur_id, stage, disp, 0.0);
                    if id >= 0 {
                        data.cur_id = id;
                    }
                }
                None => data.do_nothing = true,
            }
            let top = (sq.0, sq.1, sq.2 + 1);
            if stage == 3 && data.has_top.is_some() && host.square_exists(top) {
                let mut t = data.has_top.take().unwrap();
                self.update(host, top, &mut t, e_tick, season);
                data.has_top = Some(t);
            }
        }
        Keep::Keep
    }

    pub fn init(&mut self) {
        self.spawn_chance = (0..100).map(|i| if i >= 50 { 100 } else { 0 }).collect();
        self.base.set_disp(5, 5, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 4, true);
        self.base.set_disp(4, 4, 5, true);
        let seasons = [5, 2, 4, 1];
        let dir = [2usize, 2, 1, 1, 0, 0];
        let mut count = [0usize; 3];
        for (i, &d) in dir.iter().enumerate() {
            let mut spr = ErosionObjOverlaySprites::new(4, "WallVines");
            for stage in 0..=3 {
                for season in 0..=2usize {
                    let id = season as i32 * 24 + stage * 6 + i as i32;
                    let name = format!("{PREFIX}{id}");
                    spr.set_sprite(stage, &name, seasons[season]);
                    if season == 2 {
                        spr.set_sprite(stage, &name, seasons[season + 1]);
                    }
                    self.sprite_to_obj.insert(name.clone(), self.objs.len() as i32);
                    self.sprite_to_stage.insert(name, stage);
                }
            }
            self.objs.push(ErosionObjOverlay::new(spr, 60, false));
            self.objs_ref[d][count[d]] = self.objs.len() as i32 - 1;
            count[d] += 1;
        }
    }
}
