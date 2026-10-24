//! `WallCracks`: crack overlays fading in on exterior walls.

use super::{valid_wall, Base, Ctx, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObjOverlay, ErosionObjOverlaySprites};

#[derive(Clone, Debug, Default)]
pub struct WallCracks {
    pub base: Base,
    pub objs: Vec<ErosionObjOverlay>,
    objs_ref: [Vec<i32>; 3],
    bot_ref: [Vec<i32>; 3],
    top_ref: [Vec<i32>; 3],
    spawn_chance: Vec<i32>,
}

/// `validWall(square, north, false)`, ignoring fences.
fn crack_wall(host: &dyn ErosionHost, sq: Sq, north: bool) -> Option<u32> {
    let o = valid_wall(host, sq, north, false)?;
    match host.sprite_name(sq, o) {
        Some(n) if n.starts_with("fencing") => None,
        _ => Some(o),
    }
}

impl WallCracks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate_spawn(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        is_exterior: bool,
        ctx: &mut Ctx<'_>,
    ) -> bool {
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
        let north = crack_wall(host, sq, true);
        let west = crack_wall(host, sq, false);
        let dir = match (north, west) {
            (Some(_), Some(_)) => 0,
            (Some(_), None) => 1,
            (None, Some(_)) => 2,
            (None, None) => return false,
        };
        let do_top = e < 35 && sd.magic_num > 0.3;
        let pick = sd.rand(sq.0, sq.1, self.objs_ref[dir].len() as i32, &mut ctx.rand);
        let mut data = sd
            .set_cat_mod_data(self.base.region_id, self.base.id)
            .clone();
        data.game_obj = self.objs_ref[dir][pick as usize];
        data.alpha = 0.0;
        data.spawn_time = e;
        let top = (sq.0, sq.1, sq.2 + 1);
        if do_top && host.square_exists(top) && valid_wall(host, top, dir == 1, false).is_some() {
            let refid = sd.rand(sq.0, sq.1, self.bot_ref[dir].len() as i32, &mut ctx.rand) as usize;
            data.game_obj = self.bot_ref[dir][refid];
            data.has_top = Some(Box::new(CategoryData {
                game_obj: self.top_ref[dir][refid],
                alpha: 0.0,
                spawn_time: data.spawn_time,
                ..Default::default()
            }));
        }
        *sd.set_cat_mod_data(self.base.region_id, self.base.id) = data;
        true
    }

    pub fn update(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        data: &mut CategoryData,
        e_tick: i32,
    ) -> Keep {
        if e_tick < data.spawn_time || data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            data.do_nothing = true;
            return Keep::Keep;
        }
        let overlay = &self.objs[data.game_obj as usize];
        let old_alpha = data.alpha;
        let alpha = ((e_tick - data.spawn_time) as f32 / 100.0).clamp(0.0, 1.0);
        if alpha != old_alpha {
            let north = valid_wall(host, sq, true, false);
            let west = valid_wall(host, sq, false, false);
            match north.or(west) {
                Some(o) => {
                    let id = overlay.set_overlay(host, sq, o, data.cur_id, 0, 0, alpha);
                    if id >= 0 {
                        data.alpha = alpha;
                        data.cur_id = id;
                    }
                }
                None => data.do_nothing = true,
            }
            let top = (sq.0, sq.1, sq.2 + 1);
            if data.has_top.is_some() && host.square_exists(top) {
                let mut t = data.has_top.take().unwrap();
                self.update(host, top, &mut t, e_tick);
                data.has_top = Some(t);
            }
        }
        Keep::Keep
    }

    pub fn init(&mut self) {
        self.spawn_chance = (0..100).map(|i| if i <= 50 { 100 } else { 0 }).collect();
        let dir = [2usize, 2, 2, 1, 1, 1, 0, 0, 0];
        for (i, &d) in dir.iter().enumerate() {
            for row in 0..=7 {
                let id = row * 9 + i;
                let mut spr = ErosionObjOverlaySprites::new(1, "WallCracks");
                spr.set_sprite(0, &format!("d_wallcracks_1_{id}"), 0);
                self.objs.push(ErosionObjOverlay::new(spr, 60, true));
                let idx = self.objs.len() as i32 - 1;
                self.objs_ref[d].push(idx);
                if row == 0 {
                    self.bot_ref[d].push(idx);
                } else if row == 1 {
                    self.top_ref[d].push(idx);
                }
            }
        }
    }
}
