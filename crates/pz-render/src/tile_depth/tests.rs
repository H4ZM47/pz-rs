use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::manager::PRESET_TILESET;
use super::*;
use crate::{Texture, TextureManager};

/// A fresh directory under the system temp dir.
fn temp_dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("pz-render-tiledepth-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// An RGBA image, `w` x `h`, filled by `f(x, y)`.
fn rgba(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&f(x, y));
        }
    }
    v
}

fn write_png(path: &Path, w: u32, h: u32, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut out = Vec::new();
    {
        let mut e = ::png::Encoder::new(&mut out, w, h);
        e.set_color(::png::ColorType::Rgba);
        e.set_depth(::png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(data).unwrap();
    }
    std::fs::write(path, out).unwrap();
}

/// The red channel of every texel of a texture's backing image.
fn red(tm: &TextureManager, t: &Texture) -> Vec<u8> {
    let d = tm.data(t.id.unwrap()).unwrap();
    d.image.as_ref().unwrap().data.pixels.chunks(4).map(|p| p[0]).collect()
}

#[test]
fn byte_conversion() {
    let dir = temp_dir("bytes");
    // Depth (blue) only at a few pixels; red and green are ignored.
    let px = |x: u32, y: u32| match (x, y) {
        (10, 200) => [9, 9, 0, 255],   // depth 0 is stored as 1
        (11, 200) => [9, 9, 255, 255], // 1.0
        (12, 200) => [9, 9, 128, 255],
        (64, 10) => [9, 9, 51, 255], // top rows: clamped unless roofs_
        (13, 200) => [9, 9, 77, 0],  // transparent: no depth
        _ => [0, 0, 0, 0],
    };
    write_png(&dir.join("depthmaps/DEPTH_walls_x.png"), 128, 256, &rgba(128, 256, px));
    write_png(&dir.join("depthmaps/DEPTH_roofs_x.png"), 128, 256, &rgba(128, 256, px));
    let mut tm = TextureManager::new();
    for name in ["walls_x", "roofs_x"] {
        let mut ts = TilesetDepthTexture::new(&dir, name, 8, 1, true);
        assert!(ts.file_exists());
        ts.load(&mut tm, true).unwrap();
        let tile = ts.tile(0).unwrap();
        assert!(!tile.is_empty());
        assert_eq!(tile.pixel(0, 0), -1.0);
        assert_eq!(tile.pixel(13, 200), -1.0);
        assert_eq!(tile.pixel(11, 200), 1.0);
        let b = depth_bytes(tile);
        assert!(!b.empty);
        assert_eq!(b.values[10 + 200 * 128], 1);
        assert_eq!(b.values[11 + 200 * 128], 255);
        assert_eq!(b.values[12 + 200 * 128] as f64, ((128.0f32 / 255.0 * 255.0) as f64).floor());
        assert_eq!(b.values[13 + 200 * 128], 0);
        assert_eq!(b.values[0], 0);
        let d = 51.0f32 / 255.0;
        let want = if name.starts_with("roofs_") { d } else { clamp::clamp_pixel_to_upper_floor(64, 10, d) };
        assert_eq!(b.values[64 + 10 * 128], ((want.min(1.0) * 255.0) as f64).floor() as u8);
        if name == "walls_x" {
            assert!(b.values[64 + 10 * 128] > 51, "the floor above pushes the top rows back");
        }
        // The upload is that byte in red: nearest filtering, repeat wrap.
        let t = tile.uploaded_texture().unwrap();
        assert_eq!(&*t.name, format!("DEPTH_{name}_0"));
        assert_eq!((t.width, t.height, t.width_hw, t.height_hw), (128, 256, 128, 256));
        assert_eq!(red(&tm, t), b.values);
        let data = tm.data(t.id.unwrap()).unwrap();
        assert!(data.min_nearest() && data.mag_nearest() && !data.clamp_to_edge() && !data.mipmaps());
        // Cells outside the one-cell image stay empty.
        assert!(ts.tiles()[1..].iter().all(|t| t.is_empty() && t.uploaded_texture().is_none()));
    }
}

#[test]
fn pixels_dropped_after_upload() {
    let dir = temp_dir("drop");
    write_png(&dir.join("depthmaps/DEPTH_a_b.png"), 256, 256, &rgba(256, 256, |x, _| if x < 128 { [0, 0, 200, 255] } else { [0, 0, 0, 0] }));
    let mut tm = TextureManager::new();
    let mut ts = TilesetDepthTexture::new(&dir, "a_b", 8, 1, true);
    ts.load(&mut tm, false).unwrap();
    let (t0, t1) = (ts.tile(0).unwrap(), ts.tile(1).unwrap());
    assert_eq!((t0.name(), t0.column(), t0.row(), t0.tileset_name()), ("a_b_0", 0, 0, "a_b"));
    assert!(!t0.is_empty() && t0.pixels().is_none() && t0.pixel(3, 3) == -1.0);
    assert!(t0.uploaded_texture().is_some());
    assert!(t1.is_empty() && t1.uploaded_texture().is_none());
    assert!(ts.tile(8).is_none() && ts.tile(-1).is_none());
    assert_eq!(ts.relative_file_name(), "media/depthmaps/DEPTH_a_b.png");
    // A file that is not a PNG is an error, and leaves the tiles empty.
    std::fs::write(dir.join("depthmaps/DEPTH_bad.png"), b"nope").unwrap();
    let mut bad = TilesetDepthTexture::new(&dir, "bad", 8, 1, true);
    assert!(bad.load(&mut tm, false).is_err());
    assert!(bad.is_empty());
}

/// Game media with a tileset, a sheet without sprites and the special
/// sheets, and one mod.
fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = temp_dir(tag);
    let game = root.join("game/media");
    let modm = root.join("mod/media");
    // `walls_01`: two rows; cells 0 and 9 have depth 100, cell 1 depth 50.
    write_png(
        &game.join("depthmaps/DEPTH_walls_01.png"),
        1024,
        512,
        &rgba(1024, 512, |x, y| match (x / 128 + (y / 256) * 8, y % 256 >= 128) {
            (0 | 9, true) => [0, 0, 100, 255],
            (1, true) => [0, 0, 50, 255],
            _ => [0, 0, 0, 0],
        }),
    );
    // A sheet whose tileset has no sprites: never loaded.
    write_png(&game.join("depthmaps/DEPTH_nosprites_01.png"), 128, 256, &rgba(128, 256, |_, _| [0, 0, 7, 255]));
    // Presets: Floor (0) and NWall (5) have depth; the rest are empty.
    write_png(
        &game.join("depthmaps/DEPTH_preset_depthmaps_01.png"),
        1024,
        256,
        &rgba(1024, 256, |x, _| match x / 128 {
            0 => [0, 0, 255, 255],
            5 => [0, 0, 20, 255],
            _ => [0, 0, 0, 0],
        }),
    );
    write_png(&game.join("depthmaps/DEPTH_whole_tile.png"), 128, 256, &rgba(128, 256, |_, y| if y > 100 { [0, 0, 30, 255] } else { [0, 0, 0, 0] }));
    std::fs::write(
        game.join("tileDepthTextureAssignments.txt"),
        "tileDepthTextureAssignments\n{\n    VERSION = 1,\n    walls_01_2 = walls_01_0,\n    walls_01_3 = walls_01_4,\n    walls_01_5 = walls_01_6,\n    other_7 = walls_01_99,\n}\n",
    )
    .unwrap();
    // The mod replaces cell 0, leaves cell 1 to the game, and adds cell 4.
    write_png(
        &modm.join("depthmaps/DEPTH_walls_01.png"),
        1024,
        256,
        &rgba(1024, 256, |x, _| match x / 128 {
            0 => [0, 0, 200, 255],
            4 => [0, 0, 220, 255],
            _ => [0, 0, 0, 0],
        }),
    );
    std::fs::write(modm.join("tileDepthTextureAssignments.txt"), "x\n{\n    VERSION = 1,\n    walls_01_3 = walls_01_1,\n}\n").unwrap();
    (game, modm)
}

/// Sprites `walls_01_0` .. `walls_01_11` exist: two rows.
fn walls_only(_: &mut TextureManager, name: &str) -> bool {
    name.strip_prefix("walls_01_").and_then(|i| i.parse::<i32>().ok()).is_some_and(|i| i < 12)
}

fn tile_name(t: Option<&Rc<TileDepthTexture>>) -> Option<String> {
    t.map(|t| t.name().to_string())
}

#[test]
fn manager_loads_and_merges() {
    let (game, modm) = fixture("mgr");
    let mut tm = TextureManager::new();
    let mods = vec![("mymod".to_string(), modm)];
    let mgr = TileDepthTextureManager::init_from_dirs(&mods, &game, &mut tm, &mut walls_only, false);
    assert!(mgr.errors.is_empty(), "{:?}", mgr.errors);
    assert_eq!(mgr.mod_data.iter().map(|d| d.mod_id.as_str()).collect::<Vec<_>>(), ["mymod", "game"]);
    let g = mgr.mod_data("game").unwrap();
    assert!(g.textures.tileset("nosprites_01").is_none());
    let walls = g.textures.tileset("walls_01").unwrap();
    assert_eq!((walls.columns(), walls.rows()), (8, 2));
    assert!(g.textures.tileset(PRESET_TILESET).is_some());
    assert!(mgr.mod_data("mymod").unwrap().textures.tileset(PRESET_TILESET).is_none());

    // Merged: the mod's cells win where they have depth; tiles are shared.
    let mod_tile = mgr.mod_data("mymod").unwrap().textures.texture("walls_01", 0).unwrap();
    assert!(Rc::ptr_eq(mgr.texture("walls_01", 0).unwrap(), mod_tile));
    let game_tile = g.textures.texture("walls_01", 1).unwrap();
    assert!(Rc::ptr_eq(mgr.texture("walls_01", 1).unwrap(), game_tile));
    assert!(!mgr.texture("walls_01", 4).unwrap().is_empty());
    assert!(mgr.texture("walls_01", 2).unwrap().is_empty());
    // Rows come from the sprites, not the image: the mod's tileset has two
    // rows too, its second one empty, so cell 9 is the game's.
    assert_eq!(mgr.mod_data("mymod").unwrap().textures.tileset("walls_01").unwrap().rows(), 2);
    assert_eq!(mgr.merged().tileset("walls_01").unwrap().rows(), 2);
    assert!(Rc::ptr_eq(mgr.texture("walls_01", 9).unwrap(), g.textures.texture("walls_01", 9).unwrap()));
    assert!(mgr.texture("walls_01", 16).is_none());
    assert!(mgr.texture("nope", 0).is_none());
    assert_eq!(tile_name(mgr.texture_from_tile_name("walls_01_4")), Some("walls_01_4".into()));
    assert!(mgr.texture_from_tile_name("walls_01_x").is_none());
    assert!(mgr.texture_from_tile_name("walls").is_none());
    assert_eq!(split_tile_name("a_b_12"), Some(("a_b", 12)));
}

#[test]
fn sprite_depth_textures() {
    let (game, modm) = fixture("sprites");
    let mut tm = TextureManager::new();
    let mods = vec![("mymod".to_string(), modm)];
    let mgr = TileDepthTextureManager::init_from_dirs(&mods, &game, &mut tm, &mut walls_only, false);
    let asg = TileDepthTextureAssignmentManager::init_from_dirs(&mods, &game);
    assert!(asg.errors.is_empty(), "{:?}", asg.errors);
    let sprite = |name: &str, has_tileset: bool| tile_name(mgr.sprite_depth_texture(name, has_tileset, &asg));
    assert_eq!(sprite("walls_01_0", true), Some("walls_01_0".into()));
    // A sprite's own tile: the first mod data where it has depth.
    assert_eq!(sprite("walls_01_9", true), Some("walls_01_9".into()));
    let t = mgr.sprite_depth_texture("walls_01_9", true, &asg).unwrap();
    assert!(Rc::ptr_eq(t, mgr.mod_data("game").unwrap().textures.texture("walls_01", 9).unwrap()));
    // Assignments: the mod's file first; targets come from the merged set.
    assert_eq!(sprite("walls_01_2", true), Some("walls_01_0".into()));
    assert_eq!(sprite("walls_01_3", true), Some("walls_01_1".into()));
    // Only for sprites with a tileset name.
    assert_eq!(sprite("walls_01_3", false), None);
    // Assigned to an empty tile, or to one outside the merged tileset.
    assert_eq!(sprite("walls_01_5", true), None);
    assert_eq!(sprite("other_7", true), None);
    // Names must match a tile name exactly.
    assert_eq!(sprite("walls_01_00", true), None);
}

#[test]
fn presets_default_and_empty() {
    let (game, _) = fixture("presets");
    let mut tm = TextureManager::new();
    let mut mgr = TileDepthTextureManager::init_from_dirs(&[], &game, &mut tm, &mut walls_only, false);
    // Non-empty preset cells upload; empty ones share the empty texture.
    let floor = mgr.texture_for_preset(TileDepthPreset::Floor).clone();
    assert_eq!(&*floor.name, "DEPTH_preset_depthmaps_01_0");
    assert!(red(&tm, &floor)[200 * 128..].iter().all(|&v| v == 255));
    assert_eq!(&*mgr.texture_for_preset(TileDepthPreset::NWall).name, "DEPTH_preset_depthmaps_01_5");
    let empty = mgr.texture_for_preset(TileDepthPreset::WWall).clone();
    assert_eq!(&*empty.name, "DEPTH_Empty_128x256");
    assert_eq!(empty.id, mgr.texture_for_preset(TileDepthPreset::SEWall).id);
    assert!(red(&tm, &empty).iter().all(|&v| v == 0));
    assert_eq!(mgr.presets().len(), 8);
    assert_eq!(mgr.preset_tileset().unwrap().columns(), 8);
    // The whole-tile default exists; the billboard sheet is missing here.
    assert!(!mgr.default_depth_texture().unwrap().is_empty());
    let billboard = mgr.billboard_depth_texture().unwrap().clone();
    assert!(billboard.is_empty());
    assert_eq!(mgr.texture_of(&billboard, &mut tm).id, empty.id);
}

#[test]
fn modifier_uv1() {
    let mut tm = TextureManager::new();
    let mut depth = tm.texture_from_pixels("d", 128, 256, vec![0; 128 * 256 * 4], DEPTH_TEXTURE_FLAGS);
    // A trimmed sprite texture: 60x100 placed at (30, 140) in its frame.
    let mut tex0 = Texture::empty("s");
    tex0.width = 60;
    tex0.height = 100;
    tex0.offset_x = 30.0;
    tex0.offset_y = 140.0;
    let uv = depth_uv1(&tex0, &depth, 0, 0, 1.0, 1.0);
    let (l, r, t, b) = (30.0 / 128.0, 90.0 / 128.0, 140.0 / 256.0, 240.0 / 256.0);
    assert_eq!(uv, [[l, t], [r, t], [r, b], [l, b]]);
    // The sprite offset moves it; the depth texture's edges clip it.
    let uv = depth_uv1(&tex0, &depth, -40, 20, 1.0, 1.0);
    assert_eq!(uv, [[0.0, 160.0 / 256.0], [50.0 / 128.0, 160.0 / 256.0], [50.0 / 128.0, 1.0], [0.0, 1.0]]);
    // Scaled sprites, and a depth texture that is part of a larger image.
    depth.x_start = 0.5;
    depth.y_start = 0.25;
    depth.width_hw = 256;
    depth.height_hw = 1024;
    depth.offset_x = 4.0;
    let mut m = TileDepthModifier { depth, soff_x: 0, soff_y: 0, scale_x: 1.0, scale_y: 1.0 };
    m.set_sprite_scale(0.5, 2.0);
    let uv = m.uv1(&tex0);
    // x: 15..45; y: 280..480 clipped to the depth texture's 0..256, which
    // leaves the top below the bottom, as in the original.
    assert_eq!(uv[0], [0.5 + 11.0 / 256.0, 0.25 + 280.0 / 1024.0]);
    assert_eq!(uv[2], [0.5 + 41.0 / 256.0, 0.25 + 256.0 / 1024.0]);
}

#[test]
fn wall_presets() {
    use WallDirection::*;
    let f = WallFlags::default();
    assert_eq!(wall_preset(N, f), TileDepthPreset::NWall);
    assert_eq!(wall_preset(N, WallFlags { door_wall_n: true, ..f }), TileDepthPreset::NDoorFrame);
    assert_eq!(wall_preset(N, WallFlags { door_wall_n: true, door_n: true, ..f }), TileDepthPreset::NWall);
    assert_eq!(wall_preset(NW, f), TileDepthPreset::NWWall);
    assert_eq!(wall_preset(W, f), TileDepthPreset::WWall);
    assert_eq!(wall_preset(W, WallFlags { door_wall_w: true, ..f }), TileDepthPreset::WDoorFrame);
    assert_eq!(wall_preset(W, WallFlags { door_wall_w: true, wall_se: true, ..f }), TileDepthPreset::SEWall);
    assert_eq!(wall_preset(SE, f), TileDepthPreset::SEWall);
    assert_eq!(wall_preset(Other, f), TileDepthPreset::Floor);
    assert_eq!(TileDepthPreset::SEWall.index(), 7);
}

#[test]
fn wall_modifier_prefers_sprite_depth() {
    let (game, _) = fixture("wallmod");
    let mut tm = TextureManager::new();
    let mut mgr = TileDepthTextureManager::init_from_dirs(&[], &game, &mut tm, &mut walls_only, false);
    let own = mgr.texture("walls_01", 1).unwrap().clone();
    let m = TileDepthModifier::wall(&mut mgr, &mut tm, Some(&own), WallDirection::N, WallFlags::default(), 3, 4);
    assert_eq!(&*m.depth.name, "DEPTH_walls_01_1");
    assert_eq!((m.soff_x, m.soff_y, m.scale_x, m.scale_y), (3, 4, 1.0, 1.0));
    let m = TileDepthModifier::wall(&mut mgr, &mut tm, None, WallDirection::N, WallFlags::default(), 0, 0);
    assert_eq!(&*m.depth.name, "DEPTH_preset_depthmaps_01_5");
    assert_eq!(&*TileDepthModifier::floor(&mgr, 0, 0).depth.name, "DEPTH_preset_depthmaps_01_0");
    assert_eq!(&*TileDepthModifier::tile(&mut mgr, &mut tm, &own, 0, 0).depth.name, "DEPTH_walls_01_1");
}

#[test]
fn assignments_parse() {
    let mut a = TileDepthTextureAssignments::new();
    a.parse("t\n{\n    VERSION = 1,\n    a_1 = b_2,\n    a_2 = a_2,\n    a_3 = ,\n    a_1 = c_3,\n    junk,\n}\n").unwrap();
    assert_eq!(a.assigned_tile_name("a_1"), Some("c_3"));
    assert_eq!(a.assigned_tile_name("a_2"), None);
    assert_eq!(a.assigned_tile_name("a_3"), None);
    // `VERSION` is read as an assignment too.
    assert_eq!(a.assigned_tile_name("VERSION"), Some("1"));
    assert_eq!(a.len(), 2);
    a.assign_tile_name("a_1", "  ");
    assert_eq!(a.assigned_tile_name("a_1"), None);
    assert!(TileDepthTextureAssignments::new().parse("t { x = y, }").is_err());
    assert!(TileDepthTextureAssignments::new().parse("t { VERSION = 2, }").is_err());
    assert!(TileDepthTextureAssignments::new().parse("").is_err());
    // A missing file is not an error.
    let dir = temp_dir("asg");
    assert!(TileDepthTextureAssignments::new().load(&dir).is_ok());
}

/// `PZ_GAME_DIR=... cargo test -p pz-render --release -- --ignored installed_game_depth`.
#[test]
#[ignore]
fn installed_game_depth_textures() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let mut fs = pz_fs::GameFs::new(PathBuf::from(dir));
    // Sprite names from every texture pack (what `getSharedTexture` finds
    // once the packs are loaded), without decoding the pages.
    let mut names = std::collections::HashSet::new();
    for e in std::fs::read_dir(fs.media_file("texturepacks")).unwrap().flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(pack) = file.strip_suffix(".pack") else { continue };
        if let Ok(dev) = pz_fs::pack::TexturePackDevice::open(pack, &e.path().to_string_lossy(), 0) {
            names.extend(dev.sub_textures().into_keys());
        }
    }
    let mut tm = TextureManager::new();
    let mgr = TileDepthTextureManager::init_with(&mut fs, &mut tm, &mut |n| names.contains(n));
    let files: Vec<String> = std::fs::read_dir(fs.media_file("depthmaps")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("DEPTH_")).collect();
    let game = mgr.mod_data("game").unwrap();
    let skipped: Vec<&str> = files.iter().filter_map(|f| f.strip_prefix("DEPTH_")?.strip_suffix(".png")).filter(|t| game.textures.tileset(t).is_none()).collect();
    eprintln!("{} DEPTH_ files; not loaded as tilesets: {skipped:?}", files.len());
    for d in &mgr.mod_data {
        let tilesets: Vec<_> = d.textures.tilesets().collect();
        let tiles: usize = tilesets.iter().map(|t| t.tiles().len()).sum();
        let filled: usize = tilesets.iter().map(|t| t.tiles().iter().filter(|t| !t.is_empty()).count()).sum();
        eprintln!("{}: {} tilesets, {tiles} tiles, {filled} with depth", d.mod_id, tilesets.len());
    }
    let presets = (0..8).filter(|&i| mgr.preset_depth_texture(i, 0).is_some_and(|t| !t.is_empty())).count();
    eprintln!(
        "presets with depth: {presets}/8; whole tile empty: {:?}; billboard empty: {:?}; errors: {:?}",
        mgr.default_depth_texture().map(|t| t.is_empty()),
        mgr.billboard_depth_texture().map(|t| t.is_empty()),
        mgr.errors
    );
    let asg = TileDepthTextureAssignmentManager::init(&mut fs);
    let (mut own, mut assigned) = (0, 0);
    for name in &names {
        if let Some(t) = mgr.sprite_depth_texture(name, true, &asg) {
            if t.name() == name {
                own += 1;
            } else {
                assigned += 1;
            }
        }
    }
    eprintln!(
        "assignments: {} (errors {:?}); pack sprites with depth: {own} own, {assigned} assigned",
        asg.mod_data("game").unwrap().assignments.len(),
        asg.errors
    );
    assert!(mgr.errors.is_empty());
}
