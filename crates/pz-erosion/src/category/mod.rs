//! `zombie.erosion.categories`: the shared `ErosionCategory` logic and the
//! eight categories.

mod flowerbed;
mod nature_bush;
mod nature_generic;
mod nature_plants;
mod nature_trees;
mod street_cracks;
mod wall_cracks;
mod wall_vines;

pub use flowerbed::Flowerbed;
pub use nature_bush::NatureBush;
pub use nature_generic::NatureGeneric;
pub use nature_plants::NaturePlants;
pub use nature_trees::NatureTrees;
pub use street_cracks::StreetCracks;
pub use wall_cracks::WallCracks;
pub use wall_vines::WallVines;

use crate::data::{CategoryData, ChunkData, SquareData, SquareRand};
use crate::host::{ErosionHost, ObjId, Sq, SqFlag};
use crate::obj::ErosionObj;
use crate::season::ErosionSeason;
use crate::tables::ErosionTables;

/// What the categories read besides the square: the current season and
/// the random source of `ErosionData.Square.rand`.
pub struct Ctx<'a> {
    pub season: &'a ErosionSeason,
    pub rand: SquareRand<'a>,
}

/// `ErosionCategory.SeasonDisplay`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeasonDisplay {
    pub season1: i32,
    pub season2: i32,
    pub split: bool,
}

/// The fields every category has (`id`, `region`, `seasonDisp`).
#[derive(Clone, Debug, Default)]
pub struct Base {
    pub id: i32,
    pub region_id: i32,
    pub season_disp: [SeasonDisplay; 6],
}

/// Whether `update` left the category's data in place or cleared it
/// (`clearCatModData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Keep,
    Clear,
}

/// `ErosionCategory.clerp(t, a, b)` (float).
pub fn clerp(t: f32, a: f32, b: f32) -> f32 {
    let t2 = (1.0 - libm::cos(t as f64 * std::f64::consts::PI)) as f32 / 2.0;
    a * (1.0 - t2) + b * t2
}

impl Base {
    pub fn set_disp(&mut self, season: usize, s1: i32, s2: i32, split: bool) {
        self.season_disp[season] = SeasonDisplay {
            season1: s1,
            season2: s2,
            split,
        };
    }

    /// `setCatModData(sqErosionData)`.
    pub fn set_cat_mod_data<'s>(&self, sd: &'s mut SquareData) -> &'s mut CategoryData {
        sd.set_cat_mod_data(self.region_id, self.id)
    }

    /// `clearCatModData(sqErosionData)`.
    pub fn clear_cat_mod_data(&self, sd: &mut SquareData) {
        sd.clear_cat_mod_data(self.region_id, self.id);
    }

    /// `currentSeason(magicNum, gameObj)`.
    pub fn current_season(&self, magic_num: f32, seasons: &ErosionSeason) -> i32 {
        let season = seasons.get_season();
        let season_day = seasons.get_season_day();
        let season_days = seasons.get_season_days();
        let half = season_days / 2.0;
        let half_magic = half * magic_num;
        let cur = self.season_disp[season as usize];
        if cur.split && season_day >= half + half_magic {
            cur.season2
        } else if cur.split && season_day >= half_magic || season_day >= season_days * magic_num {
            cur.season1
        } else {
            let prev = match season {
                5 => self.season_disp[4],
                1 => self.season_disp[5],
                2 => self.season_disp[1],
                _ => self.season_disp[2],
            };
            if prev.split {
                prev.season2
            } else {
                prev.season1
            }
        }
    }

    /// `currentBloom(magicNum, gameObj)`.
    pub fn current_bloom(&self, magic_num: f32, obj: &ErosionObj, seasons: &ErosionSeason) -> bool {
        if !(obj.has_flower && seasons.get_season() == 2) {
            return false;
        }
        let season_day = seasons.get_season_day();
        let season_days = seasons.get_season_days();
        let half = season_days / 2.0;
        let half_magic = half * magic_num;
        let total = season_days - half_magic;
        let cur = season_day - half_magic;
        let max = total * obj.bloom_end;
        let min = total * obj.bloom_start;
        let split = (max - min) / 2.0;
        let offset = split * magic_num;
        let max = min + split + offset;
        let min = min + offset;
        cur >= min && cur <= max
    }

    /// `updateObj(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_obj(
        &self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        data: &mut CategoryData,
        obj: &mut ErosionObj,
        tree: bool,
        stage: i32,
        disp_season: i32,
        bloom: bool,
    ) -> Keep {
        if !data.has_spawned {
            if !obj.place_object(host, sq, stage, tree, disp_season, bloom) {
                return Keep::Clear;
            }
            data.has_spawned = true;
        } else if data.stage != stage || data.disp_season != disp_season || data.disp_bloom != bloom
        {
            let Some(o) = obj.get_object(host, sq, false) else {
                return Keep::Clear;
            };
            obj.set_stage_object(host, sq, stage, o, disp_season, bloom);
        }
        data.stage = stage;
        data.disp_season = disp_season;
        data.disp_bloom = bloom;
        Keep::Keep
    }
}

/// `validWall(square, north, doWindow)`.
pub fn valid_wall(host: &dyn ErosionHost, sq: Sq, north: bool, do_window: bool) -> Option<ObjId> {
    if !host.square_exists(sq) {
        return None;
    }
    let other = if north {
        (sq.0, sq.1 - 1, sq.2)
    } else {
        (sq.0 - 1, sq.1, sq.2)
    };
    let mut object = None;
    if host.is_wall_to(sq, other) {
        if north && host.has_flag(sq, SqFlag::CutN) && !host.has_flag(sq, SqFlag::CanPathN)
            || !north && host.has_flag(sq, SqFlag::CutW) && !host.has_flag(sq, SqFlag::CanPathW)
        {
            object = host.wall(sq, north);
        }
    } else if do_window && (host.is_window_blocked_to(sq, other) || host.is_window_to(sq, other)) {
        object = host.window_to(sq, other);
        if object.is_none() {
            object = host.wall(sq, north);
        }
    }
    let o = object?;
    if sq.2 > 0 {
        let name = host.sprite_name(sq, o);
        return match name {
            Some(n) if !n.contains("roof") => Some(o),
            _ => None,
        };
    }
    Some(o)
}

/// `(int)Math.floor((float)(eTick - spawnTime) / ((float)cycleTime / ((float)maxStage + 1.0f)))`.
pub fn grown_stage(e_tick: i32, spawn_time: i32, cycle_time: i32, max_stage: i32) -> i32 {
    (((e_tick - spawn_time) as f32 / (cycle_time as f32 / (max_stage as f32 + 1.0))) as f64).floor()
        as i32
}

/// `Integer.parseInt(name.replace(prefix, ""))`; `None` is the
/// `NumberFormatException` (which aborts the square's erosion update
/// upstream; here the object is skipped).
pub fn parse_suffix(name: &str, prefix: &str) -> Option<i32> {
    let r = crate::java::parse_int(&name.replace(prefix, ""));
    if r.is_none() {
        eprintln!("erosion: NumberFormatException for sprite {name}");
    }
    r
}

/// An erosion category.
#[derive(Clone, Debug)]
pub enum Category {
    NatureTrees(NatureTrees),
    NatureBush(NatureBush),
    NaturePlants(NaturePlants),
    NatureGeneric(NatureGeneric),
    StreetCracks(StreetCracks),
    WallVines(WallVines),
    WallCracks(WallCracks),
    Flowerbed(Flowerbed),
}

macro_rules! each {
    ($self:expr, $c:ident => $e:expr) => {
        match $self {
            Category::NatureTrees($c) => $e,
            Category::NatureBush($c) => $e,
            Category::NaturePlants($c) => $e,
            Category::NatureGeneric($c) => $e,
            Category::StreetCracks($c) => $e,
            Category::WallVines($c) => $e,
            Category::WallCracks($c) => $e,
            Category::Flowerbed($c) => $e,
        }
    };
}

/// Ice queen registrations a category's `init` makes
/// (`ErosionIceQueen.addSprite(sprite, winterSprite)`), in order.
pub type IceQueenSink = Vec<(String, String)>;

impl Category {
    pub fn base(&self) -> &Base {
        each!(self, c => &c.base)
    }

    pub fn base_mut(&mut self) -> &mut Base {
        each!(self, c => &mut c.base)
    }

    /// `init()`.
    pub fn init(&mut self, tables: &ErosionTables, ice: &mut IceQueenSink) {
        match self {
            Category::NatureTrees(c) => c.init(tables, ice),
            Category::NatureBush(c) => c.init(tables, ice),
            Category::NaturePlants(c) => c.init(tables),
            Category::NatureGeneric(c) => c.init(ice),
            Category::StreetCracks(c) => c.init(),
            Category::WallVines(c) => c.init(),
            Category::WallCracks(c) => c.init(),
            Category::Flowerbed(c) => c.init(tables),
        }
    }

    /// `replaceExistingObject(square, sqErosionData, chunkData, isExterior, hasWall)`.
    #[allow(clippy::too_many_arguments)]
    pub fn replace_existing_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        _chunk: &ChunkData,
        _is_exterior: bool,
        _has_wall: bool,
        ctx: &mut Ctx<'_>,
    ) -> bool {
        match self {
            Category::NatureTrees(c) => c.replace_existing_object(host, sq, sd, ctx),
            Category::NatureBush(c) => c.replace_existing_object(host, sq, sd, ctx),
            Category::NaturePlants(c) => c.replace_existing_object(host, sq, sd, ctx),
            Category::NatureGeneric(c) => c.replace_existing_object(host, sq, sd),
            Category::WallVines(c) => c.replace_existing_object(host, sq, sd),
            Category::Flowerbed(c) => c.replace_existing_object(host, sq, sd),
            Category::StreetCracks(_) | Category::WallCracks(_) => false,
        }
    }

    /// `validateSpawn(square, sqErosionData, chunkData, isExterior, hasWall, isRespawn)`.
    #[allow(clippy::too_many_arguments)]
    pub fn validate_spawn(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &mut SquareData,
        _chunk: &ChunkData,
        is_exterior: bool,
        has_wall: bool,
        _is_respawn: bool,
        ctx: &mut Ctx<'_>,
    ) -> bool {
        match self {
            Category::NatureTrees(c) => c.validate_spawn(host, sq, sd, has_wall, ctx),
            Category::NatureBush(c) => c.validate_spawn(host, sq, sd, has_wall, ctx),
            Category::NaturePlants(c) => c.validate_spawn(host, sq, sd, has_wall, ctx),
            Category::StreetCracks(c) => c.validate_spawn(sq, sd, ctx),
            Category::WallVines(c) => c.validate_spawn(host, sq, sd, is_exterior, ctx),
            Category::WallCracks(c) => c.validate_spawn(host, sq, sd, is_exterior, ctx),
            Category::NatureGeneric(_) | Category::Flowerbed(_) => false,
        }
    }

    /// `update(square, sqErosionData, data, chunkData, eTick)`.
    pub fn update(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        sd: &SquareData,
        data: &mut CategoryData,
        e_tick: i32,
        ctx: &Ctx<'_>,
    ) -> Keep {
        let season = ctx.season;
        match self {
            Category::NatureTrees(c) => c.update(host, sq, sd, data, e_tick, season),
            Category::NatureBush(c) => c.update(host, sq, sd, data, e_tick, season),
            Category::NaturePlants(c) => c.update(host, sq, sd, data, e_tick, season),
            Category::NatureGeneric(c) => c.update(host, sq, sd, data, e_tick, season),
            Category::StreetCracks(c) => c.update(host, sq, sd, data, e_tick, season),
            Category::WallVines(c) => c.update(host, sq, data, e_tick, season),
            Category::WallCracks(c) => c.update(host, sq, data, e_tick),
            Category::Flowerbed(c) => c.update(host, sq, data, season),
        }
    }

    /// The category's `objs[i]` when they are `ErosionObj`s (not overlays).
    pub fn erosion_obj(&self, i: usize) -> Option<&ErosionObj> {
        match self {
            Category::NatureTrees(c) => c.objs.get(i),
            Category::NatureBush(c) => c.objs.get(i),
            Category::NaturePlants(c) => c.objs.get(i),
            Category::NatureGeneric(c) => c.objs.get(i),
            Category::StreetCracks(c) => c.objs.get(i),
            Category::Flowerbed(c) => c.objs.get(i),
            Category::WallVines(_) | Category::WallCracks(_) => None,
        }
    }

    /// `getObjectNames(list)`.
    pub fn get_object_names(&self, list: &mut Vec<String>) {
        let names: Vec<String> = match self {
            Category::NatureTrees(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::NatureBush(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::NaturePlants(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::NatureGeneric(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::StreetCracks(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::WallVines(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::WallCracks(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
            Category::Flowerbed(c) => c.objs.iter().map(|o| o.name.clone()).collect(),
        };
        for n in names {
            if !list.contains(&n) {
                list.push(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clerp_ends() {
        assert_eq!(clerp(0.0, 0.0, 90.0), 0.0);
        assert!((clerp(1.0, 0.0, 90.0) - 90.0).abs() < 1e-4);
        assert_eq!(grown_stage(70, 10, 60, 2), 3);
        assert_eq!(grown_stage(10, 10, 60, 2), 0);
    }
}
