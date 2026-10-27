use super::*;
use pz_render::{flags, Sprite, Texture};

fn gpu(width: u32, height: u32) -> Option<Gpu> {
    match Gpu::new_headless(width, height) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            eprintln!("skipping GPU pixel regression: {error}");
            None
        }
    }
}

fn sprite(texture: &Texture, x: f32, width: f32, height: f32, uv: [[f32; 2]; 4]) -> Sprite {
    Sprite {
        texture: Some(texture.clone()),
        x: [x, x + width, x + width, x],
        y: [0.0, 0.0, height, height],
        u: uv.map(|p| p[0]),
        v: uv.map(|p| p[1]),
        color: [u32::MAX; 4],
        flipped: false,
    }
}

fn pixel(pixels: &[u8], width: usize, x: usize) -> [u8; 4] {
    pixels[x * 4..x * 4 + 4]
        .try_into()
        .unwrap_or_else(|_| panic!("missing pixel {x} of {width}"))
}

fn near(actual: [u8; 4], expected: [u8; 3]) {
    assert!(
        actual[..3]
            .iter()
            .zip(expected)
            .all(|(&a, b)| a.abs_diff(b) <= 1),
        "{actual:?} != {expected:?}"
    );
    assert_eq!(actual[3], 255);
}

fn params(width: u32, height: u32) -> FrameParams {
    FrameParams {
        clear: Some([0.0, 0.0, 0.0, 1.0]),
        ..FrameParams::ui(width, height)
    }
}

fn four_colors() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255,
    ]
}

#[test]
fn captured_sprite_samplers_preserve_clamp_repeat_and_later_default_filtering() {
    let Some(mut gpu) = gpu(10, 2) else { return };
    let mut textures = TextureManager::new();
    let texture = textures.texture_from_pixels(
        "map-sampler-shared",
        2,
        2,
        four_colors(),
        flags::CLAMP_TO_EDGE,
    );
    let id = texture.id.unwrap();
    let baseline_flags = textures.data(id).unwrap().flags;
    let mut list = DrawList::default();
    // Linear at the red/blue boundary, then two different samplers over
    // the very same image and out-of-range UVs, then ordinary linear again.
    list.cmds.push(DrawCmd::Sprite(sprite(
        &texture,
        0.0,
        2.0,
        2.0,
        [[0.5, 0.25]; 4],
    )));
    list.cmds.push(DrawCmd::SampledSprite {
        sprite: sprite(&texture, 2.0, 2.0, 2.0, [[1.25, 1.25]; 4]),
        nearest: true,
        repeat: false,
    });
    list.cmds.push(DrawCmd::SampledSprite {
        sprite: sprite(&texture, 4.0, 2.0, 2.0, [[1.25, 1.25]; 4]),
        nearest: true,
        repeat: true,
    });
    list.cmds.push(DrawCmd::SampledSprite {
        sprite: sprite(&texture, 6.0, 2.0, 2.0, [[1.5, 1.25]; 4]),
        nearest: false,
        repeat: true,
    });
    list.cmds.push(DrawCmd::Sprite(sprite(
        &texture,
        8.0,
        2.0,
        2.0,
        [[0.5, 0.25]; 4],
    )));
    gpu.render_frame(&[(params(10, 2), &list)], &mut textures);
    let pixels = gpu.read_pixels();
    near(pixel(&pixels, 10, 0), [128, 0, 128]);
    near(pixel(&pixels, 10, 2), [255, 255, 255]);
    near(pixel(&pixels, 10, 4), [255, 0, 0]);
    near(pixel(&pixels, 10, 6), [128, 0, 128]);
    near(pixel(&pixels, 10, 8), [128, 0, 128]);
    assert_eq!(gpu.filters[&id], (false, false, false, true));
    assert_eq!(textures.data(id).unwrap().flags, baseline_flags);
    assert_eq!(gpu.sampled.len(), 3);
    assert!(gpu.sampled.keys().all(|(sampled_id, _)| *sampled_id == id));
}

#[test]
fn captured_nearest_uses_base_level_and_respecified_image_retires_derived_bindings() {
    let Some(mut gpu) = gpu(2, 1) else { return };
    let mut textures = TextureManager::new();
    let texture = textures.texture_from_pixels(
        "map-sampler-respecified",
        2,
        2,
        four_colors(),
        flags::CLAMP_TO_EDGE | flags::MIPMAPS,
    );
    let id = texture.id.unwrap();
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut list = DrawList::default();
    list.cmds
        .push(DrawCmd::Sprite(sprite(&texture, 0.0, 1.0, 1.0, uv)));
    list.cmds.push(DrawCmd::SampledSprite {
        sprite: sprite(&texture, 1.0, 1.0, 1.0, uv),
        nearest: true,
        repeat: false,
    });
    gpu.render_frame(&[(params(2, 1), &list)], &mut textures);
    let pixels = gpu.read_pixels();
    // The ordinary minified image uses its averaged mip. Native
    // GL_NEAREST must instead select the white texel in level zero.
    near(pixel(&pixels, 2, 0), [128, 128, 128]);
    near(pixel(&pixels, 2, 1), [255, 255, 255]);
    assert_eq!(gpu.filters[&id], (false, false, true, true));
    assert!(gpu.sampled.contains_key(&(id, (true, true, false, true))));

    // glTexImage2D on the same actual texture ID must invalidate the
    // derived binding too, even when its effective sampler key is equal.
    textures.tex_image(id, 2, 2, [0, 255, 0, 255].repeat(4));
    gpu.render_frame(&[(params(2, 1), &list)], &mut textures);
    let pixels = gpu.read_pixels();
    near(pixel(&pixels, 2, 0), [0, 255, 0]);
    near(pixel(&pixels, 2, 1), [0, 255, 0]);
    assert_eq!(gpu.sampled.len(), 1);
    assert_eq!(gpu.versions[&id], textures.data(id).unwrap().version);

    textures.destroy_data(id);
    gpu.render_frame(&[(params(2, 1), &list)], &mut textures);
    assert!(!gpu.sampled.keys().any(|(sampled_id, _)| *sampled_id == id));
    assert_eq!(pixel(&gpu.read_pixels(), 2, 1), [0, 0, 0, 255]);
}

#[test]
fn captured_sampler_uses_actual_combined_image_and_reused_sampler_baseline() {
    use pz_render::combiner::{CombinerCommand, CombinerShader};
    let Some(mut gpu) = gpu(2, 2) else { return };
    let mut textures = TextureManager::new();
    let command = |texture: &Texture| CombinerCommand {
        tex: texture.clone(),
        mask: None,
        shader: CombinerShader::Blit,
        params: Vec::new(),
        blend: [770, 771, 1, 771],
        rect: None,
    };
    let red =
        textures.texture_from_pixels("sampled-combined-red", 2, 2, [255, 0, 0, 255].repeat(4), 0);
    let combined = gpu.combine_textures(&[command(&red)], &mut textures);
    let id = combined.id.unwrap();
    let draw = |texture: &Texture| DrawList {
        cmds: vec![DrawCmd::SampledSprite {
            sprite: sprite(texture, 0.0, 2.0, 2.0, [[1.25, 1.25]; 4]),
            nearest: true,
            repeat: false,
        }],
    };
    gpu.render_frame(&[(params(2, 2), &draw(&combined))], &mut textures);
    near(pixel(&gpu.read_pixels(), 2, 0), [255, 0, 0]);
    assert_eq!(gpu.filters[&id], (false, false, false, false));
    assert!(gpu.sampled.contains_key(&(id, (true, true, false, false))));

    // The real combiner pool reuses the same image, changing its native
    // baseline to clamp. Old derived bindings must leave with that state.
    gpu.release_combined(combined);
    let blue =
        textures.texture_from_pixels("sampled-combined-blue", 2, 2, [0, 0, 255, 255].repeat(4), 0);
    let reused = gpu.combine_textures(&[command(&blue)], &mut textures);
    assert_eq!(reused.id, Some(id));
    assert_eq!(gpu.filters[&id], (false, false, false, true));
    assert!(!gpu.sampled.keys().any(|(sampled_id, _)| *sampled_id == id));
    gpu.render_frame(&[(params(2, 2), &draw(&reused))], &mut textures);
    near(pixel(&gpu.read_pixels(), 2, 0), [0, 0, 255]);
    assert!(gpu.sampled.contains_key(&(id, (true, true, false, true))));
}
