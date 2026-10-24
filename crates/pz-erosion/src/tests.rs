//! Category behaviour against an in-memory host (synthetic data only).

use std::collections::HashMap;
use std::sync::Arc;

use crate::category::{Category, Ctx, Keep};
use crate::data::{ChunkData, SquareData, SquareRand};
use crate::host::{Attached, ErosionHost, ObjId, Sq, SqFlag};
use crate::obj::{ErosionObj, ErosionObjSprites};
use crate::regions::ErosionRegions;
use crate::season::ErosionSeason;
use crate::tables::{ErosionTables, TreeInit};

#[derive(Clone, Debug, Default)]
struct Obj {
    id: ObjId,
    sprite: String,
    name: Option<String>,
    attached: Option<Vec<String>>,
}

/// Squares with walls to the north and west on every square.
#[derive(Default)]
struct Mock {
    squares: HashMap<Sq, Vec<Obj>>,
    next: ObjId,
    walls: bool,
    sprite_ids: HashMap<String, i32>,
}

impl Mock {
    fn add(&mut self, sq: Sq, sprite: &str) -> ObjId {
        self.next += 1;
        let id = self.next;
        self.squares.entry(sq).or_default().push(Obj { id, sprite: sprite.into(), ..Default::default() });
        id
    }
    fn obj(&self, sq: Sq, o: ObjId) -> Option<&Obj> {
        self.squares.get(&sq)?.iter().find(|x| x.id == o)
    }
    fn obj_mut(&mut self, sq: Sq, o: ObjId) -> Option<&mut Obj> {
        self.squares.get_mut(&sq)?.iter_mut().find(|x| x.id == o)
    }
    fn id_of(&mut self, name: &str) -> i32 {
        let n = self.sprite_ids.len() as i32;
        *self.sprite_ids.entry(name.to_string()).or_insert(1000 + n)
    }
}

impl ErosionHost for Mock {
    fn square_exists(&self, sq: Sq) -> bool {
        self.squares.contains_key(&sq)
    }
    fn has_flag(&self, _: Sq, f: SqFlag) -> bool {
        matches!(f, SqFlag::Exterior | SqFlag::CutN | SqFlag::CutW)
    }
    fn has_wall_type(&self, _: Sq) -> bool {
        self.walls
    }
    fn objects(&self, sq: Sq) -> Vec<ObjId> {
        self.squares.get(&sq).map_or_else(Vec::new, |v| v.iter().map(|o| o.id).collect())
    }
    fn floor(&self, sq: Sq) -> Option<ObjId> {
        self.squares.get(&sq)?.first().map(|o| o.id)
    }
    fn wall(&self, sq: Sq, north: bool) -> Option<ObjId> {
        let want = if north { "wall_n" } else { "wall_w" };
        self.squares.get(&sq)?.iter().find(|o| o.sprite == want).map(|o| o.id)
    }
    fn is_wall_to(&self, _: Sq, _: Sq) -> bool {
        self.walls
    }
    fn is_window_to(&self, _: Sq, _: Sq) -> bool {
        false
    }
    fn is_window_blocked_to(&self, _: Sq, _: Sq) -> bool {
        false
    }
    fn window_to(&self, _: Sq, _: Sq) -> Option<ObjId> {
        None
    }
    fn sprite_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        self.obj(sq, o).map(|x| x.sprite.clone())
    }
    fn sprite_tile_sheet_index(&self, _: Sq, _: ObjId) -> i32 {
        0
    }
    fn sprite_custom_name(&self, _: Sq, _: ObjId) -> Option<String> {
        None
    }
    fn named_sprite_custom_name(&self, _: &str) -> Option<Option<String>> {
        None
    }
    fn object_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        self.obj(sq, o)?.name.clone()
    }
    fn set_object_name(&mut self, sq: Sq, o: ObjId, name: Option<&str>) {
        if let Some(x) = self.obj_mut(sq, o) {
            x.name = name.map(str::to_string);
        }
    }
    fn set_object_sprite(&mut self, sq: Sq, o: ObjId, sprite: &str) {
        if let Some(x) = self.obj_mut(sq, o) {
            x.sprite = sprite.to_string();
        }
    }
    fn attached(&self, sq: Sq, o: ObjId) -> Option<Vec<Attached>> {
        let list = self.obj(sq, o)?.attached.as_ref()?;
        Some(list.iter().map(|n| Attached { name: Some(n.clone()), id: self.sprite_ids[n] }).collect())
    }
    fn attached_remove(&mut self, sq: Sq, o: ObjId, index: usize) {
        if let Some(l) = self.obj_mut(sq, o).and_then(|x| x.attached.as_mut()) {
            l.remove(index);
        }
    }
    fn attached_clear(&mut self, sq: Sq, o: ObjId) {
        if let Some(x) = self.obj_mut(sq, o) {
            x.attached.get_or_insert_with(Vec::new).clear();
        }
    }
    fn attached_add(&mut self, sq: Sq, o: ObjId, sprite: &str) -> i32 {
        let id = self.id_of(sprite);
        if let Some(x) = self.obj_mut(sq, o) {
            x.attached.get_or_insert_with(Vec::new).push(sprite.to_string());
        }
        id
    }
    fn remove_tile_object(&mut self, sq: Sq, o: ObjId) {
        self.squares.get_mut(&sq).unwrap().retain(|x| x.id != o);
    }
    fn remove_tile_object_erosion_no_recalc(&mut self, sq: Sq, o: ObjId) {
        self.remove_tile_object(sq, o);
    }
    fn remove_object_raw(&mut self, sq: Sq, o: ObjId) {
        self.remove_tile_object(sq, o);
    }
    fn add_object(&mut self, sq: Sq, sprite: &str, name: &str, _tree: bool) -> ObjId {
        let id = self.add(sq, sprite);
        self.set_object_name(sq, id, Some(name));
        id
    }
}

struct NoRand;
impl pz_world::GameRand for NoRand {
    fn next(&mut self, _: i32) -> i32 {
        panic!("global Rand")
    }
}

fn tables() -> ErosionTables {
    ErosionTables {
        noise_perm: (0..256).map(|i| (i * 37 + 11) % 256).collect::<Vec<i32>>().into(),
        soil_table: vec![vec![1; 10]; 10],
        square_rands: (0..8100).map(|i| (i * 13) % 100).collect(),
        trees: vec![TreeInit::new("T", "t_x_1", true)],
        trees_soil_ref: vec![vec![1, 1]; 12],
        plants: Vec::new(),
        plants_soil_ref: vec![vec![1]; 12],
        bush: Vec::new(),
        bush_soil_ref: vec![vec![1]; 12],
        flowerbed_tile_ids: vec![16, 17],
        season_names: (0..6).map(|i| i.to_string()).collect(),
        config_defaults: Vec::new(),
    }
}

fn season() -> ErosionSeason {
    let t = tables();
    let mut s = ErosionSeason::new(t.noise_perm.clone(), t.season_names, Some(Arc::new(|_| 0)));
    s.init(38, 25, 0, 7, 31, 12.5, 64, 128, 255);
    s.set_rain([0.3; 12]);
    s.set_day(10, 6, 1993);
    s
}

fn spawn(cat: &mut Category, host: &mut Mock, sq: Sq, sd: &mut SquareData, season: &ErosionSeason) -> bool {
    let t = tables();
    let mut g = NoRand;
    let mut ctx = Ctx { season, rand: SquareRand { use_static: true, table: &t.square_rands, global: &mut g } };
    let walls = host.walls;
    cat.validate_spawn(host, sq, sd, &ChunkData::default(), true, walls, false, &mut ctx)
}

fn update(cat: &mut Category, host: &mut Mock, sq: Sq, sd: &mut SquareData, e_tick: i32, season: &ErosionSeason) -> Keep {
    let t = tables();
    let mut g = NoRand;
    let ctx = Ctx { season, rand: SquareRand { use_static: true, table: &t.square_rands, global: &mut g } };
    let mut d = sd.regions[0].clone();
    let k = cat.update(host, sq, sd, &mut d, e_tick, &ctx);
    sd.regions[0] = d;
    k
}

#[test]
fn street_cracks_overlay_and_grass() {
    let (mut regions, _) = ErosionRegions::init(&tables());
    let season = season();
    let mut host = Mock::default();
    let sq = (5, 5, 0);
    host.add(sq, "blends_street_01_0");
    let mut sd = SquareData { noise_main_int: 99, magic_num: 0.9, ..Default::default() };
    let cat = regions.get_category(1, 0).unwrap();
    assert!(spawn(cat, &mut host, sq, &mut sd, &season));
    let d = sd.regions[0].clone();
    assert_eq!((d.max_stage, d.spawn_time, d.has_grass), (2, 51, true));
    assert_eq!(update(cat, &mut host, sq, &mut sd, 50, &season), Keep::Keep);
    assert!(host.squares[&sq].len() == 1, "nothing before spawnTime");
    // 60-tick cycle over 3 stages: stage 2 at 51 + 40.
    update(cat, &mut host, sq, &mut sd, 91, &season);
    let floor = &host.squares[&sq][0];
    assert_eq!(floor.attached.as_ref().unwrap().len(), 1);
    assert!(floor.attached.as_ref().unwrap()[0].starts_with("d_streetcracks_1_"));
    assert_eq!(sd.regions[0].stage, 2);
    // The grass object was placed and named.
    assert_eq!(host.squares[&sq].len(), 2);
    assert_eq!(host.squares[&sq][1].name.as_deref(), Some("CrackGrass"));
    assert!(sd.regions[0].has_spawned);
}

#[test]
fn wall_vines_and_cracks_need_walls() {
    let (mut regions, _) = ErosionRegions::init(&tables());
    let season = season();
    for cat_id in 0..2 {
        let mut host = Mock::default();
        let sq = (1, 1, 0);
        host.add(sq, "floor");
        let e = if cat_id == 0 { 70 } else { 20 };
        let mut sd = SquareData { noise_main_int: e, magic_num: 0.5, ..Default::default() };
        let cat = regions.get_category(2, cat_id).unwrap();
        assert!(!spawn(cat, &mut host, sq, &mut sd, &season), "no walls");
        host.walls = true;
        let w = host.add(sq, "wall_n");
        assert!(spawn(cat, &mut host, sq, &mut sd, &season));
        update(cat, &mut host, sq, &mut sd, 100, &season);
        let wall = host.obj(sq, w).unwrap();
        let prefix = if cat_id == 0 { "f_wallvines_1_" } else { "d_wallcracks_1_" };
        assert!(wall.attached.as_ref().unwrap()[0].starts_with(prefix));
        // Re-staging replaces the overlay instead of stacking it.
        let id = sd.regions[0].cur_id;
        assert!(id >= 0);
        update(cat, &mut host, sq, &mut sd, 120, &season);
        assert_eq!(host.obj(sq, w).unwrap().attached.as_ref().unwrap().len(), 1);
    }
}

#[test]
fn tree_replacement_and_staging() {
    let (mut regions, ice) = ErosionRegions::init(&tables());
    assert!(ice.iter().any(|(a, b)| a == "t_x_1_0" && b == "t_x_1_4"));
    let season = season();
    let mut host = Mock::default();
    let sq = (2, 2, 0);
    host.add(sq, "blends_natural_01_5");
    let jumbo = host.add(sq, "t_xJUMBO_1_3");
    let mut sd = SquareData { noise_main_int: 40, soil: 0, ..Default::default() };
    let cat = regions.get_category(0, 0).unwrap();
    let t = tables();
    let mut g = NoRand;
    let mut ctx = Ctx { season: &season, rand: SquareRand { use_static: true, table: &t.square_rands, global: &mut g } };
    assert!(cat.replace_existing_object(&mut host, sq, &mut sd, &ChunkData::default(), true, false, &mut ctx));
    assert!(host.obj(sq, jumbo).is_none(), "map tree removed");
    assert_eq!((sd.regions[0].max_stage, sd.regions[0].stage), (5, 5));
    update(cat, &mut host, sq, &mut sd, 0, &season);
    let placed = &host.squares[&sq][1];
    assert_eq!(placed.name.as_deref(), Some("T"));
    // Evergreen: stage 5 is the second JUMBO sheet, no seasonal children.
    assert_eq!(placed.sprite, "t_xJUMBO_1_1");
}

#[test]
fn round_robin_base_sprites() {
    let mut spr = ErosionObjSprites::new(1, "P", false, false, false);
    spr.set_base_list(0, &["a".to_string(), "b".to_string()], 1);
    let mut obj = ErosionObj::new(spr, 30, 0.0, 0.0, false);
    let mut host = Mock::default();
    let sq = (0, 0, 0);
    host.add(sq, "floor");
    // placeObject draws once in createObject and once in setStageObject.
    assert!(obj.place_object(&mut host, sq, 0, false, 1, false));
    assert_eq!(host.squares[&sq][1].sprite, "b");
    assert!(obj.set_stage(&mut host, sq, 0, 1, false));
    assert_eq!(host.squares[&sq][1].sprite, "a");
    assert!(!obj.place_object(&mut host, sq, 3, false, 1, false));
    assert!(obj.remove_object(&mut host, sq).is_some());
}
