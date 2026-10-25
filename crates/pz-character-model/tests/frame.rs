//! A dressed player standing on a grass tile, drawn through the whole iso
//! frame (chunk texture, composite, the object pass with the player's
//! model and shadow, translucent loop, screen pass) on the GPU and saved
//! as a PNG for inspection.
//!
//! Reads the installed game at runtime: point `PZ_GAME_DIR` at the folder
//! holding `media/`; `PZ_CHARRENDER_FRAME_OUT` names the directory the
//! frames go to (default: the system temp directory). Without the game
//! (or a GPU adapter) this test does nothing.
//!
//! The body and clothing textures are the smart-texture jobs of
//! `ModelInstanceTextureCreator`, run by the GPU texture combiner. A second
//! frame shows the player with blood and dirt on the body and clothing, a
//! hole in the jacket and the jeans, and a denim patch; the combined body,
//! jacket and jeans textures are saved next to the frames.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::AnimationPlayer;
use pz_character_model::combine::run_jobs;
use pz_character_model::data::{
    BodyLocationGroup, ClothingItem, HumanModel, HumanVisual, ModelScriptRef, Population,
    ResolvedItemVisual,
};
use pz_character_model::items::EquippedModels;
use pz_character_model::lights::EffectLight;
use pz_character_model::mask::BloodBodyPartType;
use pz_character_model::mask::BLOOD_PARTS;
use pz_character_model::model_tree::TextureSource;
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot};
use pz_character_model::render::{CharacterFrame, ModelResources};
use pz_character_model::resources::{MediaModelResources, TextureLoader};
use pz_character_model::scene::{character_shadow, render_character_object, ShadowInput};
use pz_character_model::texture_creator::{create_textures, TextureCreatorInput};
use pz_iso_render::frame::{world_frame_with_objects, ChunkLevels, FrameSize};
use pz_iso_render::models::ModelCamera;
use pz_iso_render::objects::FrameObjects;
use pz_iso_render::{
    Camera, ChunkRenderer, ChunkTexture, ObjectKind, RenderObject, RenderSquare, SpriteFlags,
    SpriteInfo,
};
use pz_model::assets::{resolve_model_file, AssetKind, MediaIndex};
use pz_model::loader::{load_animation, load_mesh, MeshParams};
use pz_render::image::ImageData;
use pz_render::{flags, Texture, TextureManager};

/// A path under the system temp dir unique to this call (tag, process id and a
/// per-call counter): tests run in parallel and must not share directories.
fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{tag}-{}-{n}", std::process::id()))
}

fn game_dir() -> Option<PathBuf> {
    let game = std::env::var_os("PZ_GAME_DIR")
        .map(PathBuf::from)
        .filter(|g| g.join("media").is_dir());
    if game.is_none() {
        eprintln!("PZ_GAME_DIR not set or has no media/; skipping");
    }
    game
}

/// An RGBA image laid out power-of-two wide, as the texture manager keeps
/// it, with its real size.
#[derive(Clone)]
struct Rgba {
    w: i32,
    h: i32,
    img: ImageData,
}

fn load_png(path: &Path) -> Option<Rgba> {
    let f = std::fs::File::open(path).ok()?;
    let img = ImageData::from_png(std::io::BufReader::new(f), false).ok()?;
    Some(Rgba {
        w: img.width,
        h: img.height,
        img,
    })
}

/// Texture files and smart-texture results for the model resources.
struct Loader {
    tm: RefCell<TextureManager>,
    files: RefCell<HashMap<PathBuf, Option<Texture>>>,
    smart: RefCell<Vec<Option<Texture>>>,
}

impl TextureLoader for Loader {
    fn file(&self, path: &Path) -> Option<Texture> {
        if let Some(t) = self.files.borrow().get(path) {
            return t.clone();
        }
        let t = load_png(path).map(|p| {
            let name = path.to_string_lossy();
            self.tm
                .borrow_mut()
                .texture_from_pixels(&name, p.w, p.h, p.img.data.pixels, 0)
        });
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), t.clone());
        t
    }

    fn smart(&self, job: usize) -> Option<Texture> {
        self.smart.borrow().get(job).cloned().flatten()
    }
}

fn clothing(game: &Path, name: &str) -> ResolvedItemVisual {
    let xml =
        std::fs::read_to_string(game.join(format!("media/clothing/clothingItems/{name}.xml")))
            .unwrap();
    let item = ClothingItem::parse(name, "", xml.trim_start_matches('\u{feff}')).unwrap();
    let mut v = ResolvedItemVisual::new(&format!("Base.{name}"), Some(Rc::new(item)));
    v.visual.texture_choice = 0;
    v.visual.base_texture = 0;
    v.visual.tint = Some([1.0, 1.0, 1.0]);
    v.visual.hue = 0.0;
    v
}

fn write_png(path: &Path, w: u32, h: u32, data: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(data).unwrap();
}

#[test]
fn dressed_player_on_a_tile() {
    let Some(game) = game_dir() else { return };
    player_frame(&game, false);
}

#[test]
fn bloodied_player_on_a_tile() {
    let Some(game) = game_dir() else { return };
    player_frame(&game, true);
}

/// Blood, dirt, holes and patches (`ItemVisual`/`HumanVisual` levels).
fn damage(hv: &mut HumanVisual, visuals: &mut [ResolvedItemVisual]) {
    use BloodBodyPartType as B;
    let n = BLOOD_PARTS;
    for (p, b, d) in [
        (B::Head, 0.8, 0.0),
        (B::HandL, 1.0, 0.6),
        (B::HandR, 0.5, 0.9),
        (B::Neck, 0.6, 0.3),
    ] {
        hv.set_blood(p.index(), b);
        hv.set_dirt(p.index(), d);
    }
    for v in visuals.iter_mut() {
        let name = v.visual.full_type.clone();
        let vis = &mut v.visual;
        match name.as_str() {
            "Base.Jacket_Padded" => {
                vis.set_blood(B::TorsoUpper.index(), 1.0, n);
                vis.set_blood(B::UpperArmL.index(), 0.7, n);
                vis.set_dirt(B::ForeArmR.index(), 1.0, n);
                vis.set_dirt(B::TorsoLower.index(), 0.6, n);
                vis.set_hole(B::TorsoLower.index(), n);
                vis.set_basic_patch(B::UpperArmR.index(), n);
            }
            "Base.Trousers_Denim" => {
                vis.set_hole(B::UpperLegL.index(), n);
                vis.set_denim_patch(B::UpperLegR.index(), n);
                vis.set_blood(B::LowerLegL.index(), 0.9, n);
                vis.set_dirt(B::LowerLegR.index(), 1.0, n);
            }
            "Base.Tshirt_DefaultTEXTURE" => vis.set_blood(B::TorsoUpper.index(), 0.5, n),
            _ => {}
        }
    }
}

fn player_frame(game: &Path, damaged: bool) {
    let game = game.to_path_buf();
    let (w, h) = (640u32, 480u32);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(w, h) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let out_dir = std::env::var_os("PZ_CHARRENDER_FRAME_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cache = unique_temp_dir("pz-character-model-frame-test");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(&cache);
    let media = MediaIndex::scan(&game).unwrap();

    // The body's skeleton with an idle clip.
    let (mesh_path, mesh_ty) =
        resolve_model_file(&media, "Skinned/MaleBody", AssetKind::Mesh).unwrap();
    let body = load_mesh(
        &std::fs::read(mesh_path).unwrap(),
        mesh_ty.import_format().unwrap(),
        &MeshParams::default(),
    )
    .unwrap();
    let mut sd = body.skeleton.clone().unwrap();
    let (anim_path, anim_ty) =
        resolve_model_file(&media, "Bob/Bob_Idle", AssetKind::Animation).unwrap();
    let clips = load_animation(
        &std::fs::read(anim_path).unwrap(),
        anim_ty.import_format().unwrap(),
        Some(&sd),
    )
    .unwrap();
    let clip = clips.keys().next().unwrap().clone();
    sd.animation_clips.extend(clips);
    let mut player = AnimationPlayer::new(Rc::new(sd));
    // Facing south-east (towards the viewer's lower right).
    player.set_target_and_current_direction(0.3, 1.0);
    let t = player.play(&clip, true, false, -1.0).unwrap();
    player.track_mut(t).unwrap().set_blend_weight(1.0);
    player.update(0.5, None);

    // A dressed man: hair, T-shirt and shoes (textures on the body),
    // jeans and a padded jacket (skinned meshes), a cap (static, on the
    // head).
    let mut pop = Population::default();
    pop.init(&mut fs);
    let mut hv = HumanVisual::new(BLOOD_PARTS);
    hv.skin_texture_name = Some("MaleBody01a".into());
    hv.hair_model = Some("CrewCut".into());
    hv.beard_model = Some("Chops".into());
    hv.hair_color = Some([0.35, 0.22, 0.12]);
    hv.beard_color = Some([0.35, 0.22, 0.12]);
    let mut visuals: Vec<ResolvedItemVisual> = [
        "Tshirt_DefaultTEXTURE",
        "Shoes_Black",
        "Trousers_Denim",
        "Jacket_Padded",
        "Hat_BaseballCap",
    ]
    .iter()
    .map(|n| clothing(&game, n))
    .collect();
    if damaged {
        damage(&mut hv, &mut visuals);
    }
    let human = HumanModel::resolve(&mut hv, false, false, false, &pop.styles(&[], 0.0), &[]);
    let groups = BodyLocationGroup::new("Human").unwrap();
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let bone_index = |n: &str| player.skinning_bone_index(n, -1);
    let mut slot = ModelSlot::build(&CharacterModelInput {
        human: &human,
        item_visuals: &visuals,
        equipped: &EquippedModels::default(),
        body_locations: &groups,
        hair_styles: pop.hair_styles.as_ref(),
        beard_styles: pop.beard_styles.as_ref(),
        scripts: &scripts,
        body_model_script: None,
        bone_index: &bone_index,
    });
    let jobs = create_textures(
        &mut slot,
        &TextureCreatorInput {
            human: &human,
            item_visuals: &visuals,
            body_locations: &groups,
            decals: &|_| None,
            simple_clothing_textures: false,
        },
    );
    for inst in &slot.instances {
        eprintln!(
            "instance {:?} {} {:?} {:?}",
            inst.kind, inst.mesh_name, inst.tex, inst.model_texture
        );
    }

    // TextureCombiner on the GPU; files with ModelManager.getTextureFlags.
    let mut tm = TextureManager::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
    let smart = run_jobs(&jobs.jobs, &mut file, &mut gpu, &mut tm);
    let prefix = if damaged {
        "charrender_damaged"
    } else {
        "charrender_frame"
    };
    for inst in &slot.instances {
        let TextureSource::Smart(job) = inst.tex else {
            continue;
        };
        let Some(t) = &smart[job] else { continue };
        let px = gpu.read_combined(t).expect("combined texture");
        let name = inst.mesh_name.rsplit(['/', '\\']).next().unwrap_or("tex");
        let path = out_dir.join(format!("{prefix}_tex_{name}.png"));
        write_png(&path, t.width, t.height, &px);
        eprintln!("wrote {}", path.display());
    }

    // The tiles: a 6x6 grass patch in chunk (1, 1).
    tm.load_texture_pack(&fs, "Tiles2x.floor", 0, None);
    tm.set_texture_pack_lookup(&[]);
    let grass = tm
        .try_get_texture(&fs, "blends_natural_01_16")
        .expect("grass tile");
    let depth_maps = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let floor = Rc::new(SpriteInfo {
        texture: Some(grass),
        flags: SpriteFlags {
            solidfloor: true,
            ..Default::default()
        },
        tint_mod: [1.0; 3],
        ..Default::default()
    });
    let squares: Vec<RenderSquare> = (8..14)
        .flat_map(|y| (8..14).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects
                .push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
            sq
        })
        .collect();
    let mut level = [None; 64];
    for sq in &squares {
        level[((sq.x - 8) + (sq.y - 8) * 8) as usize] = Some(sq);
    }
    let shadow_tex = tm.shared_texture(&fs, "media/textures/NewShadow.png");
    assert!(shadow_tex.is_some(), "NewShadow.png");

    let loader = Loader {
        tm: RefCell::new(tm),
        files: RefCell::new(HashMap::new()),
        smart: RefCell::new(smart),
    };
    let res = MediaModelResources::new(&media, &loader);
    for inst in &slot.instances {
        assert!(
            res.mesh(&inst.mesh_name, inst.is_static).is_some(),
            "mesh {}",
            inst.mesh_name
        );
    }

    let (px, py, pz) = (10.5f32, 10.5f32, 0.0f32);
    for (zoom, name) in [
        (1.0f32, format!("{prefix}_zoom1.png")),
        (0.5, format!("{prefix}_zoom05.png")),
    ] {
        let cam = Camera::player(px, py, pz, w as i32, h as i32, zoom);
        let mc = ModelCamera::new(&cam, w as f32 * zoom, h as f32 * zoom);
        // Fill lights only, from an ambient of 0.8 (outside, by day).
        let amb = 0.8f32;
        let mut lights = [EffectLight::default(); 5];
        let fill = amb * 1.7 / 4.0;
        lights[3].set(px - 2.0, py - 2.0, pz + 1.0, fill, fill, fill, 5000);
        lights[4].set(px + 2.0, py + 2.0, pz + 1.0, fill, fill, fill, 5000);
        let frame = CharacterFrame {
            player: &player,
            x: px,
            y: py,
            z: pz,
            alpha: 1.0,
            effect_lights: &lights,
            ambient: [amb; 3],
            final_scale: 1.0,
        };
        let shadow = character_shadow(
            &ShadowInput {
                player: Some(&player),
                anim_angle: player.angle(),
                ragdoll_direction: None,
                height_above_floor: 0.0,
                alpha: 1.0,
                skeleton: false,
                animal_size: None,
                light: [1.0; 3],
                slope: None,
                hidden: false,
            },
            [px, py, pz],
            None,
        );
        let object = render_character_object(&slot, &frame, shadow, &mc, &res);
        assert!(object.draws.len() >= 5, "{} draws", object.draws.len());
        let objects = FrameObjects {
            players: vec![object],
            ..Default::default()
        };
        let r = ChunkRenderer::new(cam, &depth_maps, (w, h));
        let chunks = [ChunkLevels {
            texture: ChunkTexture::new(1, 1, 0, 0, 0),
            levels: vec![(0, level)],
            dirty: true,
        }];
        let cmds = world_frame_with_objects(
            &r,
            FrameSize {
                width: w,
                height: h,
            },
            &chunks,
            &objects,
            shadow_tex.clone(),
            pz_render::ScreenParams {
                desaturation: 0.0,
                timer: 0.0,
            },
            Vec::new(),
        );
        let list = pz_render::DrawList { cmds };
        gpu.render_frame(
            &[(pz_gpu::FrameParams::ui(w, h), &list)],
            &mut loader.tm.borrow_mut(),
        );
        let pixels = gpu.read_pixels();
        let path = out_dir.join(&name);
        write_png(&path, w, h, &pixels);
        eprintln!("wrote {}", path.display());
        // The grey jacket shows in the screen's centre column, grass at
        // the side, and the shadow darkens the grass at the feet.
        let at = |x: u32, y: u32| -> [i32; 3] {
            std::array::from_fn(|k| pixels[((y * w + x) * 4) as usize + k] as i32)
        };
        let jacket = (0..h).filter(|&y| {
            let p = at(w / 2, y);
            p.iter().all(|&c| c > 90) && (p[0] - p[2]).abs() < 30
        });
        assert!(
            jacket.count() > (20.0 / zoom) as usize,
            "no jacket in the centre column"
        );
        let grass = at(w / 2 + (120.0 / zoom) as u32, h / 2);
        assert!(grass[1] > grass[2] + 20, "grass is green: {grass:?}");
        // Mean green of a patch beside the feet, and of the same patch two
        // squares to the north-east (no shadow there).
        let patch = |cx: u32, cy: u32| -> i32 {
            let r = (6.0 / zoom) as i32;
            let mut sum = 0;
            for dy in -r..=r {
                for dx in -r..=r {
                    sum += at((cx as i32 + dx) as u32, (cy as i32 + dy) as u32)[1];
                }
            }
            sum / ((2 * r + 1) * (2 * r + 1))
        };
        let (fx, fy) = (w / 2 + (28.0 / zoom) as u32, h / 2 + (50.0 / zoom) as u32);
        let shade = patch(fx, fy);
        let lit = patch(fx + (128.0 / zoom) as u32, fy);
        assert!(shade < lit - 15, "shadow by the feet: {shade} vs {lit}");
    }
}

/// One `model <name> { ... }` block of a script file, loaded by
/// pz-scripts' `ModelScript.Load`.
fn model_script(
    sm: &pz_scripts::ScriptManager,
    path: &Path,
    name: &str,
) -> pz_character_model::ModelScriptRef {
    let t = std::fs::read_to_string(path).unwrap();
    let start = t
        .find(&format!("    model {name}\n"))
        .or_else(|| t.find(&format!("    model {name}\r\n")))
        .unwrap();
    let open = start + t[start..].find('{').unwrap();
    let mut depth = 0;
    let mut end = open;
    for (i, c) in t[open..].char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = open + i;
                break;
            }
        }
    }
    use pz_scripts::ScriptObject;
    let mut m = pz_character_model::ModelScript::new();
    m.init_load_pp(sm, name);
    m.load(sm, name, &t[start..=end]).unwrap();
    Rc::new(RefCell::new(m))
}

/// The player with a dead zombie and placed items beside it: the corpse
/// through `DeadBodyAtlas` (bitmap render, atlas copy, depth-texture draw
/// and its shadow), a mug and a hammer through `WorldItemAtlas`, and a
/// magazine without a model as a 2D sprite. Frames go to
/// `PZ_ATLAS_FRAME_OUT` (default: the system temp directory).
#[test]
fn corpse_and_floor_items() {
    use pz_character_model::corpse::{body_key, corpse_shadow, corpse_world_draw, pose, BodyVisualKey, DeadBody, DeadBodyAtlas};
    use pz_character_model::world_item::item_params;
    use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
    use pz_iso_render::item_atlas::{item_sprite_draw, item_world_draw, max_zoom_is_one, ItemPlacement, WorldItemAtlas};

    let Some(game) = game_dir() else { return };
    let (w, h) = (640u32, 480u32);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(w, h) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let out_dir = std::env::var_os("PZ_ATLAS_FRAME_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cache = unique_temp_dir("pz-character-model-frame-test");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(&cache);
    let media = MediaIndex::scan(&game).unwrap();

    let (mesh_path, mesh_ty) =
        resolve_model_file(&media, "Skinned/MaleBody", AssetKind::Mesh).unwrap();
    let body = load_mesh(
        &std::fs::read(mesh_path).unwrap(),
        mesh_ty.import_format().unwrap(),
        &MeshParams::default(),
    )
    .unwrap();
    let mut sd = body.skeleton.clone().unwrap();
    for anim in ["Bob/Bob_Idle", "Zombie/Zombie_Idle_FloorOnBack", "Zombie/Zombie_Idle_FloorOnFront"] {
        let (p, ty) = resolve_model_file(&media, anim, AssetKind::Animation).unwrap();
        let clips =
            load_animation(&std::fs::read(p).unwrap(), ty.import_format().unwrap(), Some(&sd)).unwrap();
        sd.animation_clips.extend(clips);
    }
    let sd = Rc::new(sd);
    let mut player = AnimationPlayer::new(sd.clone());
    player.set_target_and_current_direction(0.3, 1.0);
    let t = player.play("Bob_Idle", true, false, -1.0).unwrap();
    player.track_mut(t).unwrap().set_blend_weight(1.0);
    player.update(0.5, None);

    let mut pop = Population::default();
    pop.init(&mut fs);
    let groups = BodyLocationGroup::new("Human").unwrap();
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let mut tm = TextureManager::new();
    let mut smart: Vec<Option<Texture>> = Vec::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    let mut slot_for = |hair: &str, skin: &str, clothes: &[&str], player: &AnimationPlayer, tm: &mut TextureManager, gpu: &mut pz_gpu::Gpu| {
        let mut hv = HumanVisual::new(BLOOD_PARTS);
        hv.skin_texture_name = Some(skin.into());
        hv.hair_model = Some(hair.into());
        hv.hair_color = Some([0.35, 0.22, 0.12]);
        let visuals: Vec<ResolvedItemVisual> = clothes.iter().map(|n| clothing(&game, n)).collect();
        let human = HumanModel::resolve(&mut hv, false, false, false, &pop.styles(&[], 0.0), &[]);
        let bone_index = |n: &str| player.skinning_bone_index(n, -1);
        let mut slot = ModelSlot::build(&CharacterModelInput {
            human: &human,
            item_visuals: &visuals,
            equipped: &EquippedModels::default(),
            body_locations: &groups,
            hair_styles: pop.hair_styles.as_ref(),
            beard_styles: pop.beard_styles.as_ref(),
            scripts: &scripts,
            body_model_script: None,
            bone_index: &bone_index,
        });
        let jobs = create_textures(
            &mut slot,
            &TextureCreatorInput {
                human: &human,
                item_visuals: &visuals,
                body_locations: &groups,
                decals: &|_| None,
                simple_clothing_textures: false,
            },
        );
        // TextureCombiner on the GPU; the jobs index into one shared list.
        let base = smart.len();
        let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
        smart.extend(run_jobs(&jobs.jobs, &mut file, gpu, tm));
        for inst in &mut slot.instances {
            if let pz_character_model::TextureSource::Smart(j) = &mut inst.tex {
                *j += base;
            }
        }
        slot
    };
    let slot = slot_for("CrewCut", "MaleBody01a", &["Tshirt_DefaultTEXTURE", "Shoes_Black", "Trousers_Denim", "Jacket_Padded"], &player, &mut tm, &mut gpu);

    // The dead zombie: the zombie set's onground state on its back.
    let dead = DeadBody {
        x: 11.9,
        y: 10.2,
        z: 0.0,
        angle: 2.2,
        dir: 3,
        zombie: true,
        outside: true,
        alpha: 1.0,
        animal_size: 1.0,
        ..Default::default()
    };
    let mut corpse_player = AnimationPlayer::new(sd.clone());
    let zombie_set = Rc::new(pz_anim::AnimationSet::load_states("zombie", &game.join("media/AnimSets/zombie"), &["onground"]));
    pose(&mut corpse_player, zombie_set, &dead);
    assert_eq!(corpse_player.tracks()[0].name(), "Zombie_Idle_FloorOnBack");
    let corpse_slot = slot_for("Bob", "MaleBody02a", &["Tshirt_DefaultTEXTURE", "Trousers_Denim"], &corpse_player, &mut tm, &mut gpu);

    // Placed items.
    let sm = pz_scripts::ScriptManager::new();
    let items_txt = game.join("media/scripts/generated/models_items.txt");
    let weapons_txt = game.join("media/scripts/generated/models_weapons.txt");
    let item_scripts: HashMap<String, ModelScriptRef> = [
        ("Mug".to_string(), model_script(&sm, &items_txt, "Mug")),
        ("Hammer".to_string(), model_script(&sm, &weapons_txt, "Hammer")),
    ]
    .into();
    let mut mug_t = ItemTemplate::new(ItemClass::Normal, "Base", "Mugl");
    mug_t.world_static_model = Some("Mug".into());
    let mug = InventoryItem::create(Rc::new(mug_t), 0);
    let mut hammer_t = ItemTemplate::new(ItemClass::Weapon, "Base", "Hammer");
    hammer_t.weapon_sprite = Some("Hammer".into());
    let mut hammer = InventoryItem::create(Rc::new(hammer_t), 0);
    hammer.world_z_rotation = 60.0;

    tm.load_texture_pack(&fs, "Tiles2x.floor", 0, None);
    tm.set_texture_pack_lookup(&[]);
    let grass = tm.try_get_texture(&fs, "blends_natural_01_16").expect("grass tile");
    let depth_maps = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let floor = Rc::new(SpriteInfo {
        texture: Some(grass),
        flags: SpriteFlags { solidfloor: true, ..Default::default() },
        tint_mod: [1.0; 3],
        ..Default::default()
    });
    let squares: Vec<RenderSquare> = (8..14)
        .flat_map(|y| (8..14).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
            sq
        })
        .collect();
    let mut level = [None; 64];
    for sq in &squares {
        level[((sq.x - 8) + (sq.y - 8) * 8) as usize] = Some(sq);
    }
    let shadow_tex = tm.shared_texture(&fs, "media/textures/NewShadow.png");
    let clip_tex = tm.shared_texture(&fs, "media/inventory/BerettaClip.png").expect("BerettaClip.png");

    let loader = Loader { tm: RefCell::new(tm), files: RefCell::new(HashMap::new()), smart: RefCell::new(smart) };
    let res = MediaModelResources::new(&media, &loader);
    let mut bodies = DeadBodyAtlas::new(1 << 40);
    let mut item_atlas = WorldItemAtlas::new(1 << 41);

    let (px, py, pz) = (10.5f32, 10.5f32, 0.0f32);
    for (zoom, name) in [(1.0f32, "atlas_frame_zoom1.png"), (0.5, "atlas_frame_zoom05.png")] {
        let cam = Camera::player(px, py, pz, w as i32, h as i32, zoom);
        let mc = ModelCamera::new(&cam, w as f32 * zoom, h as f32 * zoom);
        let amb = 0.8f32;
        let mut lights = [EffectLight::default(); 5];
        let fill = amb * 1.7 / 4.0;
        lights[3].set(px - 2.0, py - 2.0, pz + 1.0, fill, fill, fill, 5000);
        lights[4].set(px + 2.0, py + 2.0, pz + 1.0, fill, fill, fill, 5000);
        let frame = CharacterFrame { player: &player, x: px, y: py, z: pz, alpha: 1.0, effect_lights: &lights, ambient: [amb; 3], final_scale: 1.0 };
        let shadow = character_shadow(
            &ShadowInput {
                player: Some(&player),
                anim_angle: player.angle(),
                ragdoll_direction: None,
                height_above_floor: 0.0,
                alpha: 1.0,
                skeleton: false,
                animal_size: None,
                light: [1.0; 3],
                slope: None,
                hidden: false,
            },
            [px, py, pz],
            None,
        );
        let mut pre = Vec::new();
        // The corpse: atlas job (first frame only), then the atlas draw.
        let key = body_key(&dead, &BodyVisualKey::default());
        let body_tex = bodies.body_texture(&key, &dead, &corpse_slot, &corpse_player, &res, &mut pre).clone();
        let e = body_tex.entry.unwrap();
        eprintln!("corpse entry {e:?} shadow {:?}", body_tex.shadow);
        let light = [amb, amb, amb, 1.0];
        let corpses = corpse_world_draw(&body_tex, &cam, &dead, light);
        let corpse_shadows = vec![corpse_shadow(&dead, Some(&body_tex), [1.0; 3])];
        // Items: their atlas jobs, then the depth-texture draws.
        let mz1 = max_zoom_is_one(true, zoom);
        let mut items = Vec::new();
        for (item, x, y) in [(&mug, 9.7f32, 11.3f32), (&hammer, 10.3, 11.8)] {
            let p = item_params(item, &item_scripts, &res, mz1).expect("item model");
            let k = item_atlas.item_texture(&p);
            item_atlas.render(&mut pre);
            let t = &item_atlas.textures[&k];
            eprintln!("item entry {:?} too big {}", t.entry, t.too_big);
            items.extend(item_world_draw(t, &cam, &ItemPlacement { x, y, z: 0.0, carpet: false, rgb: [amb; 3], alpha: 1.0 }));
        }
        // No model: the 2D sprite.
        items.extend(item_sprite_draw(&cam, &clip_tex, 1.0, 9.3, 10.3, 0.0, [amb; 3], [1.0; 3], 1.0));
        assert!(items.iter().filter(|c| matches!(c, pz_render::DrawCmd::AtlasQuad { .. })).count() == 2);

        let object = render_character_object(&slot, &frame, shadow, &mc, &res);
        let objects = FrameObjects { players: vec![object], corpse_shadows, corpses, items, ..Default::default() };
        let r = ChunkRenderer::new(cam, &depth_maps, (w, h));
        let chunks = [ChunkLevels { texture: ChunkTexture::new(1, 1, 0, 0, 0), levels: vec![(0, level)], dirty: true }];
        let cmds = world_frame_with_objects(
            &r,
            FrameSize { width: w, height: h },
            &chunks,
            &objects,
            shadow_tex.clone(),
            pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 },
            pre,
        );
        let list = pz_render::DrawList { cmds };
        gpu.render_frame(&[(pz_gpu::FrameParams::ui(w, h), &list)], &mut loader.tm.borrow_mut());
        let pixels = gpu.read_pixels();
        let path = out_dir.join(name);
        write_png(&path, w, h, &pixels);
        eprintln!("wrote {}", path.display());
        // The corpse covers grass around its screen position: something
        // not green there.
        let at = |x: f32, y: f32| -> [i32; 3] {
            let (x, y) = (x as u32, y as u32);
            std::array::from_fn(|k| pixels[((y * w + x) * 4) as usize + k] as i32)
        };
        let to_screen = |x: f32, y: f32| {
            ((pz_render::iso::x_to_screen(x, y) - cam.off_x as f32) / zoom, (pz_render::iso::y_to_screen(x, y, 0.0, 0) - cam.off_y as f32) / zoom)
        };
        let (cx, cy) = to_screen(dead.x, dead.y);
        let mut non_grass = 0;
        for dy in -6..=6 {
            for dx in -6..=6 {
                let p = at(cx + dx as f32 * 2.0 / zoom, cy + dy as f32 * 2.0 / zoom);
                if !(p[1] > p[0] + 10 && p[1] > p[2] + 10) {
                    non_grass += 1;
                }
            }
        }
        assert!(non_grass > 40, "corpse at ({cx}, {cy}): {non_grass} non-grass samples");
    }
}

/// Fluids by name only (the test's fluid has its own colour).
struct NoFluids;

impl pz_entity::EntityContext for NoFluids {
    fn attribute_kind(&self, _id: i16) -> Option<pz_entity::AttributeKind> {
        None
    }
}

/// `ModelOutlines` and the item smart textures in one scene: the player
/// holding a bloody hunting knife (its `EquippedTextureCreator` texture),
/// a zombie behind it outlined red (the bad-highlight colour; the player
/// masks the ring where it overlaps) and one in front outlined cyan at
/// 0.65 (the melee debug outline), and a green mug with red liquid on the
/// floor (the `_Fluid` model through the `WorldItemAtlas` with its
/// `FLUIDTINT` overlay). A last frame outlines the player itself in green.
/// Frames go to `PZ_OUTLINE_FRAME_OUT` (default: the system temp
/// directory), with the knife's and mug's combined textures.
#[test]
fn outlines_bloody_knife_and_mug_with_liquid() {
    use pz_character_model::item_texture::render_equipped_textures_with;
    use pz_character_model::items::HeldItem;
    use pz_character_model::world_item::{atlas_smart_texture, item_params_with_fluid};
    use pz_entity::{FluidContainer, FluidInstance, FluidRef};
    use pz_inventory::{InventoryItem, ItemClass, ItemKind, ItemTemplate};
    use pz_iso_render::item_atlas::{item_world_draw, max_zoom_is_one, ItemPlacement, WorldItemAtlas};
    use pz_iso_render::outlines::OutlineHighlight;

    let Some(game) = game_dir() else { return };
    let (w, h) = (640u32, 480u32);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(w, h) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let out_dir = std::env::var_os("PZ_OUTLINE_FRAME_OUT").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let cache = unique_temp_dir("pz-character-model-frame-test");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(&cache);
    let media = MediaIndex::scan(&game).unwrap();

    let (mesh_path, mesh_ty) = resolve_model_file(&media, "Skinned/MaleBody", AssetKind::Mesh).unwrap();
    let body = load_mesh(&std::fs::read(mesh_path).unwrap(), mesh_ty.import_format().unwrap(), &MeshParams::default()).unwrap();
    let mut sd = body.skeleton.clone().unwrap();
    for anim in ["Bob/Bob_Idle", "Zombie/Zombie_Idle"] {
        let (p, ty) = resolve_model_file(&media, anim, AssetKind::Animation).unwrap();
        let clips = load_animation(&std::fs::read(p).unwrap(), ty.import_format().unwrap(), Some(&sd)).unwrap();
        sd.animation_clips.extend(clips);
    }
    let sd = Rc::new(sd);
    let posed = |clip: &str, dir: (f32, f32), t: f32| {
        let mut p = AnimationPlayer::new(sd.clone());
        p.set_target_and_current_direction(dir.0, dir.1);
        let tr = p.play(clip, true, false, -1.0).unwrap();
        p.track_mut(tr).unwrap().set_blend_weight(1.0);
        p.update(t, None);
        p
    };
    let player = posed("Bob_Idle", (0.3, 1.0), 0.5);
    let zombie_a = posed("Zombie_Idle", (0.6, 0.8), 0.3);
    let zombie_b = posed("Zombie_Idle", (-0.7, -0.7), 0.8);

    // Scripts: the knife and the mug (with its fluid variant).
    let sm = pz_scripts::ScriptManager::new();
    let weapons_txt = game.join("media/scripts/generated/models_weapons.txt");
    let items_txt = game.join("media/scripts/generated/models_items.txt");
    let scripts: HashMap<String, ModelScriptRef> = [
        ("HuntingKnife".to_string(), model_script(&sm, &weapons_txt, "HuntingKnife")),
        ("Mug".to_string(), model_script(&sm, &items_txt, "Mug")),
        ("Mug_Fluid".to_string(), model_script(&sm, &items_txt, "Mug_Fluid")),
    ]
    .into();
    let mut knife_t = ItemTemplate::new(ItemClass::Weapon, "Base", "HuntingKnife");
    knife_t.weapon_sprite = Some("HuntingKnife".into());
    let mut knife = InventoryItem::create(Rc::new(knife_t), 0);
    if let ItemKind::Weapon(wd) = &mut knife.kind {
        wd.blood_level = 0.8;
    }
    let mut mug_t = ItemTemplate::new(ItemClass::Normal, "Base", "Mugl");
    mug_t.world_static_model = Some("Mug".into());
    let mug = InventoryItem::create(Rc::new(mug_t), 0);
    let mut fluid = FluidContainer::default();
    let mut fi = FluidInstance::new(FluidRef::Modded("TestJuice".into()), 0.8);
    fi.custom_color = Some([0.85, 0.12, 0.1]);
    fluid.fluids.push(fi);
    fluid.recalculate_caches(true);

    let mut pop = Population::default();
    pop.init(&mut fs);
    let groups = BodyLocationGroup::new("Human").unwrap();
    let mut tm = TextureManager::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    let mut smart: Vec<Option<Texture>> = Vec::new();
    let exists = |n: &str| game.join(n).is_file();
    let mut build = |skin: &str, hair: &str, clothes: &[&str], anim: &AnimationPlayer, equipped: &EquippedModels, tm: &mut TextureManager, gpu: &mut pz_gpu::Gpu| {
        let mut hv = HumanVisual::new(BLOOD_PARTS);
        hv.skin_texture_name = Some(skin.into());
        hv.hair_model = Some(hair.into());
        hv.hair_color = Some([0.35, 0.22, 0.12]);
        let visuals: Vec<ResolvedItemVisual> = clothes.iter().map(|n| clothing(&game, n)).collect();
        let human = HumanModel::resolve(&mut hv, false, false, false, &pop.styles(&[], 0.0), &[]);
        let bone_index = |n: &str| anim.skinning_bone_index(n, -1);
        let mut slot = ModelSlot::build(&CharacterModelInput {
            human: &human,
            item_visuals: &visuals,
            equipped,
            body_locations: &groups,
            hair_styles: pop.hair_styles.as_ref(),
            beard_styles: pop.beard_styles.as_ref(),
            scripts: &scripts,
            body_model_script: None,
            bone_index: &bone_index,
        });
        let jobs = create_textures(&mut slot, &TextureCreatorInput { human: &human, item_visuals: &visuals, body_locations: &groups, decals: &|_| None, simple_clothing_textures: false });
        let base = smart.len();
        let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
        smart.extend(run_jobs(&jobs.jobs, &mut file, gpu, tm));
        for inst in &mut slot.instances {
            if let TextureSource::Smart(j) = &mut inst.tex {
                *j += base;
            }
        }
        // ModelInstanceTextureInitializer: the held items' textures.
        render_equipped_textures_with(&mut slot, &exists, &mut file, gpu, tm);
        slot
    };
    let equipped = EquippedModels { primary: Some(HeldItem::new(&knife)), ..Default::default() };
    let player_slot = build("MaleBody01a", "CrewCut", &["Tshirt_DefaultTEXTURE", "Shoes_Black", "Trousers_Denim"], &player, &equipped, &mut tm, &mut gpu);
    let zombie_a_slot = build("M_ZedBody01_level1", "Bob", &["Tshirt_DefaultTEXTURE", "Trousers_Denim"], &zombie_a, &EquippedModels::default(), &mut tm, &mut gpu);
    let zombie_b_slot = build("M_ZedBody01_level1", "Bob", &["Shoes_Black", "Trousers_Denim"], &zombie_b, &EquippedModels::default(), &mut tm, &mut gpu);
    let knife_inst = player_slot.instances.iter().find(|i| i.texture_init.is_some()).expect("knife instance");
    let TextureSource::Combined(knife_tex) = &knife_inst.tex else { panic!("knife texture {:?}", knife_inst.tex) };
    let px = gpu.read_combined(knife_tex).expect("knife texture");
    write_png(&out_dir.join("outline_tex_knife.png"), knife_tex.width, knife_tex.height, &px);

    tm.load_texture_pack(&fs, "Tiles2x.floor", 0, None);
    tm.set_texture_pack_lookup(&[]);
    let grass = tm.try_get_texture(&fs, "blends_natural_01_16").expect("grass tile");
    let depth_maps = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let floor = Rc::new(SpriteInfo { texture: Some(grass), flags: SpriteFlags { solidfloor: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
    let squares: Vec<RenderSquare> = (8..14)
        .flat_map(|y| (8..14).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
            sq
        })
        .collect();
    let mut level = [None; 64];
    for sq in &squares {
        level[((sq.x - 8) + (sq.y - 8) * 8) as usize] = Some(sq);
    }
    let shadow_tex = tm.shared_texture(&fs, "media/textures/NewShadow.png");

    let loader = Loader { tm: RefCell::new(tm), files: RefCell::new(HashMap::new()), smart: RefCell::new(smart) };
    let res = MediaModelResources::new(&media, &loader);
    let mut item_atlas = WorldItemAtlas::new(1 << 41);
    let mut red = OutlineHighlight::default();
    red.set(0, true);
    red.set_col(0, 1.0, 0.0, 0.0, 1.0);
    let mut cyan = OutlineHighlight::default();
    cyan.set(0, true);
    cyan.set_col(0, 0.0, 1.0, 1.0, 0.65);
    let mut green = OutlineHighlight::default();
    green.set(0, true);
    green.set_col(0, 0.2, 1.0, 0.2, 1.0);

    let (px, py, pz) = (10.5f32, 10.5f32, 0.0f32);
    let (ax, ay) = (10.0f32, 9.7f32);
    let (bx, by) = (11.4f32, 11.2f32);
    let (mx, my) = (9.8f32, 11.4f32);
    for (zoom, name, outline_player) in [(1.0f32, "outline_frame_zoom1.png", false), (0.5, "outline_frame_zoom05.png", false), (1.0, "outline_frame_player.png", true)] {
        let cam = Camera::player(px, py, pz, w as i32, h as i32, zoom);
        let mc = ModelCamera::new(&cam, w as f32 * zoom, h as f32 * zoom);
        let amb = 0.8f32;
        let object = |slot: &ModelSlot, anim: &AnimationPlayer, x: f32, y: f32| {
            let mut lights = [EffectLight::default(); 5];
            let fill = amb * 1.7 / 4.0;
            lights[3].set(x - 2.0, y - 2.0, pz + 1.0, fill, fill, fill, 5000);
            lights[4].set(x + 2.0, y + 2.0, pz + 1.0, fill, fill, fill, 5000);
            let frame = CharacterFrame { player: anim, x, y, z: pz, alpha: 1.0, effect_lights: &lights, ambient: [amb; 3], final_scale: 1.0 };
            let shadow = character_shadow(
                &ShadowInput { player: Some(anim), anim_angle: anim.angle(), ragdoll_direction: None, height_above_floor: 0.0, alpha: 1.0, skeleton: false, animal_size: None, light: [1.0; 3], slope: None, hidden: false },
                [x, y, pz],
                None,
            );
            render_character_object(slot, &frame, shadow, &mc, &res)
        };
        let mut me = object(&player_slot, &player, px, py);
        me.local_player = true;
        if outline_player {
            me.outline = green.character_request(0, px, py, false, cam.x, cam.y);
        }
        let mut za = object(&zombie_a_slot, &zombie_a, ax, ay);
        za.outline = red.character_request(0, ax, ay, false, cam.x, cam.y);
        assert!(za.outline.unwrap().behind_player);
        let mut zb = object(&zombie_b_slot, &zombie_b, bx, by);
        zb.outline = cyan.character_request(0, bx, by, false, cam.x, cam.y);
        assert!(!zb.outline.unwrap().behind_player);

        // The mug: its atlas job with the smart texture, then the draw.
        let mut pre = Vec::new();
        let p = item_params_with_fluid(&mug, Some((&fluid, &NoFluids)), &scripts, &res, max_zoom_is_one(true, zoom)).expect("mug model");
        assert!(p.fluid_filled && p.fluid_texture_mask.is_some(), "{:?} {:?}", p.model.name, p.fluid_texture_mask);
        let key = item_atlas.item_texture(&p);
        let mut used = Vec::new();
        {
            let mut tmb = loader.tm.borrow_mut();
            let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
            item_atlas.render_with(&mut pre, &mut |p| {
                let t = atlas_smart_texture(p, &exists, &mut file, &mut gpu, &mut tmb);
                used.extend(t.clone());
                t
            });
        }
        if let Some(t) = used.first() {
            let px = gpu.read_combined(t).expect("mug texture");
            write_png(&out_dir.join("outline_tex_mug.png"), t.width, t.height, &px);
        }
        let items = item_world_draw(&item_atlas.textures[&key], &cam, &ItemPlacement { x: mx, y: my, z: 0.0, carpet: false, rgb: [amb; 3], alpha: 1.0 });

        let objects = FrameObjects { players: vec![me], items, moving: vec![za, zb], ..Default::default() };
        let r = ChunkRenderer::new(cam, &depth_maps, (w, h));
        let chunks = [ChunkLevels { texture: ChunkTexture::new(1, 1, 0, 0, 0), levels: vec![(0, level)], dirty: true }];
        let cmds = world_frame_with_objects(&r, FrameSize { width: w, height: h }, &chunks, &objects, shadow_tex.clone(), pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 }, pre);
        assert!(cmds.iter().any(|c| matches!(c, pz_render::DrawCmd::OutlineBlit)));
        let list = pz_render::DrawList { cmds };
        gpu.render_frame(&[(pz_gpu::FrameParams::ui(w, h), &list)], &mut loader.tm.borrow_mut());
        for t in used {
            pz_render::combiner::Combiner::release(&mut gpu, t);
        }
        let pixels = gpu.read_pixels();
        let path = out_dir.join(name);
        write_png(&path, w, h, &pixels);
        eprintln!("wrote {}", path.display());
        // Pure red and cyan outline pixels around the zombies.
        let count = |f: &dyn Fn([i32; 3]) -> bool| (0..w * h).filter(|i| f(std::array::from_fn(|k| pixels[(*i * 4) as usize + k] as i32))).count();
        let reds = count(&|p| p[0] > 200 && p[1] < 60 && p[2] < 60);
        let cyans = count(&|p| p[0] < 80 && p[1] > 150 && p[2] > 150);
        assert!(reds > 50, "{reds} red outline pixels");
        assert!(cyans > 50, "{cyans} cyan outline pixels");
        if outline_player {
            let greens = count(&|p| p[0] < 120 && p[1] > 200 && p[2] < 120);
            assert!(greens > 50, "{greens} green outline pixels");
        }
    }
}

/// ws/render-gaps: a mannequin drawn into its chunk texture
/// (`FBORenderCorpses` + `FBORenderChunkCamera`), a dead pig through
/// `DeadBodyAtlas`, and outlines on a corpse, the mannequin and a placed
/// item (`FBORenderObjectOutline`); then the same scene as player 2 of a
/// two-player split screen. Frames go to `PZ_GAPS_FRAME_OUT` (default: the
/// system temp directory).
#[test]
fn mannequin_animal_corpse_and_object_outlines() {
    use pz_character_model::corpse::{animal_body_key, body_key, corpse_shadow, corpse_world_draw, pose, posed_body_key, BodyVisualKey, DeadBody, DeadBodyAtlas};
    use pz_character_model::mannequin::{self, Mannequin};
    use pz_character_model::world_item::item_params;
    use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
    use pz_iso_render::frame::{world_frame_for_player, PlayerView};
    use pz_iso_render::item_atlas::{item_world_draw, max_zoom_is_one, ItemPlacement, WorldItemAtlas};
    use pz_iso_render::object_outline::ObjectOutline;
    use pz_iso_render::objects::ObjectRender;

    let Some(game) = game_dir() else { return };
    let (w, h) = (640u32, 480u32);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(w, h) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let out_dir = std::env::var_os("PZ_GAPS_FRAME_OUT").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let cache = unique_temp_dir("pz-character-model-frame-test");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(&cache);
    let media = MediaIndex::scan(&game).unwrap();
    let rig = |mesh: &str, anims: &[&str]| {
        let (p, ty) = resolve_model_file(&media, mesh, AssetKind::Mesh).unwrap();
        let m = load_mesh(&std::fs::read(p).unwrap(), ty.import_format().unwrap(), &MeshParams::default()).unwrap();
        let mut sd = m.skeleton.clone().unwrap();
        for a in anims {
            let (p, ty) = resolve_model_file(&media, a, AssetKind::Animation).unwrap();
            let clips = load_animation(&std::fs::read(p).unwrap(), ty.import_format().unwrap(), Some(&sd)).unwrap();
            sd.animation_clips.extend(clips);
        }
        Rc::new(sd)
    };
    let male = rig("Skinned/MaleBody", &["Zombie/Zombie_Idle_FloorOnBack"]);
    let female = rig("Skinned/FemaleBody", &["Kate/Kate_Mannequin01"]);
    let pig = rig("Skinned/FarmPig_SowBody", &["Pig/FarmPig_Dead"]);

    let mut pop = Population::default();
    pop.init(&mut fs);
    let groups = BodyLocationGroup::new("Human").unwrap();
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let mut tm = TextureManager::new();
    let mut smart: Vec<Option<Texture>> = Vec::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    // The dead zombie (outlined red).
    let dead = DeadBody { x: 11.9, y: 10.2, z: 0.0, angle: 2.2, dir: 3, zombie: true, outside: true, alpha: 1.0, animal_size: 1.0, ..Default::default() };
    let mut corpse_player = AnimationPlayer::new(male.clone());
    pose(&mut corpse_player, Rc::new(pz_anim::AnimationSet::load_states("zombie", &game.join("media/AnimSets/zombie"), &["onground"])), &dead);
    let corpse_slot = {
        let mut hv = HumanVisual::new(BLOOD_PARTS);
        hv.skin_texture_name = Some("MaleBody02a".into());
        hv.hair_model = Some("Bob".into());
        hv.hair_color = Some([0.35, 0.22, 0.12]);
        let visuals: Vec<ResolvedItemVisual> = ["Tshirt_DefaultTEXTURE", "Trousers_Denim"].iter().map(|n| clothing(&game, n)).collect();
        let human = HumanModel::resolve(&mut hv, false, false, false, &pop.styles(&[], 0.0), &[]);
        let bone_index = |n: &str| corpse_player.skinning_bone_index(n, -1);
        let mut slot = ModelSlot::build(&CharacterModelInput { human: &human, item_visuals: &visuals, equipped: &EquippedModels::default(), body_locations: &groups, hair_styles: pop.hair_styles.as_ref(), beard_styles: pop.beard_styles.as_ref(), scripts: &scripts, body_model_script: None, bone_index: &bone_index });
        let jobs = create_textures(&mut slot, &TextureCreatorInput { human: &human, item_visuals: &visuals, body_locations: &groups, decals: &|_| None, simple_clothing_textures: false });
        let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
        smart.extend(run_jobs(&jobs.jobs, &mut file, &mut gpu, &mut tm));
        slot
    };
    // A dead sow (animal corpse, `pig` anim set's deadbody state).
    let sow = DeadBody { x: 9.6, y: 12.4, z: 0.0, angle: 0.6, dir: 5, animal: true, animal_anim_set: Some("pig".into()), outside: true, alpha: 1.0, animal_size: 1.0, ..Default::default() };
    let mut sow_player = AnimationPlayer::new(pig.clone());
    pose(&mut sow_player, Rc::new(pz_anim::AnimationSet::load_states("pig", &game.join("media/AnimSets/pig"), &["deadbody"])), &sow);
    assert_eq!(sow_player.tracks()[0].name(), "FarmPig_Dead");
    let sow_slot = ModelSlot::single_body("Skinned/FarmPig_SowBody", "Body/FarmPig", Some("animalEffect"));
    // A black female mannequin facing south-east on square (9, 9), naked
    // (its skin texture as the body texture; `HumanVisual` clothing would
    // come from the outfit).
    let mq = Mannequin { x: 9, y: 9, z: 0, dir: 5, female: true, anim_set: "mannequin".into(), anim_state: "female".into(), pose: "pose01".into(), surfaces_below: vec![], surfaces_all: vec![], outside: true, room: false, ambient: [0.8; 3], tint: [1.0; 3], alpha: 1.0 };
    let mq_set = Rc::new(pz_anim::AnimationSet::load_states("mannequin", &game.join("media/AnimSets/mannequin"), &["female"]));
    let mut mq_player = AnimationPlayer::new(female.clone());
    mannequin::pose(&mut mq_player, mq_set.clone(), &mq);
    assert_eq!(mq_player.tracks()[0].name(), "Kate_Mannequin01");
    let mq_slot = ModelSlot::single_body("Skinned/FemaleBody", "Body/F_Mannequin_Black", None);
    // The atlas pose of the mannequin (standing, white ambient, no lights).
    let mut mq_atlas_player = AnimationPlayer::new(female.clone());
    mannequin::pose(&mut mq_atlas_player, mq_set, &Mannequin { ambient: [1.0; 3], ..mq.clone() });

    // A mug on the floor (outlined green).
    let sm = pz_scripts::ScriptManager::new();
    let items_txt = game.join("media/scripts/generated/models_items.txt");
    let item_scripts: HashMap<String, ModelScriptRef> = [("Mug".to_string(), model_script(&sm, &items_txt, "Mug"))].into();
    let mut mug_t = ItemTemplate::new(ItemClass::Normal, "Base", "Mugl");
    mug_t.world_static_model = Some("Mug".into());
    let mug = InventoryItem::create(Rc::new(mug_t), 0);

    tm.load_texture_pack(&fs, "Tiles2x.floor", 0, None);
    tm.set_texture_pack_lookup(&[]);
    let grass = tm.try_get_texture(&fs, "blends_natural_01_16").expect("grass tile");
    let depth_maps = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let floor = Rc::new(SpriteInfo { texture: Some(grass), flags: SpriteFlags { solidfloor: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
    let squares: Vec<RenderSquare> = (8..14)
        .flat_map(|y| (8..14).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
            sq
        })
        .collect();
    let mut level = [None; 64];
    for sq in &squares {
        level[((sq.x - 8) + (sq.y - 8) * 8) as usize] = Some(sq);
    }
    let shadow_tex = tm.shared_texture(&fs, "media/textures/NewShadow.png");

    let loader = Loader { tm: RefCell::new(tm), files: RefCell::new(HashMap::new()), smart: RefCell::new(smart) };
    let res = MediaModelResources::new(&media, &loader);
    let mut bodies = DeadBodyAtlas::new(1 << 40);
    let mut item_atlas = WorldItemAtlas::new(1 << 41);
    let (px, py, pz) = (10.5f32, 10.5f32, 0.0f32);
    let red = [1.0, 0.0, 0.0, 1.0];
    let cyan = [0.0, 1.0, 1.0, 0.65];
    let green = [0.0, 1.0, 0.0, 1.0];
    for (zoom, view, name) in [
        (1.0f32, None, "gaps_frame_zoom1.png"),
        (0.5, None, "gaps_frame_zoom05.png"),
        // Split screen: player 1's frame, then player 2's into the same
        // offscreen buffer (each clears and draws only its own half).
        (1.0, Some(PlayerView { player: 0, num_players: 2, screen: (w, h) }), ""),
        (1.0, Some(PlayerView { player: 1, num_players: 2, screen: (w, h) }), "gaps_frame_splitscreen.png"),
    ] {
        let (vw, vh) = view.map_or((w, h), |v: PlayerView| (v.viewport()[2], v.viewport()[3]));
        let cam = Camera::player(px, py, pz, vw as i32, vh as i32, zoom);
        let mut pre = Vec::new();
        let light = [0.8, 0.8, 0.8, 1.0];
        // The zombie corpse and the sow through the atlas.
        let body_tex = bodies.body_texture(&body_key(&dead, &BodyVisualKey::default()), &dead, &corpse_slot, &corpse_player, &res, &mut pre).clone();
        let sow_tex = bodies.body_texture(&animal_body_key(&sow, Some("FarmPig_SowBody"), Some("FarmPig"), &[]), &sow, &sow_slot, &sow_player, &res, &mut pre).clone();
        let mut corpses = corpse_world_draw(&body_tex, &cam, &dead, light);
        corpses.extend(corpse_world_draw(&sow_tex, &cam, &sow, light));
        let corpse_shadows = vec![corpse_shadow(&dead, Some(&body_tex), [0.8; 3]), corpse_shadow(&sow, Some(&sow_tex), [0.8; 3])];
        // The mannequin: chunk-texture draw, shadow, atlas entry for its
        // outline.
        let mq_body = mq.atlas_body();
        let (set, state) = (mq.anim_set.as_str(), mq.anim_state.as_str());
        let mq_tex = bodies.body_texture(&posed_body_key(&mq_body, &BodyVisualKey::default(), set, state, 0.0, true), &mq_body, &mq_slot, &mq_atlas_player, &res, &mut pre).clone();
        let chunk = ChunkTexture::new(1, 1, 0, 0, 0);
        let chunk_models = vec![mannequin::chunk_models(&mq, &mq_slot, &mq_player, &res, chunk, [px, py])];
        let mannequins = vec![ObjectRender { shadow: Some(mq.shadow([0.8, 0.8, 0.8, 0.8])), ..Default::default() }];
        // The mug.
        let mz1 = max_zoom_is_one(true, zoom);
        let p = item_params(&mug, &item_scripts, &res, mz1).expect("item model");
        let k = item_atlas.item_texture(&p);
        item_atlas.render(&mut pre);
        let mug_tex = item_atlas.textures[&k].clone();
        let mug_at = [10.3f32, 11.6, 0.0];
        let items = item_world_draw(&mug_tex, &cam, &ItemPlacement { x: mug_at[0], y: mug_at[1], z: 0.0, carpet: false, rgb: [0.8; 3], alpha: 1.0 });
        // FBORenderObjectOutline's highlighted objects.
        let object_outlines = vec![
            ObjectOutline::corpse(&body_tex.entry.unwrap(), &cam, [dead.x, dead.y, dead.z], red),
            ObjectOutline::mannequin(&mq_tex.entry.unwrap(), &cam, mq.model_position(), 0.0, 0.0, cyan),
            ObjectOutline::world_item_atlas(&mug_tex.entry.unwrap(), mug_tex.max_zoom_is_one, &cam, mug_at, green),
        ];
        let objects = FrameObjects { corpse_shadows, mannequins, corpses, items, object_outlines, chunk_models, ..Default::default() };
        let r = ChunkRenderer::new(cam, &depth_maps, (vw, vh));
        let chunks = [ChunkLevels { texture: chunk, levels: vec![(0, level)], dirty: true }];
        let cmds = world_frame_for_player(&r, FrameSize { width: vw, height: vh }, view, &chunks, &objects, shadow_tex.clone(), pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 }, pre);
        let list = pz_render::DrawList { cmds };
        gpu.render_frame(&[(pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(w, h) }, &list)], &mut loader.tm.borrow_mut());
        assert_eq!(gpu.error(), None);
        if name.is_empty() {
            continue;
        }
        let pixels = gpu.read_pixels();
        let path = out_dir.join(name);
        write_png(&path, w, h, &pixels);
        eprintln!("wrote {}", path.display());
        let count = |f: &dyn Fn(&[u8]) -> bool| pixels.chunks(4).filter(|p| f(p)).count();
        // Each outline colour shows (the ring around its object).
        assert!(count(&|p| p[0] > 200 && p[1] < 60 && p[2] < 60) > 20, "red corpse outline");
        assert!(count(&|p| p[0] < 60 && p[1] > 200 && p[2] < 60) > 10, "green item outline");
        if view.is_some() {
            // Both halves show the corpse's red outline, each centred on
            // its own player.
            let red_in = |x0: u32, x1: u32| (0..h).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| {
                let p = &pixels[((y * w + x) * 4) as usize..][..3];
                p[0] > 200 && p[1] < 60 && p[2] < 60
            }).count();
            assert!(red_in(0, w / 2) > 20 && red_in(w / 2, w) > 20, "{} {}", red_in(0, w / 2), red_in(w / 2, w));
        }
    }
}
