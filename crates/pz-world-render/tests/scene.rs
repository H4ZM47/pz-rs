//! A small hand-built scene (authored test data, not game data) through
//! pz-world, the render squares and lighting.

use pz_iso_render::CutawayWorld;
use pz_lighting::jni::{IsoDirection, PlayerRenderSettings, VisionConeInput};
use pz_world::objects::{IsoLightSource, IsoLightSwitch};
use pz_world::sprite::PropertyValueMap;
use pz_world::{
    IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectKind,
    SpriteTables,
};
use pz_world_render::{
    default_player, seam_neighbours, wall_type, NoTextures, NoWorldHooks, NoonSettings,
    PlayerFrameInput, RenderSettingsSource, RenderWorld, SpriteCache, WorldLighting, WorldView,
};

/// Hand-authored enum tables: the names pz-world and this crate refer to,
/// in an arbitrary order.
fn tables() -> SpriteTables {
    let flags = "collideW,collideN,exterior,NoWallLighting,doorW,doorN,transparentW,\
transparentN,cutW,cutN,solid,invisible,HoppableN,HoppableW,canPathW,canPathN,blocksight,\
ForceAmbient,WallSE,FloorHeightOneThird,FloorHeightTwoThirds,CantClimb,open,windowW,windowN,\
WindowW,WindowN,diamondFloor,transparentFloor,solidfloor,hidewalls,container,WallOverlay,\
attachedSE,attachedW,attachedN,FloorOverlay,vegitation,NeverCutaway,HasLightOnSprite,\
EntityScript,water,solidtrans,blueprint,BlockRain,trans,noStart,forceRender,burntOut,WallN,WallW,WallNW";
    let types = "normal,wall,doorW,doorN,tree,windowFN,windowFW,isMoveAbleObject,lightswitch,\
curtainN,curtainS,curtainE,curtainW,jukebox,radio,doorFrN,doorFrW,stairsTN,stairsMN,stairsBN,\
stairsTW,stairsMW,stairsBW";
    SpriteTables::from_text(&format!("{flags}\n{types}\nN,W\n")).unwrap()
}

type Spec<'a> = (&'a str, &'a str, &'a [&'a str]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", &["solidfloor"]),
    ("wall_w", "wall", &["collideW", "cutW", "WallW"]),
    ("wall_n", "wall", &["collideN", "cutN", "WallN"]),
    ("roof", "normal", &["solidfloor"]),
    ("lamp", "lightswitch", &[]),
];

fn sprites() -> IsoSpriteManager {
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
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, flags)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 10 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t;
        for f in *flags {
            sp.properties.set(&tables, &alias, f, "", true);
        }
        sp.cut_n = sp.properties.has(tables.flag.cutN);
        sp.cut_w = sp.properties.has(tables.flag.cutW);
    }
    m
}

fn put(cell: &mut IsoCell, m: &IsoSpriteManager, x: i32, y: i32, z: i32, names: &[&str]) {
    let mut sq = IsoGridSquare::new(x, y, z);
    for n in names {
        let id = sq.new_object_id();
        sq.objects.push(IsoObject::new(
            id,
            ObjectKind::Object,
            Some(m.named_map[*n]),
        ));
    }
    cell.chunks
        .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
        .unwrap()
        .set_square(x.rem_euclid(8), y.rem_euclid(8), z, Some(sq));
}

/// Chunks (1..=3, 1..=3), floors everywhere on level 0, a west wall along
/// x = 20 for y in 10..=20, and a roofed square at (12, 12).
fn scene() -> (IsoCell, IsoSpriteManager, IsoMetaGrid) {
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
            let wall = x == 20 && (10..=20).contains(&y);
            put(
                &mut cell,
                &m,
                x,
                y,
                0,
                if wall {
                    &["floor", "wall_w"]
                } else {
                    &["floor"]
                },
            );
        }
    }
    put(&mut cell, &m, 12, 12, 1, &["roof"]);
    for wx in 1..=3 {
        for wy in 1..=3 {
            cell.recalc_chunk_squares(wx, wy, &meta, &m);
        }
    }
    (cell, m, meta)
}

fn build(view: &WorldView<'_>) -> (RenderWorld, WorldLighting) {
    let mut world = RenderWorld::new();
    let mut lighting = WorldLighting::new();
    let mut cache = SpriteCache::new();
    for wx in 1..=3 {
        for wy in 1..=3 {
            world.build_chunk(view, wx, wy, &mut cache, &mut NoTextures);
            lighting.sync_chunk(view, wx, wy, &NoWorldHooks);
        }
    }
    (world, lighting)
}

fn run(
    view: &WorldView<'_>,
    lighting: &mut WorldLighting,
    settings: &mut dyn RenderSettingsSource,
    frames: usize,
    dir: IsoDirection,
) -> Vec<usize> {
    let mut dirty = Vec::new();
    for f in 0..frames {
        let input = PlayerFrameInput {
            index: 0,
            player: default_player(14.5, 15.5, 0.0, dir),
            cone: VisionConeInput {
                worn_items_vision_multiplier: 1.0,
                ..Default::default()
            },
            chunk_x_min: 1,
            chunk_y_min: 1,
            chunk_grid_width: 3,
        };
        let r = lighting
            .frame(
                view,
                &input,
                settings,
                &NoWorldHooks,
                f as u64 * 33_000_000,
                &|_, _, _| true,
            )
            .unwrap();
        dirty.push(r.dirty_levels.len());
    }
    dirty
}

#[test]
fn render_squares_from_world() {
    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (world, _) = build(&view);
    let sq = &world.squares[&(20, 15, 0)];
    assert_eq!(sq.objects.len(), 2);
    let wall = sq.objects[1].sprite.as_ref().unwrap();
    assert!(wall.flags.wall_w && wall.cut_w);
    assert_eq!(wall.tile_type, pz_iso_render::TileType::Wall);
    // The square west of the wall sees it as its east neighbour's west wall.
    assert_eq!(
        wall_type(&view, cell.get_grid_square(19, 15, 0).unwrap()) & 8,
        8
    );
    let seam = seam_neighbours(&view, cell.get_grid_square(20, 14, 0).unwrap());
    assert!(seam.s_floor && seam.e_floor && seam.s_wall_w);
    assert!(world.squares[&(20, 14, 0)].seam.s_wall_w);
    // The roof blocks vision down, and the square under it is not exterior.
    assert!(world.squares[&(12, 12, 1)].cut.vision_blocked_below);
    assert!(!cell.get_grid_square(12, 12, 0).unwrap().is_exterior(&m));
    assert_eq!(world.chunk_levels(1, 1), Some((0, 1)));
    assert_eq!(world.on_screen_chunks().len(), 9);
    assert_eq!(world.chunk_inputs(&[(1, 1), (9, 9)]).len(), 1);
}

#[test]
fn noon_lights_what_the_player_sees() {
    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (mut world, mut lighting) = build(&view);
    let mut settings = NoonSettings::default();
    let dirty = run(&view, &mut lighting, &mut settings, 12, IsoDirection::E);
    // Results arrive a frame late, then settle.
    assert_eq!(dirty[0], 0);
    assert!(dirty[1] > 0);
    assert_eq!(*dirty.last().unwrap(), 0);
    lighting.apply(0, &mut world);
    let here = &world.squares[&(14, 15, 0)];
    assert!(here.cut.can_see && here.cut.seen);
    assert!(here.light_level > 0.9, "{}", here.light_level);
    assert_ne!(here.vert_lights[0] & 0x00FF_FFFF, 0);
    // Ahead in the cone, before the wall: seen and lit.
    let ahead = &world.squares[&(18, 15, 0)];
    assert!(ahead.cut.can_see && ahead.light_level > 0.9);
    // Behind the wall: never seen.
    let behind = &world.squares[&(24, 15, 0)];
    assert!(!behind.cut.seen && !behind.cut.can_see);
    assert_eq!(behind.light_level, 0.0);
}

struct Night;

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

#[test]
fn lamp_lights_its_surroundings_at_night() {
    let (mut cell, m, meta) = scene();
    // A lamp post switch at (15, 15) lighting radius 6.
    let sq = cell.get_grid_square_mut(15, 15, 0).unwrap();
    let id = sq.new_object_id();
    let light = IsoLightSource {
        x: 15,
        y: 15,
        z: 0,
        r: 1.0,
        g: 0.9,
        b: 0.8,
        radius: 6,
        active: true,
        hydro_powered: true,
    };
    let switch = IsoLightSwitch {
        activated: true,
        light_room: false,
        room_id: -1,
        street_light: false,
        can_be_modified: false,
        primary_r: 1.0,
        primary_g: 0.9,
        primary_b: 0.8,
        lights: vec![light],
    };
    sq.objects.push(IsoObject::new(
        id,
        ObjectKind::LightSwitch(switch),
        Some(m.named_map["lamp"]),
    ));
    sq.special_objects.push(id);
    cell.recalc_all_with_neighbours(15, 15, 0, &meta, &m);
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (mut world, mut lighting) = build(&view);
    run(&view, &mut lighting, &mut Night, 12, IsoDirection::E);
    lighting.apply(0, &mut world);
    let near = world.squares[&(16, 15, 0)].light_level;
    let far = world.squares[&(14, 26, 0)].light_level;
    assert!(near > far + 0.2, "near {near} far {far}");
}
