//! `Flowerbed`: seasonal sprites of existing flowerbeds.

use super::{parse_suffix, Base, Keep};
use crate::data::{CategoryData, SquareData};
use crate::host::{ErosionHost, Sq};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::season::ErosionSeason;
use crate::tables::ErosionTables;

#[derive(Clone, Debug, Default)]
pub struct Flowerbed {
    pub base: Base,
    tile_id: Vec<i32>,
    pub objs: Vec<ErosionObj>,
}

const ORNAMENTAL: &str = "vegetation_ornamental_01_";

impl Flowerbed {
    pub fn new() -> Self {
        Self::default()
    }

    fn adopt(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        o: u32,
        sd: &mut SquareData,
        game_obj: i32,
    ) -> bool {
        let Some(name) = self.objs.get(game_obj as usize).map(|ob| ob.name.clone()) else {
            // Out of range: an IndexOutOfBoundsException upstream.
            return false;
        };
        let data = self.base.set_cat_mod_data(sd);
        data.has_spawned = true;
        data.game_obj = game_obj;
        data.disp_season = -1;
        host.set_object_name(sq, o, Some(&name));
        true
    }

    pub fn replace_existing_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
    ) -> bool {
        let objs = host.objects(sq);
        for i in (0..objs.len()).rev() {
            let o = objs[i];
            let Some(name) = host.sprite_name(sq, o) else {
                continue;
            };
            if name.starts_with("f_flowerbed_1") {
                let Some(mut id) = parse_suffix(&name, "f_flowerbed_1_") else {
                    continue;
                };
                if id <= 23 {
                    if id >= 12 {
                        id -= 12;
                    }
                    return self.adopt(host, sq, o, sd, id);
                }
            }
            if !name.starts_with("vegetation_ornamental_01") {
                continue;
            }
            let Some(id) = parse_suffix(&name, ORNAMENTAL) else {
                continue;
            };
            if let Some(j) = self.tile_id.iter().position(|&t| t == id) {
                return self.adopt(host, sq, o, sd, j as i32);
            }
        }
        false
    }

    pub fn update(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        data: &mut CategoryData,
        season: &ErosionSeason,
    ) -> Keep {
        if data.do_nothing {
            return Keep::Keep;
        }
        if data.game_obj < 0 || data.game_obj >= self.objs.len() as i32 {
            return Keep::Clear;
        }
        let obj = &mut self.objs[data.game_obj as usize];
        let disp = season.get_season();
        if disp == 5 {
            if let Some(o) = obj.get_object(host, sq, false) {
                host.set_object_sprite(sq, o, "blends_natural_01_64");
                host.set_object_name(sq, o, None);
            }
            return Keep::Clear;
        }
        self.base
            .update_obj(host, sq, data, obj, false, 0, disp, false)
    }

    pub fn init(&mut self, tables: &ErosionTables) {
        self.tile_id = tables.flowerbed_tile_ids.clone();
        for &t in &self.tile_id {
            let mut spr = ErosionObjSprites::new(1, "Flowerbed", false, false, false);
            spr.set_base(0, &format!("{ORNAMENTAL}{t}"), 1);
            spr.set_base(0, &format!("{ORNAMENTAL}{t}"), 2);
            spr.set_base(0, &format!("{ORNAMENTAL}{}", t + 16), 4);
            self.objs.push(ErosionObj::new(spr, 30, 0.0, 0.0, false));
        }
    }
}
