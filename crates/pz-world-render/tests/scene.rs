//! A small hand-built scene (authored test data, not game data) through
//! pz-world, the render squares and lighting.

use pz_iso_render::CutawayWorld;
use pz_lighting::jni::{Barricade, SpecialObject, Thumpable};
use pz_lighting::jni::{IsoDirection, PlayerRenderSettings, VisionConeInput};
use pz_world::objects::{IsoBarricade, IsoDoor, IsoLightSource, IsoLightSwitch, IsoThumpable};
use pz_world::sprite::PropertyValueMap;
use pz_world::world_state::PlayerLights;
use pz_world::{
    IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectKind,
    PowerSettings, SpriteTables, WorldState,
};
use pz_world_render::{
    default_player, grid_square, seam_neighbours, wall_type, CellLight, NoTextures, NoWorldHooks,
    NoonSettings, PlayerFrameInput, RenderSettings, RenderSettingsSource, RenderWorld, SpriteCache,
    StateLighting, WorldHooks, WorldLighting, WorldView,
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
    ("door_n", "doorN", &["doorN"]),
    ("thump_window", "normal", &["WindowN"]),
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
        ..Default::default()
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

    // The state hooks read the same decoded colour: `(r + g + b) / 3` and
    // `max(r, g, b)` of the `getSquareLighting` colour word (bytes r, g, b).
    let mut settings = RenderSettings::new();
    settings.update(&mut Night, [true, false, false, false], false);
    let mut words = Vec::new();
    for sq in [(16, 15, 0), (14, 26, 0), (15, 15, 0)] {
        let ints = lighting.session.get_square_lighting(0, sq.0, sq.1, sq.2 + 32).unwrap().unwrap();
        words.push((sq, ints[1]));
    }
    let hooks = StateLighting::new(&lighting, &settings);
    for (sq, c) in words {
        let (r, g, b) = (
            (c & 0xff) as f32 / 255.0,
            ((c >> 8) & 0xff) as f32 / 255.0,
            ((c >> 16) & 0xff) as f32 / 255.0,
        );
        assert_eq!(hooks.square_light(sq, 0), (r + g + b) / 3.0, "{sq:?}");
        assert_eq!(hooks.square_light_level(sq, 0), r.max(g).max(b), "{sq:?}");
        assert_eq!(hooks.square_light_level(sq, 0), world.squares[&sq].light_level);
    }
    assert!(hooks.square_light((16, 15, 0), 0) > hooks.square_light((14, 26, 0), 0));
    // A square no frame decoded keeps `new ColorInfo()` (white).
    assert_eq!(hooks.square_light((200, 200, 0), 0), 1.0);
    assert_eq!(hooks.square_light_level((200, 200, 0), 0), 1.0);
    // `getLightLevel(-1)` decodes the shared buffer, zeroed here.
    assert_eq!(hooks.square_light_level((16, 15, 0), -1), 0.0);
    // Ambient as of the last `RenderSettings.update()`; slot 1 never ran.
    assert_eq!(hooks.ambient_for_player(0), 0.1);
    assert_eq!(hooks.ambient_for_player(1), 0.0);
}

fn add_special(
    cell: &mut IsoCell,
    m: &IsoSpriteManager,
    at: (i32, i32),
    sprite: &str,
    k: ObjectKind,
) {
    let sq = cell.get_grid_square_mut(at.0, at.1, 0).unwrap();
    let id = sq.new_object_id();
    sq.objects
        .push(IsoObject::new(id, k, Some(m.named_map[sprite])));
    sq.special_objects.push(id);
}

/// Barricades, thumpables, door curtains, power and torches from
/// [`WorldState`] in lighting's square inputs and hooks.
#[test]
fn world_state_feeds_lighting() {
    let (mut cell, m, meta) = scene();
    let bar = |dir: i8, planks: [i32; 4], metal: i32| {
        ObjectKind::Barricade(IsoBarricade {
            dir,
            plank_health: planks,
            metal_health: metal,
            metal_bar_health: 0,
        })
    };
    let door = IsoDoor {
        north: true,
        has_curtain: true,
        curtain_open: false,
        ..Default::default()
    };
    add_special(&mut cell, &m, (15, 15), "door_n", ObjectKind::Door(door));
    // The S side of the door's edge lives on the square to the north.
    add_special(&mut cell, &m, (15, 14), "floor", bar(4, [9, 9, 0, 0], 0));
    let th = IsoThumpable {
        north: true,
        can_barricade: true,
        ..Default::default()
    };
    add_special(
        &mut cell,
        &m,
        (17, 15),
        "thump_window",
        ObjectKind::Thumpable(Box::new(th)),
    );
    add_special(&mut cell, &m, (17, 15), "floor", bar(0, [0; 4], 77));
    let mut p = PowerSettings {
        hydro_power_on: false,
        world_age_days: 1.0,
        elec_shut_modifier: 4,
        allow_exterior_generator: true,
        generator_tile_range: 3,
        generator_vertical_power_range: 1,
    };
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let ws = WorldState::new(&cell, &meta, &m, p);
    let g = grid_square(&view, cell.get_grid_square(15, 15, 0).unwrap(), &ws, 0, 0);
    match &g.special_objects[..] {
        [SpecialObject::Door(d)] => {
            assert!(d.has_curtain && !d.curtain_open);
            assert_eq!(d.barricade_same, None);
            let b = d.barricade_opposite.unwrap();
            assert_eq!((b.dir, b.planks), (Some(IsoDirection::S), 2));
        }
        o => panic!("{o:?}"),
    }
    let g = grid_square(&view, cell.get_grid_square(17, 15, 0).unwrap(), &ws, 0, 0);
    let metal = Barricade {
        dir: Some(IsoDirection::N),
        metal_health: 77,
        metal_bar: false,
        planks: 0,
    };
    assert_eq!(
        g.special_objects,
        vec![
            SpecialObject::Thumpable(Thumpable {
                north: true,
                is_window: true,
                barricade_same: Some(metal),
                ..Default::default()
            }),
            SpecialObject::Barricade(metal),
        ]
    );
    // The same view without the world state sees no barricades or curtain.
    let g = grid_square(
        &view,
        cell.get_grid_square(15, 15, 0).unwrap(),
        &NoWorldHooks,
        0,
        0,
    );
    assert!(
        matches!(&g.special_objects[..], [SpecialObject::Door(d)] if !d.has_curtain && d.barricade_opposite.is_none())
    );

    assert!(ws.hydro_power_off());
    // No switch at the position: no power.
    assert!(ws.light_switch_unpowered(9, 9, 0));
    p.hydro_power_on = true;
    let flashlight = {
        let mut t = pz_inventory::template::ItemTemplate::new(
            pz_inventory::template::ItemClass::Normal,
            "Test",
            "Lamp",
        );
        t.light_strength = 0.5;
        t.light_distance = 7;
        pz_inventory::item::InventoryItem::create(std::rc::Rc::new(t), 3)
    };
    let mut ws = WorldState::new(&cell, &meta, &m, p);
    ws.players.push(PlayerLights {
        index: 2,
        x: 14.5,
        y: 14.5,
        z: 0.0,
        look_x: 1.0,
        look_y: 0.0,
        items: vec![&flashlight],
    });
    assert!(!ws.hydro_power_off());
    let t = ws.torches();
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].id, t[0].dist, t[0].strength), (9, 7.0, 0.5));
}

/// Night frames for a player at (14.5, 15.5), returning the render world
/// with the results applied.
fn lit_world(view: &WorldView<'_>, lighting: &mut WorldLighting, frames: usize) -> RenderWorld {
    let mut world = RenderWorld::new();
    let mut cache = SpriteCache::new();
    for wx in 1..=3 {
        for wy in 1..=3 {
            world.build_chunk(view, wx, wy, &mut cache, &mut NoTextures);
        }
    }
    run(view, lighting, &mut Night, frames, IsoDirection::E);
    lighting.apply(0, &mut world);
    world
}

#[test]
fn a_fire_lights_its_neighbours_through_its_stages() {
    use pz_core::random::Rand;
    use pz_iso_render::fire::{FireEvent, FireSquare, FireState, FIRE_COLOR, LIGHT_RGB};
    use pz_world_render::LightLink;

    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (_, mut dark) = build(&view);
    let dark = lit_world(&view, &mut dark, 12);

    let (_, mut lighting) = build(&view);
    let r = Rand::new(21);
    let mut fire = FireState::new_fire(FireSquare::default(), 1000, 1000, &r);
    let mut link = LightLink::new();
    let (fx, fy) = (13, 15);
    // The first update adds the light (radius 5, LIGHT_R/G/B).
    fire.update(0.0, 0.0, 0, 1.0, &r);
    link.sync_fire(&mut lighting, &fire, fx, fy, 0);
    let first = link.key.unwrap();
    let ls: Vec<_> = lighting.lampposts().map(|(k, l, d)| (k, *l, d)).collect();
    assert_eq!(ls.len(), 1);
    let [lr, lg, lb] = LIGHT_RGB;
    assert_eq!(ls[0].1, CellLight::new(fx, fy, 0, lr, lg, lb, 5));
    let world = lit_world(&view, &mut lighting, 12);
    assert!(lighting.light_id(first).is_some());
    let near = |w: &RenderWorld| w.squares[&(fx + 1, fy, 0)].light_level;
    let far = |w: &RenderWorld| w.squares[&(fx + 6, fy, 0)].light_level;
    assert!(near(&world) > near(&dark) + 0.2, "{} {}", near(&world), near(&dark));
    // Six squares away, ahead of the player: outside radius 5.
    assert!((far(&world) - far(&dark)).abs() < 0.01, "{} {}", far(&world), far(&dark));

    // Stage 1 keeps radius 5; stage 2 replaces the light with radius 8 in
    // fireColor.
    let mut changed = 0;
    while fire.life_stage < 2 {
        let ev = fire.update_from_timer(1.0001, &r);
        changed += ev.iter().filter(|e| **e == FireEvent::LightChanged).count();
        link.sync_fire(&mut lighting, &fire, fx, fy, 0);
    }
    assert_eq!(changed, 1);
    let second = link.key.unwrap();
    assert_ne!(second, first);
    let ls: Vec<_> = lighting.lampposts().map(|(k, l, d)| (k, *l, d)).collect();
    let [cr, cg, cb] = FIRE_COLOR;
    assert_eq!(ls.len(), 2);
    assert_eq!((ls[0].0, ls[0].2), (first, true));
    assert_eq!((ls[1].0, ls[1].1, ls[1].2), (second, CellLight::new(fx, fy, 0, cr, cg, cb, 8), false));
    let world = lit_world(&view, &mut lighting, 12);
    assert_eq!(lighting.light_id(first), None);
    assert!(lighting.light_id(second).is_some());
    assert_eq!(lighting.lampposts().count(), 1);
    assert!(far(&world) > far(&dark) + 0.05, "{} {}", far(&world), far(&dark));

    // Through stage 5 (radius 1); its fade scales the light's colour in
    // place.
    while fire.life_stage < 5 {
        fire.update_from_timer(1.0001, &r);
        link.sync_fire(&mut lighting, &fire, fx, fy, 0);
    }
    let last = link.key.unwrap();
    assert_eq!(lighting.lamppost_mut(last).unwrap().radius, 1);
    fire.update(0.0, 0.0, 0, 0.5, &r);
    link.sync_fire(&mut lighting, &fire, fx, fy, 0);
    assert_eq!(link.key, Some(last));
    let l = *lighting.lamppost_mut(last).unwrap();
    assert_eq!((l.r, l.g, l.b), (lr * 0.5, lg * 0.5, lb * 0.5));
    lit_world(&view, &mut lighting, 2);
    assert!(lighting.light_id(last).is_some());

    // removeFromWorld: the light leaves with the next frame.
    link.remove(&mut lighting);
    let world = lit_world(&view, &mut lighting, 12);
    assert_eq!(lighting.light_id(last), None);
    assert_eq!(lighting.lampposts().count(), 0);
    assert!((near(&world) - near(&dark)).abs() < 0.01);
}

#[test]
fn a_fireplace_light_follows_its_fuel() {
    use pz_core::random::Rand;
    use pz_iso_render::hearth::{FireplaceInput, FireplaceState};
    use pz_world_render::LightLink;

    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (_, mut lighting) = build(&view);
    let r = Rand::new(2);
    let mut fp = FireplaceState::new();
    let mut link = LightLink::new();
    let input = |lit, fuel_amount| FireplaceInput {
        lit,
        fuel_amount,
        sprite_name: "fixtures_fireplace_01_0",
        container_type: Some("fireplace"),
        collide_w: true,
        ..Default::default()
    };
    let size = |_: &str| Some((128, 256));
    fp.update(&input(true, 90), 0.0, &r, &size);
    link.sync_fireplace(&mut lighting, &fp, 16, 17, 0);
    let first = link.key.unwrap();
    assert_eq!(lighting.lamppost_mut(first).unwrap().radius, 8);
    lit_world(&view, &mut lighting, 3);
    assert!(lighting.light_id(first).is_some());
    // Fuel below an hour shrinks the light: a new one replaces it.
    fp.update(&input(true, 30), 0.0, &r, &size);
    link.sync_fireplace(&mut lighting, &fp, 16, 17, 0);
    let second = link.key.unwrap();
    assert_ne!(first, second);
    assert_eq!(lighting.lamppost_mut(second).unwrap().radius, 4);
    lit_world(&view, &mut lighting, 3);
    assert_eq!(lighting.light_id(first), None);
    assert!(lighting.light_id(second).is_some());
    // Put out.
    fp.update(&input(false, 30), 0.0, &r, &size);
    link.sync_fireplace(&mut lighting, &fp, 16, 17, 0);
    assert_eq!(link.key, None);
    lit_world(&view, &mut lighting, 3);
    assert_eq!(lighting.light_id(second), None);
    assert_eq!(lighting.lampposts().count(), 0);
}

#[test]
fn a_cell_light_stays_while_inside_any_players_chunk_map() {
    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let (_, mut lighting) = build(&view);
    // Player 0's map is chunk (1, 1) alone (squares 8..16); player 1's is
    // chunks (2, 2)..(3, 3) (squares 16..32). The light is in player 1's.
    let frame = |lighting: &mut WorldLighting, f: u64| {
        let input = PlayerFrameInput {
            index: 0,
            player: default_player(12.5, 12.5, 0.0, IsoDirection::E),
            cone: VisionConeInput {
                worn_items_vision_multiplier: 1.0,
                ..Default::default()
            },
            chunk_x_min: 1,
            chunk_y_min: 1,
            chunk_grid_width: 1,
        };
        lighting
            .frame(
                &view,
                &input,
                &mut Night,
                &NoWorldHooks,
                f * 33_000_000,
                &|_, _, _| true,
            )
            .unwrap();
    };
    lighting.set_chunk_map(1, Some((2, 2, 2)));
    let key = lighting.add_lamppost(CellLight::new(20, 20, 0, 1.0, 0.5, 0.2, 4));
    for f in 0..3 {
        frame(&mut lighting, f);
    }
    // Outside player 0's map but inside player 1's: kept and sent.
    assert_eq!(lighting.lampposts().count(), 1);
    assert!(lighting.light_id(key).is_some());

    // Player 1 leaves: outside every map, the light goes.
    lighting.set_chunk_map(1, None);
    for f in 3..6 {
        frame(&mut lighting, f);
    }
    assert_eq!(lighting.lampposts().count(), 0);
    assert_eq!(lighting.light_id(key), None);
}
