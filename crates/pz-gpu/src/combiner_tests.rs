//! `TextureCombiner` commands on the GPU, one program at a time, with the
//! pixels the original's shaders and blend functions give.

use pz_render::combiner::{CombinerCommand, CombinerShader};
use pz_render::{flags, DrawCmd, DrawList, Rgba, Texture, TextureManager};

use crate::{FrameParams, Gpu};

const BLEND: [u32; 4] = [770, 771, 1, 771];
const OVERLAY: [u32; 4] = [774, 771, 772, 771];
const HOLE: [u32; 4] = [770, 0, 1, 771];

fn gpu() -> Option<Gpu> {
    match Gpu::new_headless(16, 16) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn image(
    tm: &mut TextureManager,
    w: i32,
    h: i32,
    f: impl Fn(i32, i32) -> [u8; 4],
    flags: i32,
) -> Texture {
    // Laid out power-of-two wide and high, as the texture manager keeps
    // images; the padding is transparent.
    let (wh, hh) = (pz_render::image::pow2(w), pz_render::image::pow2(h));
    let px = (0..hh)
        .flat_map(|y| (0..wh).map(move |x| (x, y)))
        .flat_map(|(x, y)| if x < w && y < h { f(x, y) } else { [0; 4] })
        .collect();
    tm.texture_from_pixels(&format!("img{}", tm.data_len()), w, h, px, flags)
}

fn solid(tm: &mut TextureManager, w: i32, h: i32, c: [u8; 4]) -> Texture {
    image(tm, w, h, |_, _| c, 0)
}

fn cmd(
    tex: &Texture,
    shader: CombinerShader,
    mask: Option<&Texture>,
    params: &[(&str, f32)],
    blend: [u32; 4],
) -> CombinerCommand {
    CombinerCommand {
        tex: tex.clone(),
        mask: mask.cloned(),
        shader,
        params: params.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
        blend,
        rect: None,
    }
}

fn blit(tex: &Texture) -> CombinerCommand {
    cmd(tex, CombinerShader::Blit, None, &[], BLEND)
}

/// Combines and reads back the result (rows in image order).
fn run(g: &mut Gpu, tm: &mut TextureManager, cmds: &[CombinerCommand]) -> (Texture, Vec<[u8; 4]>) {
    let t = g.combine_textures(cmds, tm);
    let px = g.read_combined(&t).unwrap();
    (t, px.chunks(4).map(|c| c.try_into().unwrap()).collect())
}

#[test]
fn blit_keeps_rows_and_blends_over_transparent_black() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    // Rows of different colours, 3 x 4: the result is 4 x 4 (power of
    // two), rows in the same order; the padding column is transparent.
    let rows = image(
        &mut tm,
        3,
        4,
        |_, y| [(y * 60) as u8, 255 - (y * 60) as u8, 7, 255],
        0,
    );
    let (t, px) = run(&mut g, &mut tm, &[blit(&rows)]);
    assert_eq!((t.width, t.height), (4, 4));
    for y in 0..4 {
        assert_eq!(
            px[y * 4],
            [(y * 60) as u8, 255 - (y * 60) as u8, 7, 255],
            "row {y}"
        );
        assert_eq!(px[y * 4 + 3], [0, 0, 0, 0]);
    }
    // SRC_ALPHA, ONE_MINUS_SRC_ALPHA for colour, ONE, ONE_MINUS_SRC_ALPHA
    // for alpha: a half-transparent texel lands premultiplied.
    let half = solid(&mut tm, 4, 4, [200, 100, 50, 128]);
    let quarter = solid(&mut tm, 4, 4, [0, 0, 255, 64]);
    let (_, px) = run(&mut g, &mut tm, &[blit(&half)]);
    assert_eq!(px[5], [100, 50, 25, 128]);
    let (_, px) = run(&mut g, &mut tm, &[blit(&half), blit(&quarter)]);
    assert_eq!(px[5], [75, 37, 83, 160]);
}

#[test]
fn rects_are_placed_in_the_256_space_from_the_bottom_row_up() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let black = solid(&mut tm, 8, 8, [0, 0, 0, 255]);
    // A 2 x 2 decal, red over green, sampled GL_NEAREST.
    let decal = image(
        &mut tm,
        2,
        2,
        |_, y| {
            if y == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            }
        },
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    let mut d = blit(&decal);
    d.rect = Some([128, 0, 128, 128]);
    let (_, px) = run(&mut g, &mut tm, &[blit(&black), d]);
    for y in 0..8 {
        for x in 0..8 {
            let want = match (x >= 4, y) {
                (true, 4 | 5) => [255, 0, 0, 255],
                (true, 6 | 7) => [0, 255, 0, 255],
                _ => [0, 0, 0, 255],
            };
            assert_eq!(px[y * 8 + x], want, "{x},{y}");
        }
    }
}

#[test]
fn hue_change_tints_and_shifts_hue() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let c = solid(&mut tm, 4, 4, [255, 200, 100, 255]);
    let tint = cmd(
        &c,
        CombinerShader::HueChange,
        None,
        &[("R", 0.25), ("G", 1.0), ("B", 0.5)],
        BLEND,
    );
    assert_eq!(run(&mut g, &mut tm, &[tint]).1[0], [64, 200, 50, 255]);
    let red = solid(&mut tm, 4, 4, [255, 0, 0, 255]);
    for (h, want) in [
        (1.0 / 3.0, [0, 255, 0, 255]),
        (2.0 / 3.0, [0, 0, 255, 255]),
        (1.5, [0, 255, 255, 255]),
        (-2.5, [0, 255, 255, 255]),
    ] {
        let hue = cmd(
            &red,
            CombinerShader::HueChange,
            None,
            &[("HueChange", h)],
            BLEND,
        );
        assert_eq!(run(&mut g, &mut tm, &[hue]).1[0], want, "hue {h}");
    }
}

#[test]
fn blood_and_dirt_overlays_multiply_the_texture_below() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let base = solid(&mut tm, 4, 4, [200, 150, 100, 255]);
    let overlay = solid(&mut tm, 4, 4, [30, 80, 40, 255]);
    let mask = |tm: &mut TextureManager, a: u8| solid(tm, 4, 4, [255, 255, 255, a]);
    let cases = [
        (CombinerShader::OverlayMask, 1.0, 255, [100, 47, 16, 255]),
        (CombinerShader::OverlayMask, 0.5, 128, [125, 72, 37, 255]),
        (CombinerShader::DirtMask, 1.0, 255, [24, 47, 16, 255]),
        (CombinerShader::DirtMask, 1.2, 64, [77, 78, 41, 255]),
    ];
    for (shader, intensity, a, want) in cases {
        let m = mask(&mut tm, a);
        let mut params = vec![("intensity", intensity)];
        if shader == CombinerShader::OverlayMask {
            params.push(("bloodDark", 0.5));
        }
        let o = cmd(&overlay, shader, Some(&m), &params, OVERLAY);
        let (_, px) = run(&mut g, &mut tm, &[blit(&base), o]);
        assert_eq!(px[0], want, "{shader:?} {intensity} {a}");
    }
}

#[test]
fn body_masks_gate_alpha_with_a_nearest_sampled_mask() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let white = solid(&mut tm, 4, 4, [255, 255, 255, 255]);
    let red = solid(&mut tm, 4, 4, [255, 0, 0, 255]);
    // 2 x 2 mask over a 4 x 4 result: each mask texel covers a 2 x 2
    // quadrant, with no interpolation.
    let mask = image(
        &mut tm,
        2,
        2,
        |x, y| [0, 0, 0, if x == y { 255 } else { 0 }],
        0,
    );
    let (_, px) = run(
        &mut g,
        &mut tm,
        &[cmd(
            &white,
            CombinerShader::BodyMask,
            Some(&mask),
            &[],
            BLEND,
        )],
    );
    for y in 0..4 {
        for x in 0..4 {
            let inside = (x < 2) == (y < 2);
            assert_eq!(
                px[y * 4 + x],
                if inside { [255; 4] } else { [0; 4] },
                "{x},{y}"
            );
        }
    }
    let tint = cmd(
        &white,
        CombinerShader::BodyMaskTint,
        Some(&mask),
        &[("R", 0.25), ("G", 1.0), ("B", 0.5)],
        BLEND,
    );
    assert_eq!(run(&mut g, &mut tm, &[tint]).1[0], [64, 255, 128, 255]);
    let hue = cmd(
        &red,
        CombinerShader::BodyMaskHue,
        Some(&mask),
        &[("HueChange", 2.0 / 3.0)],
        BLEND,
    );
    let (_, px) = run(&mut g, &mut tm, &[hue]);
    assert_eq!((px[0], px[2]), ([0, 0, 255, 255], [0, 0, 0, 0]));
}

#[test]
fn add_hole_cuts_alpha_between_the_cutoffs() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let c = solid(&mut tm, 4, 4, [200, 100, 50, 255]);
    let mask = image(
        &mut tm,
        4,
        4,
        |x, _| [0, 0, 0, [0, 102, 255, 255][x as usize]],
        0,
    );
    let (_, px) = run(
        &mut g,
        &mut tm,
        &[cmd(&c, CombinerShader::AddHole, Some(&mask), &[], HOLE)],
    );
    assert_eq!(
        &px[..3],
        &[[200, 100, 50, 255], [86, 43, 21, 109], [0, 0, 0, 0]]
    );
}

#[test]
fn remove_hole_pads_the_mask_by_one_texel() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let red = solid(&mut tm, 64, 64, [255, 0, 0, 255]);
    let mask = image(
        &mut tm,
        64,
        64,
        |x, y| [0, 0, 0, if (x, y) == (10, 20) { 255 } else { 0 }],
        0,
    );
    let (_, px) = run(
        &mut g,
        &mut tm,
        &[cmd(
            &red,
            CombinerShader::RemoveHole,
            Some(&mask),
            &[],
            BLEND,
        )],
    );
    for y in 0..64usize {
        for x in 0..64usize {
            let near = x.abs_diff(10) <= 1 && y.abs_diff(20) <= 1;
            assert_eq!(
                px[y * 64 + x],
                if near { [255, 0, 0, 255] } else { [0; 4] },
                "{x},{y}"
            );
        }
    }
}

#[test]
fn results_chain_draw_as_textures_and_return_to_the_pool() {
    let Some(mut g) = gpu() else { return };
    let mut tm = TextureManager::new();
    let rows = image(
        &mut tm,
        4,
        4,
        |_, y| {
            if y < 2 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        },
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    let (first, _) = run(&mut g, &mut tm, &[blit(&rows)]);
    // A job drawing an earlier job's result.
    let (second, px) = run(&mut g, &mut tm, &[blit(&first)]);
    assert_eq!((px[0], px[15]), ([255, 0, 0, 255], [0, 0, 255, 255]));
    // Drawn like any texture (at one texel a pixel): the result and its
    // source side by side show the same rows.
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::Quad {
        texture: Some(rows.clone()),
        x: 0.0,
        y: 0.0,
        w: 4.0,
        h: 4.0,
        color: Rgba(1.0, 1.0, 1.0, 1.0),
    });
    dl.cmds.push(DrawCmd::Quad {
        texture: Some(second.clone()),
        x: 8.0,
        y: 0.0,
        w: 4.0,
        h: 4.0,
        color: Rgba(1.0, 1.0, 1.0, 1.0),
    });
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..FrameParams::ui(16, 16)
            },
            &dl,
        )],
        &mut tm,
    );
    let screen = g.read_pixels();
    let at =
        |x: usize, y: usize| -> [u8; 4] { screen[(y * 16 + x) * 4..][..4].try_into().unwrap() };
    for y in 0..4 {
        assert_eq!(at(1, y), at(9, y), "row {y}");
    }
    assert_eq!(at(9, 1)[..3], [255, 0, 0]);
    assert_eq!(at(9, 2)[..3], [0, 0, 255]);
    // releaseTexture, then the next combine of that size reuses it.
    let id = first.id;
    g.release_combined(first);
    let grey = solid(&mut tm, 4, 4, [9, 8, 7, 255]);
    let (third, px) = run(&mut g, &mut tm, &[blit(&grey)]);
    assert_eq!(third.id, id);
    assert_eq!(px[0], [9, 8, 7, 255]);
}
