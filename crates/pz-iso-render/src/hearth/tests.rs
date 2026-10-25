use super::*;
use crate::chunk::ChunkTexture;
use crate::world::{DepthMaps, DepthPreset};
use crate::Camera;
use pz_core::random::Rand;
use pz_render::{Sprite, WorldShader};

fn tex(name: &str, w: u32, h: u32, id: u32) -> Texture {
    let mut t = Texture::empty(name);
    t.id = Some(id);
    t.width = w;
    t.height = h;
    t.width_orig = w as i32;
    t.height_orig = h as i32;
    t.width_hw = w.next_power_of_two() as i32;
    t.height_hw = h.next_power_of_two() as i32;
    t.x_end = 1.0;
    t.y_end = 1.0;
    t.ready = true;
    t
}

struct Maps;

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        None
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(tex("DEPTH_default", 128, 256, 40))
    }
    fn billboard(&self) -> Option<Texture> {
        None
    }
}

fn frame_size(_: &str) -> Option<(i32, i32)> {
    Some((128, 256))
}

fn textures(n: &str) -> Option<Texture> {
    match n {
        FUEL_TEXTURE => Some(tex(n, 64, 32, 21)),
        _ if n.starts_with("Fire_01_") || n.starts_with("Smoke_01_") => Some(tex(n, 128, 256, 22)),
        _ => None,
    }
}

fn wall_fireplace(lit: bool, fuel: i32) -> FireplaceInput<'static> {
    FireplaceInput {
        lit,
        fuel_amount: fuel,
        sprite_name: "fixtures_fireplace_01_0",
        metal_drum: false,
        container_type: Some("fireplace"),
        collide_w: true,
        collide_n: false,
    }
}

#[test]
fn a_lit_fireplace_attaches_logs_flames_and_a_light() {
    let r = Rand::new(3);
    let mut f = FireplaceState::new();
    f.update(&wall_fireplace(true, 30), 0.0, &r, &frame_size);
    let t = Rand::new(3);
    let frame = t.next_float(0.0, 30.0);
    assert_eq!((f.fuel_index, f.fire_index), (0, 1));
    assert_eq!(
        f.attached[0],
        HearthSprite::Fuel {
            soff_x: 10 * TILE_SCALE,
            soff_y: 90 * TILE_SCALE
        }
    );
    // AttachAnim(…, 15, -8): soff (-30, 16), then the 128x256 frame at
    // scale 0.5 centred on the offsets: +(64 - 32), +(192 - 128).
    let HearthSprite::Anim(a) = f.attached[1] else {
        panic!()
    };
    assert_eq!((a.kind, a.anim, a.frame), (AnimKind::Fire, 1, frame));
    assert_eq!((a.soff_x, a.soff_y), (-30 + 32, 16 + 64));
    assert!(f.fire_uses_our_depth);
    assert_eq!(f.overlay, None);
    // Half an hour of fuel: Lerp(1, 8, 0.5) = 4.5 -> 4.
    let [lr, lg, lb] = FIRE_COLOR;
    assert_eq!(
        f.light,
        Some(FireLight {
            r: lr,
            g: lg,
            b: lb,
            radius: 4
        })
    );
    // An hour or more: radius 8; empty: 1.
    assert_eq!(fireplace_light_radius(600), 8);
    assert_eq!(fireplace_light_radius(0), 1);
    f.update(&wall_fireplace(true, 90), 0.0, &r, &frame_size);
    assert_eq!(f.light.map(|l| l.radius), Some(8));

    // Put out: the flame, overlay and light go, the logs stay.
    f.update(&wall_fireplace(false, 90), 0.0, &r, &frame_size);
    assert_eq!((f.fuel_index, f.fire_index, f.attached.len()), (0, -1, 1));
    assert_eq!(f.light, None);
    // Burnt out: the logs go too.
    f.update(&wall_fireplace(false, 0), 0.0, &r, &frame_size);
    assert!(f.attached.is_empty());
    assert_eq!(f.fuel_index, -1);
}

#[test]
fn removing_the_logs_shifts_the_flame() {
    let r = Rand::new(4);
    let mut f = FireplaceState::new();
    f.update(&wall_fireplace(true, 5), 0.0, &r, &frame_size);
    f.update(&wall_fireplace(true, 0), 0.0, &r, &frame_size);
    assert_eq!((f.fuel_index, f.fire_index), (-1, 0));
    assert!(matches!(f.attached[..], [HearthSprite::Anim(_)]));
    // The flame animates at half a frame per tick.
    let HearthSprite::Anim(before) = f.attached[0] else {
        panic!()
    };
    f.update(&wall_fireplace(true, 0), 1.0, &r, &frame_size);
    let HearthSprite::Anim(after) = f.attached[0] else {
        panic!()
    };
    let want = if (before.frame + 0.5) as i32 >= 30 {
        0.0
    } else {
        before.frame + 0.5
    };
    assert_eq!(after.frame, want);
}

#[test]
fn campfires_and_drums_swap_their_overlay() {
    let r = Rand::new(5);
    let mut f = FireplaceState::new();
    let input = FireplaceInput {
        lit: true,
        fuel_amount: 10,
        sprite_name: "camping_03_16",
        container_type: Some("campfire"),
        ..Default::default()
    };
    f.update(&input, 0.0, &r, &frame_size);
    assert_eq!(f.overlay.as_deref(), Some("camping_03_17"));
    assert!(!f.fire_uses_our_depth);
    assert_eq!(f.fuel_index, -1);
    let HearthSprite::Anim(a) = f.attached[0] else {
        panic!()
    };
    assert_eq!(
        (a.soff_x, a.soff_y),
        (-2 * TILE_SCALE + 32, 13 * TILE_SCALE + 64)
    );

    let mut f = FireplaceState::new();
    let input = FireplaceInput {
        lit: true,
        sprite_name: "appliances_cooking_01_17",
        ..Default::default()
    };
    f.update(&input, 0.0, &r, &frame_size);
    assert_eq!(f.overlay.as_deref(), Some("appliances_cooking_01_on_17"));
    assert!(f.attached.is_empty());
    assert_eq!(f.fire_index, 0);

    let mut f = FireplaceState::new();
    let input = FireplaceInput {
        lit: true,
        sprite_name: "crafted_01_30",
        metal_drum: true,
        ..Default::default()
    };
    f.update(&input, 0.0, &r, &frame_size);
    assert_eq!(f.overlay.as_deref(), Some("crafted_01_31"));
    let HearthSprite::Anim(a) = f.attached[0] else {
        panic!()
    };
    assert_eq!((a.soff_x, a.soff_y), (32, -10 * TILE_SCALE + 64));

    // Unlit with fuel: the brazier shows its logs overlay.
    let mut f = FireplaceState::new();
    let input = FireplaceInput {
        lit: false,
        fuel_amount: 10,
        sprite_name: "crafted_02_42",
        ..Default::default()
    };
    f.update(&input, 0.0, &r, &frame_size);
    assert_eq!(f.overlay.as_deref(), Some("crafted_02_43"));
}

fn sprites(out: &[DrawCmd]) -> Vec<(&Sprite, &WorldShader)> {
    out.iter()
        .filter_map(|c| match c {
            DrawCmd::WorldSprite(s, w) => Some((s, w)),
            _ => None,
        })
        .collect()
}

fn fireplace_square(depth: Option<Texture>) -> RenderSquare {
    let r = Rand::new(8);
    let mut f = FireplaceState::new();
    f.update(&wall_fireplace(true, 30), 0.0, &r, &frame_size);
    let main = SpriteInfo {
        name: "fixtures_fireplace_01_0".into(),
        texture: Some(tex("fixtures_fireplace_01_0", 128, 256, 20)),
        depth_texture: depth,
        ..Default::default()
    };
    let mut o = RenderObject::new(crate::ObjectKind::Plain, Some(Rc::new(main)));
    f.apply(&mut o, &textures);
    let mut sq = RenderSquare::new(10, 6, 0);
    sq.light_info = ColorInfo {
        r: 0.5,
        g: 0.4,
        b: 0.3,
        a: 1.0,
    };
    sq.objects.push(o);
    sq
}

#[test]
fn the_logs_go_in_the_chunk_texture_and_the_flames_per_frame() {
    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let depth = tex("DEPTH_fireplace", 128, 256, 30);
    let sq = fireplace_square(Some(depth));
    let obj = &sq.objects[0];
    assert!(has_animated_attachments(obj));
    assert_eq!(
        crate::cell::render_layer(&sq, obj),
        crate::cell::Layer::MinusFloor
    );
    let maps = Maps;
    let r = crate::ChunkRenderer::new(cam, &maps, (800, 600));

    // Chunk texture: the fireplace and its logs, both with its depth
    // texture; no flame.
    let chunk = ChunkTexture::new(1, 0, 0, 0, 0);
    let mut levels: crate::cell::LevelSquares<'_> = [None; 64];
    levels[2 + 6 * 8] = Some(&sq);
    let mut out = Vec::new();
    r.record(chunk, &[(0, levels)], &mut out);
    let s = sprites(&out);
    assert_eq!(s.len(), 2, "{s:?}");
    for (_, w) in &s {
        let WorldShader::TileDepth { depth, .. } = w else {
            panic!("{w:?}")
        };
        assert_eq!(depth.id, Some(30));
    }
    // The fireplace and its logs in the square's light: a sprite's tint
    // starts white, so neither comes out black.
    let lit = crate::color::to_abgr(0.5, 0.4, 0.3, 1.0);
    assert_eq!(s[0].0.color, [lit; 4]);
    assert_eq!(s[1].0.color, [lit; 4]);

    // Per frame: the flame alone, white, half size, its depth nudged in
    // front of the fireplace.
    let mut out = Vec::new();
    r.render_animated_attachments(&[&sq], &mut out);
    let s = sprites(&out);
    assert_eq!(s.len(), 1);
    let (flame, w) = s[0];
    assert_eq!(flame.color[0], crate::color::to_abgr(1.0, 1.0, 1.0, 1.0));
    assert_eq!(flame.x[1] - flame.x[0], 64.0);
    let cx = pz_render::iso::x_to_screen(10.0, 6.0) - cam.off_x as f32;
    let cy = pz_render::iso::y_to_screen(10.0, 6.0, 0.0, 0) - cam.off_y as f32;
    assert_eq!(flame.x[0], cx - 64.0 + 2.0);
    assert_eq!(flame.y[0], cy - 192.0 + 80.0);
    let WorldShader::TileDepth {
        depth, front, far, ..
    } = w
    else {
        panic!("{w:?}")
    };
    assert_eq!(depth.id, Some(30));
    let (cxs, cys) = (
        pz_render::iso::fastfloor(cam.x),
        pz_render::iso::fastfloor(cam.y),
    );
    assert_eq!(
        *far,
        pz_render::iso::square_depth(cxs, cys, 10.0, 6.0, 0.0) - 5.0e-5
    );
    assert_eq!(
        *front,
        pz_render::iso::square_depth(cxs, cys, 11.0, 7.0, 1.0) - 5.0e-5
    );

    // Without a depth texture of its own: the default one.
    let sq = fireplace_square(None);
    let mut out = Vec::new();
    r.render_animated_attachments(&[&sq], &mut out);
    let WorldShader::TileDepth { depth, .. } = sprites(&out)[0].1 else {
        panic!()
    };
    assert_eq!(depth.id, Some(40));
    // The translucent pass leaves non-translucent objects to this pass.
    let mut out = Vec::new();
    r.render_translucent_objects(&[&sq], 0, &mut out);
    assert!(sprites(&out).is_empty());
}

#[test]
fn a_lit_barbecue_smokes_without_a_depth_texture() {
    let mut b = BarbecueState::new();
    b.update(true, 0.0);
    assert_eq!(b.attached.len(), 1);
    assert_eq!(
        (b.attached[0].kind, b.attached[0].frame),
        (AnimKind::Smoke, 0.0)
    );
    b.update(true, 4.0);
    assert_eq!(b.attached[0].frame, 2.0);
    assert_eq!(b.attached[0].texture_name(), "Smoke_01_2");

    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let main = SpriteInfo {
        name: "appliances_cooking_01_35".into(),
        texture: Some(tex("appliances_cooking_01_35", 128, 256, 20)),
        ..Default::default()
    };
    let mut o = RenderObject::new(crate::ObjectKind::Plain, Some(Rc::new(main)));
    b.apply(&mut o, &|n| {
        (n == "Smoke_01_2").then(|| tex(n, 128, 256, 22))
    });
    let mut sq = RenderSquare::new(10, 6, 0);
    sq.light_info = ColorInfo {
        r: 0.5,
        g: 0.4,
        b: 0.3,
        a: 1.0,
    };
    sq.objects.push(o);
    let maps = Maps;
    let r = crate::ChunkRenderer::new(cam, &maps, (800, 600));
    let mut out = Vec::new();
    r.render_animated_attachments(&[&sq], &mut out);
    let s = sprites(&out);
    assert_eq!(s.len(), 1);
    let (smoke, w) = s[0];
    // Scale tileScale (2), which performRenderFrame takes back to 1 for
    // a 128x256 frame; soffY -58 * tileScale; lit by the square.
    assert_eq!(smoke.x[1] - smoke.x[0], 128.0);
    let cy = pz_render::iso::y_to_screen(10.0, 6.0, 0.0, 0) - cam.off_y as f32;
    assert_eq!(smoke.y[0], cy - 192.0 - 58.0 * TILE_SCALE as f32);
    assert_eq!(smoke.color[0], crate::color::to_abgr(0.5, 0.4, 0.3, 1.0));
    let soff = -58.0 * TILE_SCALE as f32 / 96.0 * pz_render::iso::LEVEL_DEPTH;
    let depth = pz_render::iso::chunk_depth_data(cam.chunk_x(), cam.chunk_y(), 1, 0, 0).depth_start
        + soff
        + 0.5;
    let z = depth * 2.0 - 1.0 + (pz_render::iso::calculate_depth(10.0, 6.0, 0.0) * 2.0 - 1.0);
    assert_eq!(*w, WorldShader::Default { z });

    b.update(false, 1.0);
    assert!(b.attached.is_empty());
}

/// The frame draws the flames after the chunk composite
/// (`renderAnimatedAttachments`), once per chunk.
#[test]
fn the_frame_draws_the_flames() {
    let cam = Camera::centred(10.0, 6.0, 0.0, 800, 600, 1.0);
    let sq = fireplace_square(Some(tex("DEPTH_fireplace", 128, 256, 30)));
    let maps = Maps;
    let r = crate::ChunkRenderer::new(cam, &maps, (800, 600));
    let mut levels: crate::cell::LevelSquares<'_> = [None; 64];
    levels[2 + 6 * 8] = Some(&sq);
    let chunks = [crate::frame::ChunkLevels {
        texture: ChunkTexture::new(1, 0, 0, 0, 0),
        levels: vec![(0, levels)],
        dirty: false,
    }];
    let frame = crate::frame::world_frame(
        &r,
        crate::frame::FrameSize {
            width: 800,
            height: 600,
        },
        &chunks,
        pz_render::ScreenParams {
            desaturation: 0.0,
            timer: 0.0,
        },
    );
    let mut alone = Vec::new();
    r.render_animated_attachments(&[&sq], &mut alone);
    let flames = sprites(&alone);
    assert_eq!(flames.len(), 1);
    let drawn = sprites(&frame);
    assert_eq!(
        drawn.iter().filter(|s| s.0 == flames[0].0).count(),
        1,
        "flame drawn once"
    );
}
