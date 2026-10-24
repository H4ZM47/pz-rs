//! `NatureGeneric`: grass, generic ground plants and ferns.

use super::{grown_stage, Base, IceQueenSink, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::season::ErosionSeason;

#[derive(Clone, Debug, Default)]
pub struct NatureGeneric {
    pub base: Base,
    pub objs: Vec<ErosionObj>,
}

/// `toInt(char)`.
fn to_int(c: char) -> i32 {
    c.to_digit(10).map_or(0, |d| d as i32)
}

impl NatureGeneric {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace_existing_object(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData) -> bool {
        let objs = host.objects(sq);
        for i in (1..objs.len()).rev() {
            let o = objs[i];
            let Some(mut name) = host.sprite_name(sq, o) else {
                continue;
            };
            // The custom name of the sprite in use: the object's own unless
            // a grass overlay is looked up in the sprite manager.
            let mut custom: Option<String> = None;
            let mut custom_from_map = false;
            if name.starts_with("blends_grassoverlays_01") {
                let idx = host.sprite_tile_sheet_index(sq, o);
                let lookup = if (0..=21).contains(&idx) {
                    Some(24 + idx)
                } else if (22..=45).contains(&idx) {
                    Some(48 + idx - 22)
                } else if (48..=69).contains(&idx) {
                    Some(72 + idx - 48)
                } else {
                    None
                };
                if let Some(n) = lookup {
                    let key = format!("e_newgrass_1_{n}");
                    if let Some(c) = host.named_sprite_custom_name(&key) {
                        name = key;
                        custom = c;
                        custom_from_map = true;
                    }
                }
            }
            if !name.starts_with("e_newgrass_1_") && !name.starts_with("d_generic_1_") {
                continue;
            }
            let Some(gi) = self.objs.iter().position(|ob| ob.get_entry(&name).is_some()) else {
                continue;
            };
            if !custom_from_map {
                custom = host.sprite_custom_name(sq, o);
            }
            if custom.is_some_and(|c| c.contains("Twig") || c.contains("Branch")) {
                continue;
            }
            let obj = &self.objs[gi];
            let entry = obj.get_entry(&name).unwrap();
            let obj_name = obj.name.clone();
            let data = self.base.set_cat_mod_data(sd);
            data.game_obj = gi as i32;
            data.stage = entry.stage;
            data.spawn_time = 0;
            data.disp_season = -1;
            host.set_object_name(sq, o, Some(&obj_name));
            data.has_spawned = true;
            if name.starts_with("e_newgrass_1_") {
                data.max_stage = 2;
            } else {
                data.not_grass = true;
                data.max_stage = 1;
            }
            return true;
        }
        false
    }

    pub fn update(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &SquareData, data: &mut CategoryData, e_tick: i32, season: &ErosionSeason) -> Keep {
        if e_tick < data.spawn_time || data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            data.do_nothing = true;
            return Keep::Keep;
        }
        let ground = Self::ground_grass_type(host, sq);
        let obj = &mut self.objs[data.game_obj as usize];
        let max_stage = data.max_stage;
        let mut stage = grown_stage(e_tick, data.spawn_time, obj.cycle_time, max_stage);
        if stage > max_stage {
            stage = max_stage;
        }
        if stage >= obj.stages {
            stage = obj.stages - 1;
        }
        if data.stage == data.max_stage {
            stage = data.max_stage;
        }
        let mut disp = 0;
        if !data.not_grass {
            disp = self.base.current_season(sd.magic_num, season);
            if ground == 2 {
                disp = disp.max(3);
            } else if ground == 3 {
                disp = disp.max(4);
            }
        }
        self.base.update_obj(host, sq, data, obj, false, stage, disp, false)
    }

    /// `getGroundGrassType(square)`.
    fn ground_grass_type(host: &dyn ErosionHost, sq: Sq) -> i32 {
        let Some(floor) = host.floor(sq) else {
            return 0;
        };
        let Some(name) = host.sprite_name(sq, floor) else {
            return 0;
        };
        if !name.starts_with("blends_natural_01_") {
            return 0;
        }
        let chars: Vec<char> = name.encode_utf16().map(|u| char::from_u32(u as u32).unwrap_or('\0')).collect();
        let mut id: i32 = 0;
        for x in 18..chars.len() {
            id = id.wrapping_add(to_int(chars[x]));
            if x < chars.len() - 1 {
                id = id.wrapping_mul(10);
            }
        }
        let row = id / 8;
        let col = id % 8;
        let edge = col == 0 || col >= 5;
        match row {
            2 if edge => 1,
            4 if edge => 2,
            6 if edge => 3,
            _ => 0,
        }
    }

    pub fn init(&mut self, ice: &mut IceQueenSink) {
        // `spawnChance` is filled upstream but never read: this category
        // only replaces existing objects.
        self.base.set_disp(5, 5, 0, false);
        self.base.set_disp(1, 1, 0, false);
        self.base.set_disp(2, 2, 3, true);
        self.base.set_disp(4, 4, 5, true);
        let snames = [1, 2, 3, 4, 5];
        let stages = [2, 1, 0];
        for grassx in 0..=5 {
            let mut spr = ErosionObjSprites::new(3, "Grass", false, false, false);
            for (season, &sname) in snames.iter().enumerate() {
                for (stage, &st) in stages.iter().enumerate() {
                    let id = season * 24 + stage * 8 + grassx;
                    spr.set_base(st, &format!("e_newgrass_1_{id}"), sname);
                }
            }
            self.objs.push(ErosionObj::new(spr, 60, 0.0, 0.0, false));
        }
        for genericx in 0..=15 {
            let mut spr = ErosionObjSprites::new(2, "Generic", false, false, false);
            for stage in 0..=1 {
                spr.set_base(stage, &format!("d_generic_1_{}", stage * 16 + genericx), 0);
            }
            self.objs.push(ErosionObj::new(spr, 60, 0.0, 0.0, true));
        }
        for fernx in 0..=7 {
            let mut spr = ErosionObjSprites::new(2, "Fern", true, false, false);
            for stage in 0..=1 {
                let id = 48 + stage * 32 + fernx;
                spr.set_base(stage, &format!("d_generic_1_{id}"), 0);
                ice.push((format!("d_generic_1_{id}"), format!("d_generic_1_{}", id + 16)));
            }
            self.objs.push(ErosionObj::new(spr, 60, 0.0, 0.0, true));
        }
    }
}
