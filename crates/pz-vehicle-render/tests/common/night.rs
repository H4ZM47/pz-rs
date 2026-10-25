//! A hand-built night scene for the lighting tests (authored test data,
//! not game data): floors on chunks (1..=3, 1..=3), night render settings,
//! the engine's torch hook with the cell's vehicles, and a lighting driver.

use std::cell::RefCell;
use std::rc::Rc;

use pz_lighting::jni::{IsoDirection, PlayerRenderSettings, TorchInfo, VisionConeInput};
use pz_vehicle_render::lights::cell_torches;
use pz_vehicles::BaseVehicle;
use pz_world::sprite::PropertyValueMap;
use pz_world::{
    IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectKind,
    SpriteTables, SquareHooks,
};
use pz_world_render::{
    default_player, NoTextures, PlayerFrameInput, RenderSettingsSource, RenderWorld, SpriteCache,
    WorldHooks, WorldLighting, WorldView,
};

/// Authored enum tables with the names pz-world refers to.
pub fn sprites() -> IsoSpriteManager {
    let flags = "collideW,collideN,exterior,NoWallLighting,doorW,doorN,transparentW,\
transparentN,cutW,cutN,solid,invisible,HoppableN,HoppableW,canPathW,canPathN,blocksight,\
ForceAmbient,WallSE,FloorHeightOneThird,FloorHeightTwoThirds,CantClimb,open,windowW,windowN,\
WindowW,WindowN,diamondFloor,transparentFloor,solidfloor,hidewalls,container,WallOverlay,\
attachedSE,attachedW,attachedN,FloorOverlay,vegitation,NeverCutaway,HasLightOnSprite,\
EntityScript,water,solidtrans,blueprint,BlockRain,trans,noStart,forceRender,burntOut,WallN,WallW,WallNW";
    let types = "normal,wall,doorW,doorN,tree,windowFN,windowFW,isMoveAbleObject,lightswitch,\
curtainN,curtainS,curtainE,curtainW,jukebox,radio,doorFrN,doorFrW,stairsTN,stairsMN,stairsBN,\
stairsTW,stairsMW,stairsBW";
    let tables = SpriteTables::from_text(&format!("{flags}\n{types}\nN,W\n")).unwrap();
    let mut map = PropertyValueMap::default();
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    let mut m = IsoSpriteManager::new(tables);
    m.generate_alias_map(map, &[]).unwrap();
    let s = m.add_sprite("floor", 10);
    let t = m.tables.object_type_from_string("normal");
    let (tables, alias) = (m.tables.clone(), m.alias.clone());
    let sp = &mut m.sprites[s];
    sp.name = Some("floor".into());
    sp.tile_type = t;
    sp.properties.set(&tables, &alias, "solidfloor", "", true);
    m
}

/// Chunks (1..=3, 1..=3) with a floor on every level-0 square.
pub fn scene() -> (IsoCell, IsoSpriteManager, IsoMetaGrid) {
    let m = sprites();
    let meta = IsoMetaGrid::empty();
    let mut cell = IsoCell::default();
    for wx in 1..=3 {
        for wy in 1..=3 {
            cell.chunks.insert((wx, wy), IsoChunk::new(wx, wy));
        }
    }
    for x in 8..32 {
        for y in 8..32 {
            let mut sq = IsoGridSquare::new(x, y, 0);
            let id = sq.new_object_id();
            sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
                id,
                ObjectKind::Object,
                Some(m.named_map["floor"]),
            )));
            cell.chunks
                .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
                .unwrap()
                .set_square(x.rem_euclid(8), y.rem_euclid(8), 0, Some(sq));
        }
    }
    for wx in 1..=3 {
        for wy in 1..=3 {
            cell.recalc_chunk_squares(wx, wy, &meta, &m);
        }
    }
    (cell, m, meta)
}

pub struct Night;

impl RenderSettingsSource for Night {
    fn player_settings(&mut self, _player: i32) -> PlayerRenderSettings {
        PlayerRenderSettings {
            rmod: 0.1,
            gmod: 0.1,
            bmod: 0.15,
            ambient: 0.1,
            night: 1.0,
            view_distance: 30.0,
        }
    }

    fn day_light_strength(&self) -> f32 {
        0.0
    }

    fn night(&self) -> f32 {
        1.0
    }
}

/// The engine's hooks: no player light items, then the cell's vehicles.
pub struct VehicleTorches<'a>(pub &'a [&'a BaseVehicle]);

impl SquareHooks for VehicleTorches<'_> {}

impl WorldHooks for VehicleTorches<'_> {
    fn torches(&self) -> Vec<TorchInfo> {
        cell_torches(self.0.iter().copied())
    }
}

pub fn charge_battery(v: &BaseVehicle) {
    let battery = v.battery().unwrap();
    let mut t =
        pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Drainable, "Base", "CarBattery1");
    t.use_delta = 0.1;
    let mut i = pz_inventory::InventoryItem::create(Rc::new(t), -1);
    i.uses = 10;
    i.condition = 100;
    battery.st_mut().item = Some(pz_vehicles::part::PartItem::Item(Rc::new(RefCell::new(i))));
}

/// The player at `(x, y)` looking along `dir`, frames run and lighting
/// applied to the level's squares.
pub fn light_from(
    view: &WorldView<'_>,
    lighting: &mut WorldLighting,
    hooks: &dyn WorldHooks,
    frames: usize,
    (x, y): (f32, f32),
    dir: IsoDirection,
) -> RenderWorld {
    let mut world = RenderWorld::new();
    let mut cache = SpriteCache::new();
    for wx in 1..=3 {
        for wy in 1..=3 {
            world.build_chunk(view, wx, wy, &mut cache, &mut NoTextures);
        }
    }
    for f in 0..frames {
        let input = PlayerFrameInput {
            index: 0,
            player: default_player(x, y, 0.0, dir),
            cone: VisionConeInput {
                worn_items_vision_multiplier: 1.0,
                ..Default::default()
            },
            chunk_x_min: 1,
            chunk_y_min: 1,
            chunk_grid_width: 3,
        };
        lighting
            .frame(
                view,
                &input,
                &mut Night,
                hooks,
                f as u64 * 33_000_000,
                &|_, _, _| true,
            )
            .unwrap();
    }
    lighting.apply(0, &mut world);
    world
}

/// A player standing beside the lamp side of the car, looking east.
pub fn light(
    view: &WorldView<'_>,
    lighting: &mut WorldLighting,
    hooks: &dyn WorldHooks,
    frames: usize,
) -> RenderWorld {
    light_from(view, lighting, hooks, frames, (14.5, 16.5), IsoDirection::E)
}

pub fn new_lighting(view: &WorldView<'_>, hooks: &dyn WorldHooks) -> WorldLighting {
    let mut lighting = WorldLighting::new();
    for wx in 1..=3 {
        for wy in 1..=3 {
            lighting.sync_chunk(view, wx, wy, hooks);
        }
    }
    lighting
}
