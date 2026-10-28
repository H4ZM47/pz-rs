//! `StreetCracks`: crack overlays on street floors, with grass in them.

use super::{clerp, grown_stage, Base, Ctx, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjOverlay, ErosionObjOverlaySprites, ErosionObjSprites};
use crate::season::ErosionSeason;

#[derive(Clone, Debug, Default)]
pub struct StreetCracks {
    pub base: Base,
    pub objs: Vec<ErosionObj>,
    crack_objs: Vec<ErosionObjOverlay>,
    spawn_chance: Vec<i32>,
}

impl StreetCracks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate_spawn(&mut self, sq: Sq, sd: &mut SquareData, ctx: &mut Ctx<'_>) -> bool {
        let e = sd.noise_main_int;
        let chance = self.spawn_chance.get(e as usize).copied().unwrap_or(0);
        if chance == 0 {
            return false;
        }
        if sd.rand(sq.0, sq.1, 101, &mut ctx.rand) >= chance {
            return false;
        }
        let pick = sd.rand(sq.0, sq.1, self.crack_objs.len() as i32, &mut ctx.rand);
        let magic = sd.magic_num;
        let data = self.base.set_cat_mod_data(sd);
        data.game_obj = pick;
        data.max_stage = if e > 65 {
            2
        } else if e > 55 {
            1
        } else {
            0
        };
        data.stage = 0;
        data.spawn_time = 150 - e;
        if magic > 0.5 {
            data.has_grass = true;
        }
        true
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
        let floor = host.floor(sq);
        let (Some(floor), true) = (
            floor,
            data.game_obj >= 0 && data.game_obj < self.crack_objs.len() as i32,
        ) else {
            data.do_nothing = true;
            return Keep::Keep;
        };
        let crack = &self.crack_objs[data.game_obj as usize];
        let mut stage = grown_stage(e_tick, data.spawn_time, crack.cycle_time, data.max_stage);
        if stage < data.stage {
            stage = data.stage;
        }
        if stage >= crack.stages {
            stage = crack.stages - 1;
        }
        if stage != data.stage {
            let id = crack.set_overlay(host, sq, floor, data.cur_id, stage, 0, 0.0);
            if id >= 0 {
                data.cur_id = id;
            }
            data.stage = stage;
        } else if !data.has_grass && stage == crack.stages - 1 {
            data.do_nothing = true;
        }
        if data.has_grass {
            if let Some(grass) = self.objs.get_mut(data.game_obj as usize) {
                let disp = self.base.current_season(sd.magic_num, season);
                return self
                    .base
                    .update_obj(host, sq, data, grass, false, stage, disp, false);
            }
        }
        Keep::Keep
    }

    pub fn init(&mut self) {
        self.spawn_chance = (0..100)
            .map(|i| {
                if i >= 40 {
                    clerp((i - 40) as f32 / 60.0, 0.0, 60.0) as i32
                } else {
                    0
                }
            })
            .collect();
        self.base.set_disp(5, 5, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 4, true);
        self.base.set_disp(4, 4, 5, true);
        let seasons = [5, 1, 2, 4];
        for i in 0..=7 {
            let mut crack = ErosionObjOverlaySprites::new(3, "StreeCracks");
            let mut grass = ErosionObjSprites::new(3, "CrackGrass", false, false, false);
            for stage in 0..=2 {
                for season in 0..=seasons.len() {
                    let id = season as i32 * 24 + stage * 8 + i;
                    let name = format!("d_streetcracks_1_{id}");
                    if season == 0 {
                        crack.set_sprite(stage, &name, 0);
                    } else {
                        grass.set_base(stage, &name, seasons[season - 1]);
                    }
                }
            }
            self.crack_objs
                .push(ErosionObjOverlay::new(crack, 60, false));
            self.objs.push(ErosionObj::new(grass, 60, 0.0, 0.0, false));
        }
    }
}
