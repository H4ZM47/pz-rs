//! `ErosionRegions` and `ErosionWorld`: which categories apply to a square
//! and running them.

use crate::category::{
    Category, Ctx, Flowerbed, IceQueenSink, Keep, NatureBush, NatureGeneric, NaturePlants, NatureTrees, StreetCracks,
    WallCracks, WallVines,
};
use crate::data::{ChunkData, SquareData};
use crate::host::{ErosionHost, Sq, SqFlag};
use crate::tables::ErosionTables;

pub const REGION_NATURE: i32 = 0;
pub const CATEGORY_TREES: i32 = 0;
pub const CATEGORY_BUSH: i32 = 1;
pub const CATEGORY_PLANTS: i32 = 2;
pub const CATEGORY_GENERIC: i32 = 3;
pub const REGION_STREET: i32 = 1;
pub const CATEGORY_STREET_CRACKS: i32 = 0;
pub const REGION_WALL: i32 = 2;
pub const CATEGORY_WALL_VINES: i32 = 0;
pub const CATEGORY_WALL_CRACKS: i32 = 1;
pub const REGION_FLOWERBED: i32 = 3;
pub const CATEGORY_FLOWERBED: i32 = 0;

/// `ErosionRegions.Region`.
#[derive(Clone, Debug)]
pub struct Region {
    pub id: i32,
    pub tile_name_match: Option<&'static str>,
    pub check_exterior: bool,
    pub is_exterior: bool,
    pub has_wall: bool,
    pub categories: Vec<Category>,
}

impl Region {
    fn new(id: i32, tile_match: Option<&'static str>, check_ext: bool, is_ext: bool, has_wall: bool) -> Self {
        Region { id, tile_name_match: tile_match, check_exterior: check_ext, is_exterior: is_ext, has_wall, categories: Vec::new() }
    }

    /// `addCategory(id, cat)`.
    fn add_category(mut self, id: i32, mut cat: Category) -> Self {
        cat.base_mut().id = id;
        cat.base_mut().region_id = self.id;
        self.categories.push(cat);
        self
    }
}

/// `ErosionRegions` (the region list) and `ErosionWorld`.
#[derive(Clone, Debug, Default)]
pub struct ErosionRegions {
    pub regions: Vec<Region>,
}

impl ErosionRegions {
    /// `ErosionRegions.init()`; the ice queen registrations the categories
    /// make are returned in order.
    pub fn init(tables: &ErosionTables) -> (Self, IceQueenSink) {
        let regions = vec![
            Region::new(REGION_NATURE, Some("blends_natural_01"), true, true, false)
                .add_category(CATEGORY_TREES, Category::NatureTrees(NatureTrees::new()))
                .add_category(CATEGORY_BUSH, Category::NatureBush(NatureBush::new()))
                .add_category(CATEGORY_PLANTS, Category::NaturePlants(NaturePlants::new()))
                .add_category(CATEGORY_GENERIC, Category::NatureGeneric(NatureGeneric::new())),
            Region::new(REGION_STREET, Some("blends_street"), true, true, false)
                .add_category(CATEGORY_STREET_CRACKS, Category::StreetCracks(StreetCracks::new())),
            Region::new(REGION_WALL, None, false, false, true)
                .add_category(CATEGORY_WALL_VINES, Category::WallVines(WallVines::new()))
                .add_category(CATEGORY_WALL_CRACKS, Category::WallCracks(WallCracks::new())),
            Region::new(REGION_FLOWERBED, None, true, true, false)
                .add_category(CATEGORY_FLOWERBED, Category::Flowerbed(Flowerbed::new())),
        ];
        let mut r = ErosionRegions { regions };
        for (i, reg) in r.regions.iter_mut().enumerate() {
            reg.id = i as i32;
        }
        let mut ice = Vec::new();
        for reg in &mut r.regions {
            for c in &mut reg.categories {
                c.init(tables, &mut ice);
            }
        }
        (r, ice)
    }

    /// `getCategory(regionId, categoryId)`.
    pub fn get_category(&mut self, region: i32, category: i32) -> Option<&mut Category> {
        self.regions.get_mut(region as usize)?.categories.get_mut(category as usize)
    }

    /// `ErosionWorld.validateSpawn(square, sqErosionData, chunkData)`.
    pub fn validate_spawn(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData, chunk: &ChunkData, ctx: &mut Ctx<'_>) {
        let is_exterior = host.has_flag(sq, SqFlag::Exterior);
        let has_wall = host.has_wall_type(sq);
        let tex = host.floor(sq).and_then(|f| host.sprite_name(sq, f));
        let Some(tex) = tex else {
            sd.do_nothing = true;
            return;
        };
        let mut has_spawned = false;
        for region in &mut self.regions {
            if region.tile_name_match.is_some_and(|m| !tex.starts_with(m))
                || region.check_exterior && region.is_exterior != is_exterior
                || region.has_wall && region.has_wall != has_wall
            {
                continue;
            }
            for cat in &mut region.categories {
                let mut spawned = cat.replace_existing_object(host, sq, sd, chunk, is_exterior, has_wall, ctx);
                if !spawned {
                    spawned = cat.validate_spawn(host, sq, sd, chunk, is_exterior, has_wall, false, ctx);
                }
                if spawned {
                    has_spawned = true;
                    break;
                }
            }
        }
        if !has_spawned {
            sd.do_nothing = true;
        }
    }

    /// `ErosionWorld.update(square, sqErosionData, chunkData, eTick)`.
    pub fn update(&mut self, host: &mut dyn ErosionHost, sq: Sq, sd: &mut SquareData, e_tick: i32, ctx: &Ctx<'_>) {
        let mut i = 0;
        while i < sd.regions.len() {
            let mut data = sd.regions[i].clone();
            let size = sd.regions.len();
            let Some(cat) = self.get_category(data.region_id, data.category_id) else {
                i += 1;
                continue;
            };
            match cat.update(host, sq, sd, &mut data, e_tick, ctx) {
                Keep::Keep => sd.regions[i] = data,
                Keep::Clear => sd.clear_cat_mod_data(data.region_id, data.category_id),
            }
            if size > sd.regions.len() {
                continue;
            }
            i += 1;
        }
    }

    /// `getObjectNames` over every category.
    pub fn get_object_names(&self) -> Vec<String> {
        let mut list = Vec::new();
        for r in &self.regions {
            for c in &r.categories {
                c.get_object_names(&mut list);
            }
        }
        list
    }
}
