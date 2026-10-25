//! A sports car on a road next to a dressed player, drawn through the
//! whole iso frame on the GPU (chunk texture and composite, the object pass
//! with the player and the vehicle with their shadows, the translucent
//! loop, the screen pass) and saved as a PNG for inspection. The car's
//! front-left door is open (its skinned door model posed by the
//! `Opened` part animation) and its headlights are on (the lights zones of
//! the vehicle shader lit).
//!
//! Reads the installed game at runtime: point `PZ_GAME_DIR` at the folder
//! holding `media/`; `PZ_VEHRENDER_FRAME_OUT` names the directory the
//! frames go to (default: the system temp directory). Without the game
//! (or a GPU adapter) this test does nothing.

mod common;

use common::night;

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
use pz_character_model::lights::{EffectLight, LightContext, PlayerLightData};
use pz_character_model::mask::BLOOD_PARTS;
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot};
use pz_character_model::render::CharacterFrame;
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
use pz_render::{flags, DrawCmd, Texture, TextureManager};
use pz_vehicle_render::lights::vehicle_torches;
use pz_vehicle_render::models::MediaVehicleModels;
use pz_vehicle_render::render::{render_vehicle_object, SquareLight, VehicleFrame};
use pz_vehicle_render::VehicleModelSet;

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
        let t = std::fs::File::open(path)
            .ok()
            .and_then(|f| ImageData::from_png(std::io::BufReader::new(f), false).ok())
            .map(|img| {
                let name = path.to_string_lossy();
                self.tm.borrow_mut().texture_from_pixels(
                    &name,
                    img.width,
                    img.height,
                    img.data.pixels,
                    0,
                )
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

/// A path under the system temp dir unique to this call (tag, process id and a
/// per-call counter): tests run in parallel and must not share directories.
fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{tag}-{}-{n}", std::process::id()))
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
fn car_with_open_door_and_headlights_next_to_the_player() {
    let Some(game) = common::game_dir() else {
        return;
    };
    let (w, h) = (640u32, 480u32);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(w, h) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let out_dir = std::env::var_os("PZ_VEHRENDER_FRAME_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let cache = unique_temp_dir("pz-vehicle-render-frame-test");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(&cache);
    let media = MediaIndex::scan(&game).unwrap();
    let sm = common::scripts(&game);

    // The player: idle, facing south-east, in a T-shirt, jeans and shoes.
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
    player.set_target_and_current_direction(0.3, 1.0);
    let t = player.play(&clip, true, false, -1.0).unwrap();
    player.track_mut(t).unwrap().set_blend_weight(1.0);
    player.update(0.5, None);
    let mut pop = Population::default();
    pop.init(&mut fs);
    let mut hv = HumanVisual::new(BLOOD_PARTS);
    hv.skin_texture_name = Some("MaleBody01a".into());
    hv.hair_model = Some("CrewCut".into());
    hv.hair_color = Some([0.35, 0.22, 0.12]);
    let visuals: Vec<ResolvedItemVisual> =
        ["Tshirt_DefaultTEXTURE", "Shoes_Black", "Trousers_Denim"]
            .iter()
            .map(|n| clothing(&game, n))
            .collect();
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
    let mut tm = TextureManager::new();
    let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
    let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
    let smart = run_jobs(&jobs.jobs, &mut file, &mut gpu, &mut tm);

    // A 6x6 road patch in chunk (1, 1).
    tm.load_texture_pack(&fs, "Tiles2x.floor", 0, None);
    tm.set_texture_pack_lookup(&[]);
    let road = tm
        .try_get_texture(&fs, "blends_street_01_0")
        .expect("road tile");
    let depth_maps = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let floor = Rc::new(SpriteInfo {
        texture: Some(road),
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
    let vehicle_shadow = tm.shared_texture_flags(&fs, "media/vehicleShadow.png", flags::COMPRESS);

    // The car: a SportsCar_ez beside the player, front-left door open,
    // headlights on with a charged battery and the headlight spots made
    // as Vehicles.Create.Headlight makes them.
    let v = common::vehicle("Base.SportsCar_ez", 12.6, 10.4, 0.5);
    {
        let battery = v.battery().unwrap();
        let mut t = pz_inventory::ItemTemplate::new(
            pz_inventory::ItemClass::Drainable,
            "Base",
            "CarBattery1",
        );
        t.use_delta = 0.1;
        let mut i = pz_inventory::InventoryItem::create(Rc::new(t), -1);
        i.uses = 10;
        i.condition = 100;
        battery.st_mut().item = Some(pz_vehicles::part::PartItem::Item(Rc::new(RefCell::new(i))));
    }
    for (id, x, y) in [
        ("HeadlightLeft", 0.5f32, 1.0f32),
        ("HeadlightRight", -0.5, 1.0),
        ("HeadlightRearLeft", 0.5, -1.0),
        ("HeadlightRearRight", -0.5, -1.0),
    ] {
        let p = v.part_by_id(id).unwrap();
        p.create_spot_light(x, y, 8.0, 1.0, 0.96, 0);
    }
    // createPhysics collects the light parts (`lights`).
    v.create_physics(false);
    v.set_headlights_on(true);
    for p in v.parts() {
        if p.id().is_some_and(|i| i.contains("Headlight")) {
            p.set_light_active(true);
        }
    }
    v.part_by_id("DoorFrontLeft")
        .unwrap()
        .st_mut()
        .door
        .as_mut()
        .unwrap()
        .open = true;
    {
        let mut st = v.st_mut();
        st.color_hue = 0.02;
        st.color_saturation = 0.8;
        st.color_value = 0.65;
        st.rust = 0.1;
        st.vehicle_id = 1;
    }
    let torches = vehicle_torches(&v);
    eprintln!("torches: {torches:?}");

    // Night: the same car in the lighting pass (its headlights as torches,
    // `LightingJNI.checkLights`), the player at its spot looking south-east
    // along the beams; the results light a full chunk of road.
    let (lcell, lsprites, lmeta) = night::scene();
    let lview = pz_world_render::WorldView {
        cell: &lcell,
        meta: &lmeta,
        sprites: &lsprites,
    };
    let vehicles = [&*v];
    let hooks = night::VehicleTorches(&vehicles);
    let mut lighting = night::new_lighting(&lview, &hooks);
    // A street lamp north of the player and the car (`IsoCell`'s
    // lamppost list), reaching both but neither the beams' squares nor the
    // dark square beside the car.
    lighting.add_lamppost(pz_world_render::CellLight::new(11, 8, 0, 0.5, 0.4, 0.28, 5));
    let lit = night::light_from(
        &lview,
        &mut lighting,
        &hooks,
        12,
        (10.5, 10.5),
        pz_lighting::jni::IsoDirection::SE,
    );
    let night_squares: Vec<RenderSquare> = (8..16)
        .flat_map(|y| (8..16).map(move |x| (x, y)))
        .map(|(x, y)| {
            let l = &lit.squares[&(x, y, 0)];
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects
                .push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
            sq.light_info = l.light_info;
            sq.vert_lights = l.vert_lights;
            sq.light_level = l.light_level;
            sq.cut = l.cut;
            sq
        })
        .collect();
    let mut night_level = [None; 64];
    for sq in &night_squares {
        night_level[((sq.x - 8) + (sq.y - 8) * 8) as usize] = Some(sq);
    }
    let night_light = |x: f32, y: f32| {
        let c = lit.squares[&(x.floor() as i32, y.floor() as i32, 0)].light_info;
        [c.r, c.g, c.b]
    };
    // A square 4 ahead of the car's centre along its axis, in the front
    // beams, and one beside the car (outside every beam; the rear pair
    // lights the road behind it).
    let (fx, fy) = (torches[0].angle_x, torches[0].angle_y);
    let ahead = (
        (12.6 + fx * 4.0).floor() as i32,
        (10.4 + fy * 4.0).floor() as i32,
    );
    let beside = (9, 13);
    let (la, lb) = (
        lit.squares[&(ahead.0, ahead.1, 0)].light_level,
        lit.squares[&(beside.0, beside.1, 0)].light_level,
    );
    eprintln!("night light ahead {ahead:?} {la}, beside {beside:?} {lb}");
    assert!(la > lb + 0.3 && lb > 0.0, "beam {la} vs beside {lb}");

    let loader = Loader {
        tm: RefCell::new(tm),
        files: RefCell::new(HashMap::new()),
        smart: RefCell::new(smart),
    };
    let load_tex = |p: &str, f: i32| loader.tm.borrow_mut().shared_texture_flags(&fs, p, f);
    let source = MediaVehicleModels::new(&media, sm.clone(), false, &load_tex);
    let mut load_tex2 = |p: &str, f: i32| loader.tm.borrow_mut().shared_texture_flags(&fs, p, f);
    let mut set = VehicleModelSet::create(&v, &source, &mut load_tex2, &|_, _| None);
    assert!(set.body.is_some(), "body model");
    assert_eq!(set.parts.len(), 7, "doors, hood and wheels");
    set.update_animations(1.0);
    set.update_animations(1.0);
    set.update_transform(&v);
    set.instance.update_lights(&v);
    set.instance.do_damage_overlay(&v, true);
    assert_eq!(
        set.instance.texture_lights_enables2[8], 1.0,
        "left headlight lit"
    );
    let door = set
        .parts
        .iter()
        .position(|p| p.part.id().as_deref() == Some("DoorFrontLeft"))
        .unwrap();
    assert!(set.parts[door].track.is_some(), "door posed");

    // The result lights: the lamp on the player's and the car's squares,
    // the front headlights on a square in their beams.
    let car_lights = lighting.result_lights(0, 12, 10, 0);
    eprintln!("car square result lights {car_lights:?}");
    eprintln!(
        "beam square result lights {:?}",
        lighting.result_lights(0, ahead.0, ahead.1, 0)
    );
    assert!(car_lights
        .iter()
        .any(|l| (l.x, l.y, l.radius) == (11, 8, 5)));
    assert!(lighting
        .result_lights(0, ahead.0, ahead.1, 0)
        .iter()
        .any(|l| l.id == torches[0].id || l.id == torches[1].id));
    let res = MediaModelResources::new(&media, &loader);
    let (px, py, pz) = (10.5f32, 10.5f32, 0.0f32);
    let mut lit_sum = None;
    for (zoom, name, headlights, night) in [
        (1.0f32, "vehrender_frame_zoom1.png", true, false),
        (0.5, "vehrender_frame_zoom05.png", true, false),
        (1.0, "vehrender_frame_lights_off.png", false, false),
        (1.0, "vehrender_frame_night.png", true, true),
    ] {
        v.st_mut().headlights_on = headlights;
        set.instance.update_lights(&v);
        let cam = Camera::player(px, py, pz, w as i32, h as i32, zoom);
        let mc = ModelCamera::new(&cam, w as f32 * zoom, h as f32 * zoom);
        // The player's model lights: by day a flat ambient with the two
        // fill lights; at night `ModelInstance.updateLights` on the result
        // lights of the player's square (`PlayerData`, faded in over a few
        // frames as in the game).
        let mut lights = [EffectLight::default(); 5];
        let mut amb3 = [0.8f32; 3];
        if night {
            let mut pd = PlayerLightData::default();
            let ctx = LightContext {
                x: px,
                y: py,
                z: pz,
                result_lights: lighting
                    .result_lights(0, px.floor() as i32, py.floor() as i32, 0)
                    .to_vec(),
                target_ambient: night_light(px, py),
                outside: true,
                in_room: false,
                room_light_ids: Vec::new(),
                player_index: Some(0),
                torch_strength: 0.0,
                forward: [std::f32::consts::FRAC_1_SQRT_2; 2],
                multiplier: 1.0,
            };
            eprintln!("player square result lights {:?}", ctx.result_lights);
            assert!(!ctx.result_lights.is_empty(), "the lamp reaches the player");
            for _ in 0..120 {
                pd.update(&ctx);
            }
            lights = pd.effect_lights_main;
            amb3 = pd.current_ambient;
            eprintln!("player effect lights {lights:?}");
        } else {
            let fill = 0.8 * 1.7 / 4.0;
            lights[3].set(px - 2.0, py - 2.0, pz + 1.0, fill, fill, fill, 5000);
            lights[4].set(px + 2.0, py + 2.0, pz + 1.0, fill, fill, fill, 5000);
        }
        let frame = CharacterFrame {
            player: &player,
            x: px,
            y: py,
            z: pz,
            alpha: 1.0,
            effect_lights: &lights,
            ambient: amb3,
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
        let chr = render_character_object(&slot, &frame, shadow, &mc, &res);
        let vframe = VehicleFrame {
            alpha: 1.0,
            ambient: if night { night_light(12.6, 10.4) } else { amb3 },
            // `VehicleModelInstance.UpdateLights`: the car's square's
            // result lights.
            lights: if night {
                SquareLight::from_result_lights(car_lights)
            } else {
                Vec::new()
            },
            sky_texture_shift: 0.0,
            sky_textures: [None, None],
            square_light: if night { night_light(12.6, 10.4) } else { amb3 },
            seen: true,
            slope: None,
            default_shadow: vehicle_shadow.clone(),
        };
        let car = render_vehicle_object(&v, &set, &vframe, &mc);
        let n = car
            .draws
            .iter()
            .filter(|d| matches!(d, DrawCmd::VehicleModel(_)))
            .count();
        assert_eq!(n, 8, "body, two doors, hood, four wheels");
        assert!(car.shadow.is_some());
        let objects = FrameObjects {
            players: vec![chr],
            moving: vec![car],
            ..Default::default()
        };
        let r = ChunkRenderer::new(cam, &depth_maps, (w, h));
        let chunks = [ChunkLevels {
            texture: ChunkTexture::new(1, 1, 0, 0, 0),
            levels: vec![(0, if night { night_level } else { level })],
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
        let path = out_dir.join(name);
        write_png(&path, w, h, &pixels);
        eprintln!("wrote {}", path.display());
        // The car is where it stands: 2.1 squares east and 0.1 north of
        // the player is (2.1 - -0.1) * 64 = 141 px right, (2.1 + -0.1) * 32
        // = 64 px down at zoom 1 (tile scale 2). Its paint is red; the
        // road is grey.
        let at = |x: u32, y: u32| -> [i32; 3] {
            std::array::from_fn(|k| pixels[((y * w + x) * 4) as usize + k] as i32)
        };
        let (cx, cy) = (
            (w as f32 / 2.0 + 141.0 / zoom) as u32,
            (h as f32 / 2.0 + 64.0 / zoom) as u32,
        );
        let r = (40.0 / zoom) as i32;
        let mut red = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                let p = at((cx as i32 + dx) as u32, (cy as i32 + dy) as u32);
                if p[0] > p[1] + 40 && p[0] > p[2] + 40 {
                    red += 1;
                }
            }
        }
        if night {
            // The road in the beams is lit, behind the car it is dark.
            let screen = |(x, y): (i32, i32)| {
                let (dx, dy) = (x as f32 + 0.5 - px, y as f32 + 0.5 - py);
                (
                    (w as f32 / 2.0 + (dx - dy) * 64.0) as u32,
                    (h as f32 / 2.0 + (dx + dy) * 32.0) as u32,
                )
            };
            let sum = |(x, y): (u32, u32)| -> i32 {
                (0..5)
                    .flat_map(|dy| (0..5).map(move |dx| (x + dx - 2, y + dy - 2)))
                    .map(|(x, y)| at(x, y).iter().sum::<i32>())
                    .sum()
            };
            let (a, b) = (sum(screen(ahead)), sum(screen(beside)));
            eprintln!("night pixels ahead {a} beside {b}");
            assert!(a > b * 2, "beam {a} vs beside {b}");
            continue;
        }
        assert!(red > r * r, "car paint around the car: {red}");
        // The headlights' lights texture: brighter with them on.
        if zoom == 1.0 {
            let front: i32 = (370..440)
                .flat_map(|y| (350..480).map(move |x| (x, y)))
                .map(|(x, y)| at(x, y).iter().sum::<i32>())
                .sum();
            match lit_sum {
                None => lit_sum = Some(front),
                Some(on) => assert!(on > front + 2000, "headlights on {on} vs off {front}"),
            }
        }
    }
}
