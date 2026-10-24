use super::*;
use crate::world::{DepthMaps, DepthPreset};
use crate::Camera;
use pz_render::{Sprite, WorldShader};

fn tex(name: &str, w: u32, h: u32, ox: f32, oy: f32) -> Texture {
    let mut t = Texture::empty(name);
    t.id = Some(7);
    t.width = w;
    t.height = h;
    t.width_orig = 128;
    t.height_orig = 256;
    t.width_hw = w.next_power_of_two() as i32;
    t.height_hw = h.next_power_of_two() as i32;
    t.offset_x = ox;
    t.offset_y = oy;
    t.x_end = 1.0;
    t.y_end = 1.0;
    t.ready = true;
    t
}

struct Maps(Option<Texture>);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        None
    }
    fn default_texture(&self) -> Option<Texture> {
        None
    }
    fn billboard(&self) -> Option<Texture> {
        self.0.clone()
    }
}

fn names(f: &FireState) -> Vec<String> {
    f.anims.iter().map(FireAnim::sprite_name).collect()
}

#[test]
fn a_new_fire_draws_rand_in_the_constructors_order() {
    let r = Rand::new(5);
    let f = FireState::new_fire(FireSquare::default(), 100, 0, &r);
    // Same calls on a twin generator: the random frame, the life, the
    // spread delay.
    let t = Rand::new(5);
    let frame = t.next_float(0.0, 30.0);
    let life = 800 + t.next(2200);
    let spread = t.next(life - life / 2);
    assert_eq!(
        f.anims,
        vec![FireAnim {
            kind: AnimKind::Fire,
            anim: 1,
            frame,
            frame_increase: 0.5,
            soff_x: 0,
            soff_y: 0,
            tint: [1.0; 3]
        }]
    );
    assert_eq!(
        (f.life, f.spread_delay, f.spread_timer),
        (life, spread, spread)
    );
    assert_eq!(
        (f.life_stage, f.life_stage_duration, f.life_stage_timer),
        (0, life / 5, life / 5)
    );
    assert_eq!(f.energy, 100);
    assert!(f.burning());
    assert_eq!(f.anims[0].texture_name_for(29), "Fire_01_29");

    // A floor that burns poorly shortens the fire; vegetation adds energy.
    let r = Rand::new(9);
    let f = FireState::new_fire(
        FireSquare {
            floor_fire_requirement: Some(40),
            ..Default::default()
        },
        10,
        0,
        &r,
    );
    assert!((300..600).contains(&f.life), "{}", f.life);
    let f = FireState::new_fire(
        FireSquare {
            vegitation: true,
            floor_fire_requirement: Some(40),
            ..Default::default()
        },
        10,
        1000,
        &r,
    );
    assert_eq!((f.life, f.energy), (1000, 60));
}

#[test]
fn the_life_stages_swap_flames_and_smoke() {
    let r = Rand::new(1);
    let mut f = FireState::new_fire(FireSquare::default(), 1000, 1000, &r);
    assert_eq!(f.life_stage_duration, 200);
    // The light appears on the first update.
    let ev = f.update(0.0, 0.0, 0, 1.0, &r);
    assert!(ev.is_empty());
    assert_eq!(
        f.light,
        Some(FireLight {
            r: 0.61,
            g: 0.175,
            b: 0.0,
            radius: 5
        })
    );
    let mut stages = Vec::new();
    let mut all = Vec::new();
    for _ in 0..1100 {
        let ev = f.update_from_timer(1.0001, &r);
        if stages.last().map(|(s, _)| *s) != Some(f.life_stage) {
            stages.push((f.life_stage, names(&f)));
        }
        all.extend(ev);
    }
    let want: Vec<(i32, Vec<&str>)> = vec![
        (0, vec!["Fire01"]),
        (1, vec!["Fire01"]),
        (2, vec!["Smoke02", "Fire02"]),
        (3, vec!["Smoke03", "Fire03"]),
        (4, vec!["Smoke02", "Fire02"]),
        (5, vec!["Smoke01"]),
    ];
    assert_eq!(
        stages,
        want.into_iter()
            .map(|(s, n)| (s, n.into_iter().map(String::from).collect()))
            .collect::<Vec<_>>()
    );
    // Two burns, four light changes, spreads, then the end.
    assert_eq!(all.iter().filter(|e| **e == FireEvent::Burn).count(), 2);
    // Stage 1 keeps radius 5, so no new light; 8, 12, 8, 1 follow.
    assert_eq!(
        all.iter()
            .filter(|e| **e == FireEvent::LightChanged)
            .count(),
        4
    );
    assert!(all.contains(&FireEvent::Spread));
    assert_eq!(all.last(), Some(&FireEvent::Extinct));
    let l = f.light.unwrap();
    assert_eq!((l.r, l.g, l.b, l.radius), (0.51, 0.27, 0.18, 1));
    // In stage 5 the light follows the object's alpha.
    f.update(0.0, 0.0, 0, 0.5, &r);
    assert_eq!(f.light.unwrap().r, 0.61 * 0.5);
}

#[test]
fn animations_loop_through_thirty_frames() {
    let mut a = FireAnim {
        kind: AnimKind::Smoke,
        anim: 2,
        frame: 29.0,
        frame_increase: 0.5,
        soff_x: 0,
        soff_y: 0,
        tint: SMOKE_TINT_MOD,
    };
    a.advance(1.0);
    assert_eq!(a.texture_name(), "Smoke_02_29");
    a.advance(1.0);
    assert_eq!(a.frame, 0.0);
    a.frame = 31.5;
    assert_eq!(a.frame_index(), 29);
}

#[test]
fn loading_attaches_the_next_stages_animations() {
    let r = Rand::new(3);
    let save = |stage: i32, smoke: bool| FireSave {
        life_stage: stage,
        life_stage_duration: 90,
        life_stage_timer: 7,
        smoke,
        ..Default::default()
    };
    let cases: [(i32, bool, i32, i32, &[&str]); 7] = [
        (-1, false, 0, 7, &["Fire01"]),
        (0, false, 1, 90, &["Fire02"]),
        (1, false, 2, 90, &["Smoke01", "Fire03"]),
        (2, false, 3, 30, &["Smoke02", "Fire02"]),
        (3, false, 4, 30, &["Smoke03", "Fire01"]),
        (3, true, 4, 30, &["Smoke03"]),
        (4, false, 5, 30, &["Smoke01"]),
    ];
    for (saved, smoke, stage, timer, anims) in cases {
        let f = FireState::load(&save(saved, smoke), &r);
        assert_eq!(
            (f.life_stage, f.life_stage_timer, names(&f)),
            (
                stage,
                timer,
                anims.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            ),
            "saved {saved}"
        );
        assert_eq!(f.save(0).life_stage, stage - 1);
    }
    let p = FireState::load(
        &FireSave {
            perm: true,
            light_radius: 3,
            ..Default::default()
        },
        &r,
    );
    assert_eq!(
        (p.anims.len(), p.anims[0].soff_x, p.anims[0].soff_y),
        (1, -2, 2)
    );
    let mut bytes = Vec::new();
    for v in [10, 2, 1, 4, 5, 6, 7, 8, 9] {
        bytes.extend(i32::to_be_bytes(v));
    }
    bytes.extend([1, 12, 0]);
    bytes.extend(i64::to_be_bytes(123));
    let (s, n) = FireSave::read(&bytes, 242).unwrap();
    assert_eq!(n, 47);
    assert_eq!(
        (
            s.life,
            s.life_stage,
            s.age,
            s.perm,
            s.light_radius,
            s.smoke,
            s.saved_update_time
        ),
        (10, 1, 9, true, 12, false, Some(123))
    );
    assert_eq!(FireSave::read(&bytes, 241).unwrap().1, 39);
}

#[test]
fn smoke_fires_and_permanent_fires() {
    let r = Rand::new(4);
    let s = FireState::new_smoke(5, 0, true, &r);
    assert_eq!(
        (s.life_stage, names(&s), s.anims[0].frame),
        (4, vec!["Smoke03".to_string()], 0.0)
    );
    assert_eq!(s.life_stage_duration, s.life / 4);
    assert!(!s.burning());
    let mut s = s;
    s.update(1.0, 0.0, 0, 1.0, &r);
    assert_eq!(s.light, None);

    let mut p = FireState::permanent();
    p.light_radius = 8;
    p.set_life_stage(2, &r);
    assert_eq!(names(&p), ["Smoke03", "Fire03"]);
    p.update(1.0, 50.0, 0, 1.0, &r);
    assert_eq!((p.light.unwrap().radius, p.life), (8, 0));
}

#[test]
fn saved_time_catches_up_once() {
    let r = Rand::new(2);
    let mut f = FireState::load(
        &FireSave {
            life: 500,
            life_stage: -1,
            life_stage_duration: 100,
            life_stage_timer: 100,
            energy: 10,
            saved_update_time: Some(1000),
            ..Default::default()
        },
        &r,
    );
    // 3000 ms away: 100 ticks, entering stage 1.
    let ev = f.update(0.0, 0.0, 4000, 1.0, &r);
    assert_eq!(f.age, 99);
    f.update(0.0, 0.0, 4000, 1.0, &r);
    assert_eq!(f.age, 99);
    assert!(ev.is_empty());
}

#[test]
fn the_last_stage_fades_the_smoke() {
    let f = FireRender {
        campfire: false,
        life_stage: 5,
        life_stage_timer: 50,
        life_stage_duration: 200,
    };
    // f = 1 - 50/100: 1 - 2^-5.
    assert_eq!(f.stage5_alpha(), Some(1.0 - 2.0f32.powf(-5.0)));
    assert_eq!(
        FireRender {
            life_stage_timer: 100,
            ..f
        }
        .stage5_alpha(),
        Some(1.0)
    );
    assert_eq!(FireRender { life_stage: 4, ..f }.stage5_alpha(), None);
}

fn sprites(out: &[DrawCmd]) -> Vec<(&Sprite, &WorldShader)> {
    out.iter()
        .filter_map(|c| match c {
            DrawCmd::WorldSprite(s, w) => Some((s, w)),
            _ => None,
        })
        .collect()
}

fn fire_square(campfire: bool) -> RenderSquare {
    let r = Rand::new(11);
    let mut f = FireState::new_fire(FireSquare::default(), 100, 1000, &r);
    f.anims = vec![
        FireAnim {
            kind: AnimKind::Smoke,
            anim: 2,
            frame: 3.0,
            frame_increase: 0.5,
            soff_x: 0,
            soff_y: 0,
            tint: SMOKE_TINT_MOD,
        },
        FireAnim {
            kind: AnimKind::Fire,
            anim: 2,
            frame: 5.7,
            frame_increase: 0.5,
            soff_x: 0,
            soff_y: 0,
            tint: FIRE_TINT_MOD,
        },
    ];
    let o = f.render_object(campfire, &|n| match n {
        "Smoke_02_3" => Some(tex(n, 107, 205, 18.0, 51.0)),
        "Fire_02_5" => Some(tex(n, 77, 114, 30.0, 142.0)),
        _ => None,
    });
    let mut sq = RenderSquare::new(10, 6, 0);
    sq.light_info = ColorInfo {
        r: 0.8,
        g: 0.6,
        b: 0.4,
        a: 1.0,
    };
    sq.objects.push(o);
    sq
}

#[test]
fn flames_and_smoke_stand_on_the_square_centre() {
    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let sq = fire_square(false);
    let mut out = Vec::new();
    let maps = Maps(None);
    let mut ctx = SpriteCtx {
        cam: &cam,
        chunk: None,
        depth_maps: &maps,
        out: &mut out,
    };
    let mut obj = sq.objects[0].clone();
    obj.alpha = 0.9;
    render_animated_attachments(&mut ctx, &sq, &obj);
    assert!(matches!(
        out[0],
        DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA)
    ));
    let s = sprites(&out);
    assert_eq!(s.len(), 2);
    // Scale 1: the 128x256 frame's bottom centre on the square's centre.
    let cx = pz_render::iso::x_to_screen(10.0, 6.0) - cam.off_x as f32;
    let cy = pz_render::iso::y_to_screen(10.0, 6.0, 0.0, 0) - cam.off_y as f32 + 32.0;
    let (smoke, sw) = s[0];
    assert_eq!(smoke.x[0], cx - 64.0 + 18.0);
    assert_eq!(smoke.y[0], cy - 256.0 + 51.0);
    assert_eq!(smoke.x[1] - smoke.x[0], 107.0);
    // Smoke in the square's light times its tint; flames white.
    assert_eq!(smoke.color[0], crate::color::to_abgr(0.4, 0.3, 0.2, 0.9));
    let (flame, _) = s[1];
    assert_eq!(flame.color[0], crate::color::to_abgr(1.0, 1.0, 1.0, 0.9));
    assert_eq!(flame.y[2], cy - 256.0 + 142.0 + 114.0);
    // No billboard depth texture: the default shader at the square's depth.
    let depth =
        pz_render::iso::chunk_depth_data(cam.chunk_x(), cam.chunk_y(), 1, 0, 0).depth_start + 0.5;
    let z = depth * 2.0 - 1.0 + (pz_render::iso::calculate_depth(10.0, 6.0, 0.0) * 2.0 - 1.0);
    assert_eq!(*sw, WorldShader::Default { z });
}

#[test]
fn campfires_are_smaller_and_fires_use_the_billboard_depth() {
    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let sq = fire_square(true);
    let mut out = Vec::new();
    let mut bb = tex("DEPTH_billboard", 128, 256, 0.0, 0.0);
    bb.id = Some(9);
    let maps = Maps(Some(bb.clone()));
    let mut ctx = SpriteCtx {
        cam: &cam,
        chunk: None,
        depth_maps: &maps,
        out: &mut out,
    };
    render_animated_attachments(&mut ctx, &sq, &sq.objects[0]);
    let s = sprites(&out);
    let (smoke, sw) = s[0];
    // Scale 0.75: soffX = (short)(64 - 48) = 16, soffY = (short)(256 - 192).
    let cx = pz_render::iso::x_to_screen(10.0, 6.0) - cam.off_x as f32;
    let cy = pz_render::iso::y_to_screen(10.0, 6.0, 0.0, 0) - cam.off_y as f32;
    assert_eq!(smoke.x[0], cx - 64.0 + 16.0 + 18.0 * 0.75);
    assert_eq!(smoke.x[1] - smoke.x[0], 107.0 * 0.75);
    assert_eq!(smoke.y[0], cy - 224.0 + 64.0 + 51.0 * 0.75);
    let WorldShader::TileDepth {
        depth, front, far, ..
    } = sw
    else {
        panic!("{sw:?}")
    };
    assert_eq!(depth.id, Some(9));
    let (cxs, cys) = (
        pz_render::iso::fastfloor(cam.x),
        pz_render::iso::fastfloor(cam.y),
    );
    assert_eq!(*far, pz_render::iso::square_depth(cxs, cys, 9.5, 5.5, 0.0));
    assert_eq!(
        *front,
        pz_render::iso::square_depth(cxs, cys, 10.5, 6.5, 1.0)
    );
}

#[test]
fn the_translucent_pass_draws_fires() {
    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let sq = fire_square(false);
    let maps = Maps(None);
    let r = crate::ChunkRenderer::new(cam, &maps, (800, 600));
    assert_eq!(
        crate::cell::render_layer(&sq, &sq.objects[0]),
        crate::cell::Layer::Translucent
    );
    let mut out = Vec::new();
    r.render_translucent_objects(&[&sq], 0, &mut out);
    assert_eq!(sprites(&out).len(), 2);
}

#[test]
fn the_manager_evicts_the_oldest_fire_and_oscillates() {
    let mut ages = vec![0; MAX_FIRE_OBJECTS - 1];
    assert_eq!(fire_to_evict(&ages), None);
    ages.push(0);
    assert_eq!(fire_to_evict(&ages), None);
    ages[3] = 9;
    ages[7] = 9;
    assert_eq!(fire_to_evict(&ages), Some(3));
    let mut o = FireOscillators::default();
    o.update();
    let x = f64::from(0.0876f32) * f64::from(15.6f32);
    assert_eq!(o.red_val, (x.sin() + 1.0) / 2.0 * f64::from(0.0039f32));
}

/// The fire and smoke frames in the game's tile packs:
/// `PZ_GAME_DIR=... cargo test -p pz-iso-render -- --ignored fire_frames`.
#[test]
#[ignore]
fn fire_frames_in_the_game_packs() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else {
        return;
    };
    let fs = pz_fs::GameFs::new(std::path::PathBuf::from(dir));
    let mut tm = pz_render::TextureManager::new();
    tm.load_texture_pack(&fs, "Tiles2x", 0, None);
    tm.set_texture_pack_lookup(&[]);
    for kind in [AnimKind::Fire, AnimKind::Smoke] {
        for anim in 1..=3 {
            let a = FireAnim {
                kind,
                anim,
                frame: 0.0,
                frame_increase: 0.5,
                soff_x: 0,
                soff_y: 0,
                tint: [1.0; 3],
            };
            for i in 0..a.frame_count() {
                let t = tm
                    .shared_texture(&fs, &a.texture_name_for(i))
                    .unwrap_or_else(|| panic!("{}", a.texture_name_for(i)));
                // Drawn at scale 1 at tile scale 2: the size performRenderFrame keeps.
                assert_eq!(
                    (t.width_orig(), t.height_orig()),
                    (128, 256),
                    "{}",
                    a.texture_name_for(i)
                );
            }
        }
    }
}
