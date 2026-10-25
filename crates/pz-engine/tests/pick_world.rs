//! Object picking over what the chunk renderer recorded: a chunk level
//! recorded and composited, the translucent pass, then
//! `FBORenderObjectPicker.ContextPick` through `EnginePick` over pz-world-render's `WorldPick`. Hand-built
//! scene (authored test data, not game data).

use std::rc::Rc;

use pz_engine::world_hud::{EnginePick, PickHooks};
use pz_iso_render::cell::Layer;
use pz_iso_render::chunk::ChunkTexture;
use pz_iso_render::render_info::ObjectSlot;
use pz_iso_render::world::{DepthMaps, DepthPreset};
use pz_iso_render::{Camera, ChunkRenderer, RenderInfos};
use pz_lua_api::hud::picking::{click_objects, context_pick, PickPlayer};
use pz_render::image::BooleanGrid;
use pz_render::{Mask, Texture};
use pz_world::sprite::PropertyValueMap;
use pz_world::{
    IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectKind,
    SpriteTables,
};
use pz_world_render::{RenderWorld, SpriteCache, SpriteTextures, WorldPick, WorldView};

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

const SPRITES: &[(&str, &str, &[&str])] = &[
    ("floor", "normal", &["solidfloor"]),
    ("crate", "normal", &["container", "solid"]),
    ("door_w", "doorW", &["collideW", "cutW", "doorW"]),
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

/// A 128x256 frame whose trimmed 40x60 image sits at (44, 150), its mask
/// set everywhere.
fn texture(name: &str) -> Texture {
    let mut t = Texture::empty(name);
    t.id = Some(1);
    t.width = 40;
    t.height = 60;
    t.width_orig = 128;
    t.height_orig = 256;
    t.offset_x = 44.0;
    t.offset_y = 150.0;
    t.width_hw = 64;
    t.height_hw = 64;
    t.x_end = 40.0 / 64.0;
    t.y_end = 60.0 / 64.0;
    t.ready = true;
    let mut g = BooleanGrid::new(40, 60);
    for y in 0..60 {
        for x in 0..40 {
            g.set_value(x, y, true);
        }
    }
    t.mask = Some(Rc::new(Mask::copy_of(&g, 40, 60)));
    t
}

struct Textures;

impl SpriteTextures for Textures {
    fn texture(&mut self, name: &str) -> Option<Texture> {
        Some(texture(name))
    }
    fn depth_texture(&mut self, _name: &str, _has_tileset: bool) -> Option<Texture> {
        None
    }
    fn depth_flags(&self, _name: &str) -> i32 {
        0
    }
}

struct Maps(Texture);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.0.clone())
    }
}

struct Player(PickPlayer);

impl PickHooks for Player {
    fn player0(&self) -> Option<PickPlayer> {
        Some(self.0)
    }
}

struct Scene {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
    render: RenderWorld,
    infos: RenderInfos,
    cam: Camera,
}

const CRATE: (i32, i32) = (10, 12);
const DOOR: (i32, i32) = (13, 10);

impl Scene {
    /// Chunk (1, 1) floored, a crate at [`CRATE`], an open west door at
    /// [`DOOR`]; recorded, composited and its translucent pass drawn at
    /// `zoom`.
    fn new(zoom: f32) -> Scene {
        let m = sprites();
        let meta = IsoMetaGrid::empty();
        let mut cell = IsoCell::default();
        cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        for x in 8..16 {
            for y in 8..16 {
                let mut sq = IsoGridSquare::new(x, y, 0);
                let id = sq.new_object_id();
                sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
                    id,
                    ObjectKind::Object,
                    Some(m.named_map["floor"]),
                )));
                if (x, y) == CRATE {
                    let id = sq.new_object_id();
                    let mut o = IsoObject::new(id, ObjectKind::Object, Some(m.named_map["crate"]));
                    o.container = Some(pz_inventory::container::ItemContainer::new(Some("crate")));
                    sq.objects.push(pz_world::ObjectSlot::new(o));
                }
                if (x, y) == DOOR {
                    let id = sq.new_object_id();
                    let door = pz_world::objects::IsoDoor {
                        open: true,
                        ..Default::default()
                    };
                    sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
                        id,
                        ObjectKind::Door(door),
                        Some(m.named_map["door_w"]),
                    )));
                }
                cell.chunks.get_mut(&(1, 1)).unwrap().set_square(
                    x % 8,
                    y % 8,
                    0,
                    Some((sq).into()),
                );
            }
        }
        cell.recalc_chunk_squares(1, 1, &meta, &m);
        let mut render = RenderWorld::new();
        let mut cache = SpriteCache::new();
        render.build_chunk(
            &WorldView {
                cell: &cell,
                meta: &meta,
                sprites: &m,
            },
            1,
            1,
            &mut cache,
            &mut Textures,
        );
        let cam = Camera::centred(12.0, 12.0, 0.0, 1280, 720, zoom);
        let infos = RenderInfos::new();
        {
            let maps = Maps(texture("depth"));
            let r = ChunkRenderer::new(cam, &maps, (1280, 720)).with_render_infos(&infos, 0);
            let tex = ChunkTexture::new(1, 1, 0, 0, 0);
            let mut out = Vec::new();
            r.record(tex, &[(0, render.level_squares(1, 1, 0))], &mut out);
            r.composite(&[tex], 1280, 720, &mut out);
            let squares: Vec<_> = render.squares.values().collect();
            r.render_translucent(&squares, 0, &mut out);
        }
        Scene {
            cell,
            m,
            meta,
            render,
            infos,
            cam,
        }
    }

    fn pick(&self) -> EnginePick<'_, Player> {
        EnginePick {
            world: WorldPick {
                view: WorldView {
                    cell: &self.cell,
                    meta: &self.meta,
                    sprites: &self.m,
                },
                render: &self.render,
                infos: &self.infos,
                player: 0,
                camera: self.cam,
            },
            hooks: Player(PickPlayer {
                x: 11.5,
                y: 11.5,
                z: 0.0,
                zi: 0,
                forward: (1.0, 0.0),
                has_square: true,
                room: None,
            }),
        }
    }

    /// The UI point over the middle of the drawn image of an object on
    /// `(x, y)` (the trimmed image covers 44..84 x 150..210 of the frame
    /// drawn at the square's screen position minus (64, 192)).
    fn point(&self, (x, y): (i32, i32)) -> (i32, i32) {
        let sx =
            pz_render::iso::x_to_screen(x as f32, y as f32) - 64.0 + 64.0 - self.cam.off_x as f32;
        let sy = pz_render::iso::y_to_screen(x as f32, y as f32, 0.0, 0) - 192.0 + 180.0
            - self.cam.off_y as f32;
        ((sx / self.cam.zoom) as i32, (sy / self.cam.zoom) as i32)
    }
}

#[test]
fn records_what_was_drawn() {
    let s = Scene::new(1.0);
    let (cx, cy) = CRATE;
    let floor = s.infos.get(0, [cx, cy, 0], 0, 1);
    let crate_ = s.infos.get(0, [cx, cy, 0], 1, 2);
    assert_eq!(floor.layer, Layer::Floor);
    assert_eq!(crate_.layer, Layer::MinusFloor);
    assert_eq!((crate_.target_alpha, crate_.cutaway), (1.0, false));
    // The chunk-texture position of the whole 128x256 frame at scale 1.
    let tex = ChunkTexture::new(1, 1, 0, 0, 0);
    let (xoff, yoff) = tex.record_offset();
    let lx = (cx % 8) as f32;
    let ly = (cy % 8) as f32;
    assert_eq!(
        crate_.render_x,
        pz_render::iso::x_to_screen(lx, ly) - 64.0 + xoff
    );
    assert_eq!(
        crate_.render_y,
        pz_render::iso::y_to_screen(lx, ly, 0.0, 0) - 192.0 + yoff
    );
    assert_eq!((crate_.render_width, crate_.render_height), (128.0, 256.0));
    assert_eq!((crate_.render_scale_x, crate_.render_scale_y), (1.0, 1.0));
    // The open door: translucent at 0.6, screen relative.
    let (dx, dy) = DOOR;
    let door = s.infos.get(0, [dx, dy, 0], 1, 2);
    assert_eq!(door.layer, Layer::Translucent);
    assert_eq!(door.target_alpha, 0.6);
    assert_eq!(
        door.render_x,
        pz_render::iso::x_to_screen(dx as f32, dy as f32) - 64.0 - s.cam.off_x as f32
    );
    // The composited chunk.
    assert_eq!(
        s.infos.chunk_render_xy(0, 9, 9, 0),
        Some(tex.render_xy(&s.cam))
    );
    assert_eq!(s.infos.chunk_render_xy(0, 17, 9, 0), None);
}

#[test]
fn picks_the_container_over_its_floor() {
    for zoom in [1.0, 2.0] {
        let s = Scene::new(zoom);
        let (x, y) = s.point(CRATE);
        let c = context_pick(&s.pick(), x, y).expect("picked");
        assert_eq!((c.square, c.tile.object_index), ([CRATE.0, CRATE.1, 0], 1));
        // Clicked at the image middle: (64, 180) in the frame.
        assert!(
            (c.lx - 64).abs() <= 1 && (c.ly - 180).abs() <= 1,
            "{:?}",
            (c.lx, c.ly)
        );
        let (x, y) = s.point(DOOR);
        let c = context_pick(&s.pick(), x, y).expect("door");
        assert_eq!((c.square, c.tile.object_index), ([DOOR.0, DOOR.1, 0], 1));
    }
}

#[test]
fn click_objects_by_layer_then_square() {
    let s = Scene::new(1.0);
    let (x, y) = s.point(CRATE);
    let objs = click_objects(&s.pick(), x, y);
    let order: Vec<_> = objs
        .iter()
        .map(|c| (c.tile.render.layer, c.square))
        .collect();
    // Floors first (by y * 100000 + x), the crate last.
    assert_eq!(order.last().unwrap().1, [CRATE.0, CRATE.1, 0]);
    let mut sorted = order.clone();
    sorted.sort_by_key(|(l, sq)| {
        (
            *l != pz_lua_api::hud::picking::RenderLayer::Floor,
            sq[0] + sq[1] * 100_000,
        )
    });
    assert_eq!(order, sorted);
}

#[test]
fn cutaway_and_hidden_objects_are_not_picked() {
    let s = Scene::new(1.0);
    let (x, y) = s.point(CRATE);
    let slot = ObjectSlot {
        square: [CRATE.0, CRATE.1, 0],
        index: 1,
        id: 2,
    };
    // Faded out: the floor under it is picked instead.
    s.infos.update(0, slot, |i| i.target_alpha = 0.0);
    let c = context_pick(&s.pick(), x, y).expect("floor");
    assert_eq!(c.tile.object_index, 0);
    // Cut away (not a door, window frame or thumpable window): skipped.
    s.infos.update(0, slot, |i| {
        i.target_alpha = 1.0;
        i.cutaway = true;
    });
    assert_eq!(context_pick(&s.pick(), x, y).unwrap().tile.object_index, 0);
    // Not drawn (`setNotRendered`, or never recorded).
    s.infos.update(0, slot, |i| {
        i.cutaway = false;
        i.layer = Layer::None;
    });
    assert_eq!(context_pick(&s.pick(), x, y).unwrap().tile.object_index, 0);
    // Another object now at that index reads a fresh info.
    let fresh = RenderInfos::new();
    let mut p = s.pick();
    p.world.infos = &fresh;
    assert!(context_pick(&p, x, y).is_none());
}
