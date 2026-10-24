//! In-memory erosion state (`ErosionData.Chunk`, `ErosionData.Square`,
//! `ErosionCategory.Data` and its subclasses) and its conversion to and
//! from the saved records of `pz_world::erosion_data`.
//!
//! The original keeps fields that are not saved (the unquantized
//! `noiseMain`, `noiseKudzu`, the square's `RandLocation`, the stage of a
//! wall overlay's top part); [`SquareData`] carries them while the square
//! stays loaded. Converting from a saved record recomputes them the way
//! `ErosionData.Square.load` does.

use pz_world::erosion_data as saved;
use pz_world::erosion_data::{ErosionExtra, ErosionTop};

use crate::bits::unpack_byte_to_float_unit;
use crate::java::JavaRandom;

/// `ErosionData.Chunk`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkData {
    pub init: bool,
    pub e_tick_stamp: i32,
    pub epoch: i32,
    pub x: i32,
    pub y: i32,
    pub moisture: f32,
    pub minerals: f32,
    pub soil: i32,
}

impl Default for ChunkData {
    fn default() -> Self {
        ChunkData { init: false, e_tick_stamp: -1, epoch: -1, x: 0, y: 0, moisture: 0.0, minerals: 0.0, soil: 0 }
    }
}

impl ChunkData {
    /// `set(chunk)`.
    pub fn set(&mut self, wx: i32, wy: i32) {
        self.x = wx;
        self.y = wy;
    }

    /// From a chunk's saved record (`load` followed by `set(this)`); a
    /// chunk without one gets `new ErosionData.Chunk()`.
    pub fn from_saved(s: Option<&saved::ErosionChunkData>, wx: i32, wy: i32) -> Self {
        let mut c = ChunkData::default();
        if let Some(s) = s {
            c.init = s.init;
            if s.init {
                c.e_tick_stamp = s.e_tick_stamp;
                c.epoch = s.epoch;
                c.moisture = s.moisture;
                c.minerals = s.minerals;
                c.soil = s.soil as i32;
            }
        }
        c.set(wx, wy);
        c
    }

    pub fn to_saved(&self) -> saved::ErosionChunkData {
        saved::ErosionChunkData {
            init: self.init,
            e_tick_stamp: self.e_tick_stamp,
            epoch: self.epoch,
            moisture: self.moisture,
            minerals: self.minerals,
            soil: self.soil as i8,
        }
    }
}

/// `ErosionCategory.Data` plus the fields of every category's
/// `CategoryData` subclass.
#[derive(Clone, Debug, PartialEq)]
pub struct CategoryData {
    pub region_id: i32,
    pub category_id: i32,
    pub do_nothing: bool,
    pub has_spawned: bool,
    pub stage: i32,
    pub disp_season: i32,
    pub disp_bloom: bool,
    pub game_obj: i32,
    pub max_stage: i32,
    pub spawn_time: i32,
    /// `curId`; `-999999` initially.
    pub cur_id: i32,
    pub has_grass: bool,
    pub not_grass: bool,
    pub alpha: f32,
    /// `hasTop` (wall vines, wall cracks).
    pub has_top: Option<Box<CategoryData>>,
}

impl Default for CategoryData {
    fn default() -> Self {
        CategoryData {
            region_id: 0,
            category_id: 0,
            do_nothing: false,
            has_spawned: false,
            stage: 0,
            disp_season: 0,
            disp_bloom: false,
            game_obj: 0,
            max_stage: 0,
            spawn_time: 0,
            cur_id: -999_999,
            has_grass: false,
            not_grass: false,
            alpha: 0.0,
            has_top: None,
        }
    }
}

fn top_from(t: &ErosionTop) -> Box<CategoryData> {
    Box::new(CategoryData {
        game_obj: t.game_obj as i32,
        spawn_time: t.spawn_time as i32,
        cur_id: t.cur_id,
        alpha: t.alpha,
        ..Default::default()
    })
}

fn top_to(t: &CategoryData) -> ErosionTop {
    ErosionTop { game_obj: t.game_obj as i8, spawn_time: t.spawn_time as i16, cur_id: t.cur_id, alpha: t.alpha }
}

impl CategoryData {
    /// `allocData()` with the ids set.
    pub fn new(region_id: i32, category_id: i32) -> Self {
        CategoryData { region_id, category_id, ..Default::default() }
    }

    pub fn from_saved(s: &saved::ErosionCategoryData) -> Self {
        let mut d = CategoryData::new(s.region_id as i32, s.category_id as i32);
        d.do_nothing = s.do_nothing;
        d.has_spawned = s.has_spawned;
        d.stage = s.stage;
        d.disp_season = s.disp_season as i32;
        d.disp_bloom = s.disp_bloom;
        match &s.extra {
            ErosionExtra::Grown { game_obj, max_stage, spawn_time } => {
                d.game_obj = *game_obj as i32;
                d.max_stage = *max_stage as i32;
                d.spawn_time = *spawn_time as i32;
            }
            ErosionExtra::NatureGeneric { game_obj, max_stage, spawn_time, not_grass } => {
                d.game_obj = *game_obj as i32;
                d.max_stage = *max_stage as i32;
                d.spawn_time = *spawn_time as i32;
                d.not_grass = *not_grass;
            }
            ErosionExtra::NaturePlants { game_obj, spawn_time } => {
                d.game_obj = *game_obj as i32;
                d.spawn_time = *spawn_time as i32;
            }
            ErosionExtra::StreetCracks { game_obj, max_stage, spawn_time, cur_id, has_grass } => {
                d.game_obj = *game_obj as i32;
                d.max_stage = *max_stage as i32;
                d.spawn_time = *spawn_time as i32;
                d.cur_id = *cur_id;
                d.has_grass = *has_grass;
            }
            ErosionExtra::WallVines { game_obj, max_stage, spawn_time, cur_id, top } => {
                d.game_obj = *game_obj as i32;
                d.max_stage = *max_stage as i32;
                d.spawn_time = *spawn_time as i32;
                d.cur_id = *cur_id;
                d.has_top = top.as_ref().map(top_from);
            }
            ErosionExtra::WallCracks { game_obj, spawn_time, cur_id, alpha, top } => {
                d.game_obj = *game_obj as i32;
                d.spawn_time = *spawn_time as i32;
                d.cur_id = *cur_id;
                d.alpha = *alpha;
                d.has_top = top.as_ref().map(top_from);
            }
            ErosionExtra::Flowerbed { game_obj } => d.game_obj = *game_obj as i32,
        }
        d
    }

    /// The saved record; `None` for ids no region defines (the original's
    /// `getCategory` would have thrown before such data existed).
    pub fn to_saved(&self) -> Option<saved::ErosionCategoryData> {
        let (g, m, s) = (self.game_obj as i8, self.max_stage as i8, self.spawn_time as i16);
        let extra = match (self.region_id, self.category_id) {
            (0, 0) | (0, 1) => ErosionExtra::Grown { game_obj: g, max_stage: m, spawn_time: s },
            (0, 2) => ErosionExtra::NaturePlants { game_obj: g, spawn_time: s },
            (0, 3) => ErosionExtra::NatureGeneric { game_obj: g, max_stage: m, spawn_time: s, not_grass: self.not_grass },
            (1, 0) => ErosionExtra::StreetCracks {
                game_obj: g,
                max_stage: m,
                spawn_time: s,
                cur_id: self.cur_id,
                has_grass: self.has_grass,
            },
            (2, 0) => ErosionExtra::WallVines {
                game_obj: g,
                max_stage: m,
                spawn_time: s,
                cur_id: self.cur_id,
                top: self.has_top.as_deref().map(top_to),
            },
            (2, 1) => ErosionExtra::WallCracks {
                game_obj: g,
                spawn_time: s,
                cur_id: self.cur_id,
                alpha: self.alpha,
                top: self.has_top.as_deref().map(top_to),
            },
            (3, 0) => ErosionExtra::Flowerbed { game_obj: g },
            _ => return None,
        };
        Some(saved::ErosionCategoryData {
            region_id: self.region_id as i8,
            category_id: self.category_id as i8,
            do_nothing: self.do_nothing,
            has_spawned: self.has_spawned,
            stage: self.stage,
            disp_season: self.disp_season as i8,
            disp_bloom: self.disp_bloom,
            extra,
        })
    }
}

/// `ErosionData.Square`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SquareData {
    pub init: bool,
    pub do_nothing: bool,
    pub noise_main_byte: i8,
    pub noise_main: f32,
    pub noise_main_int: i32,
    pub noise_kudzu: f32,
    pub soil: i32,
    /// `rand`: the square's `RandLocation`; `None` after a load.
    pub rand: Option<JavaRandom>,
    pub magic_num_byte: i8,
    pub magic_num: f32,
    pub regions: Vec<CategoryData>,
}

impl SquareData {
    /// `reset()`.
    pub fn reset(&mut self) {
        self.init = false;
        self.do_nothing = false;
        self.regions.clear();
    }

    /// From a square's saved record (`ErosionData.Square.load`).
    pub fn from_saved(s: &saved::ErosionSquareData) -> Self {
        let mut d = SquareData { init: s.init, ..Default::default() };
        if s.init {
            d.do_nothing = s.do_nothing;
            d.noise_main_byte = s.noise_main_byte;
            d.noise_main = unpack_byte_to_float_unit(s.noise_main_byte);
            d.noise_main_int = (d.noise_main * 100.0).floor() as i32;
            d.soil = s.soil as i32;
            d.magic_num_byte = s.magic_num_byte;
            d.magic_num = unpack_byte_to_float_unit(s.magic_num_byte);
            d.regions = s.regions.iter().map(CategoryData::from_saved).collect();
        }
        d
    }

    pub fn to_saved(&self) -> saved::ErosionSquareData {
        saved::ErosionSquareData {
            init: self.init,
            do_nothing: self.do_nothing,
            noise_main_byte: self.noise_main_byte,
            soil: self.soil as i8,
            magic_num_byte: self.magic_num_byte,
            regions: self.regions.iter().filter_map(CategoryData::to_saved).collect(),
        }
    }

    /// `getCatModData`: index of this category's data.
    pub fn cat_index(&self, region: i32, category: i32) -> Option<usize> {
        self.regions.iter().position(|d| d.region_id == region && d.category_id == category)
    }

    /// `setCatModData`: this category's data, added if missing.
    pub fn set_cat_mod_data(&mut self, region: i32, category: i32) -> &mut CategoryData {
        let i = match self.cat_index(region, category) {
            Some(i) => i,
            None => {
                self.regions.push(CategoryData::new(region, category));
                if self.regions.len() > 5 {
                    eprintln!("> 5 regions on a square");
                }
                self.regions.len() - 1
            }
        };
        &mut self.regions[i]
    }

    /// `clearCatModData`.
    pub fn clear_cat_mod_data(&mut self, region: i32, category: i32) {
        if let Some(i) = self.cat_index(region, category) {
            self.regions.remove(i);
        }
    }
}

/// Where `ErosionData.Square.rand` draws from.
pub struct SquareRand<'a> {
    /// `staticRand || GameServer.server || GameClient.client`.
    pub use_static: bool,
    /// `ErosionData.Square.rands`.
    pub table: &'a [i32],
    /// `Rand` (the global generator) for squares without a `RandLocation`.
    pub global: &'a mut dyn pz_world::GameRand,
}

impl SquareData {
    /// `rand(x, y, max)`.
    pub fn rand(&mut self, x: i32, y: i32, max: i32, r: &mut SquareRand<'_>) -> i32 {
        if r.use_static {
            let x = x.wrapping_abs();
            let y = y.wrapping_abs();
            let idx = (x % 90 + y % 90 * 90) as usize;
            let rand = r.table[idx] as f32 / 100.0;
            return ((rand * max as f32) as i32).min(max - 1);
        }
        match &mut self.rand {
            Some(rl) => {
                if max <= 0 {
                    0
                } else {
                    rl.next_int_bound(max)
                }
            }
            None => r.global.next(max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_round_trip() {
        let mut vines = CategoryData::new(2, 0);
        vines.stage = 6;
        vines.cur_id = 12;
        vines.has_top = Some(Box::new(CategoryData { game_obj: 2, spawn_time: 9, cur_id: 4, ..Default::default() }));
        let sq = SquareData {
            init: true,
            noise_main_byte: 10,
            noise_main: unpack_byte_to_float_unit(10),
            noise_main_int: (unpack_byte_to_float_unit(10) * 100.0).floor() as i32,
            soil: 3,
            magic_num_byte: -2,
            magic_num: unpack_byte_to_float_unit(-2),
            regions: vec![vines, CategoryData::new(0, 3), CategoryData::new(3, 0)],
            ..Default::default()
        };
        assert_eq!(SquareData::from_saved(&sq.to_saved()), sq);
        let c = ChunkData { init: true, soil: -1, x: 3, y: 4, ..Default::default() };
        assert_eq!(ChunkData::from_saved(Some(&c.to_saved()), 3, 4), c);
        assert_eq!(ChunkData::from_saved(None, 0, 0).e_tick_stamp, -1);
    }

    #[test]
    fn static_rand() {
        struct NoRand;
        impl pz_world::GameRand for NoRand {
            fn next(&mut self, _: i32) -> i32 {
                unreachable!()
            }
        }
        let table: Vec<i32> = (0..8100).map(|i| i % 100).collect();
        let mut g = NoRand;
        let mut r = SquareRand { use_static: true, table: &table, global: &mut g };
        let mut sq = SquareData::default();
        assert_eq!(sq.rand(-1, 0, 100, &mut r), 1);
        assert_eq!(sq.rand(0, 1, 10, &mut r), 9);
    }
}
