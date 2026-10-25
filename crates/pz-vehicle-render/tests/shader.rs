//! The vehicle shaders on the GPU with synthetic textures: a mask zone
//! lights up only when its enable is set (checking the transposed upload
//! of the enable matrices), the paint fills transparent skin texels, the
//! uninstall zone darkens, and the wheel shader takes its texture
//! coordinates from the second slot.

#![allow(clippy::field_reassign_with_default)]

use std::rc::Rc;

use pz_render::model::{
    MeshData, MeshKind, ModelDraw, ModelLight, ModelPass, VehicleDraw, VehicleShader,
    VehicleUniforms,
};
use pz_render::{DrawCmd, Texture, TextureManager};

fn solid(tm: &mut TextureManager, name: &str, rgba: [u8; 4]) -> Texture {
    tm.texture_from_pixels(name, 4, 4, rgba.repeat(16), 0)
}

/// A screen-filling quad in clip space, both UV slots `(0.5, 0.5)`.
fn quad() -> Rc<MeshData> {
    let mut v = Vec::new();
    for (x, y) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        v.extend_from_slice(&[x, y, 0.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.5, 0.5]);
    }
    Rc::new(MeshData {
        id: 7 << 40,
        kind: MeshKind::StaticUv2,
        vertices: v,
        indices: vec![0, 2, 1, 0, 3, 2],
    })
}

fn draw(shader: VehicleShader, textures: [Option<Texture>; 10], u: VehicleUniforms) -> DrawCmd {
    let identity = pz_iso_render::trees::Mat4::identity().0;
    DrawCmd::VehicleModel(Box::new(VehicleDraw {
        base: ModelDraw {
            mesh: quad(),
            texture: None,
            mvp: identity,
            transform: identity,
            palette: Rc::new(Vec::new()),
            target_depth: 0.5,
            final_scale: 1.0,
            uv_scale: [1.0, 1.0],
            lights: [ModelLight {
                direction: [0.0, 1.0, 0.0],
                colour: [0.0; 3],
            }; 5],
            ambient: [1.0; 3],
            tint: [1.0; 3],
            hue: 0.0,
            alpha: 1.0,
            pass: ModelPass::Character { cull: true },
        },
        shader,
        noreflect: true,
        invert_x: false,
        textures,
        uniforms: u,
    }))
}

fn render(gpu: &mut pz_gpu::Gpu, tm: &mut TextureManager, cmd: DrawCmd) -> [u8; 4] {
    let cmds = vec![
        DrawCmd::DepthMask(true),
        DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT),
        DrawCmd::GlEnable(pz_render::gl::DEPTH_TEST),
        cmd,
    ];
    let list = pz_render::DrawList { cmds };
    gpu.render_frame(
        &[(
            pz_gpu::FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..pz_gpu::FrameParams::ui(16, 16)
            },
            &list,
        )],
        tm,
    );
    let p = gpu.read_pixels();
    let i = (8 * 16 + 8) * 4;
    [p[i], p[i + 1], p[i + 2], p[i + 3]]
}

#[test]
fn zone_enables_paint_and_uninstall() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(16, 16) else {
        return;
    };
    let mut tm = TextureManager::new();
    // Skin: opaque grey. Mask: zone 18 "Lights R H" (0.25, 0, 0) —
    // texen2[0][1], enabled by TextureLightsEnables2[4] (HeadlightRight).
    let skin = solid(&mut tm, "skin", [128, 128, 128, 255]);
    let mask = solid(&mut tm, "mask", [64, 0, 0, 255]);
    let lights = solid(&mut tm, "lights", [255, 255, 0, 255]);
    let mut t: [Option<Texture>; 10] = Default::default();
    t[0] = Some(skin.clone());
    t[2] = Some(mask.clone());
    t[3] = Some(lights.clone());
    let mut u = VehicleUniforms::default();
    let off = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::MultiUv, t.clone(), u.clone()),
    );
    assert!(
        (off[0] as i32 - 128).abs() <= 2 && (off[2] as i32 - 128).abs() <= 2,
        "unlit {off:?}"
    );
    // The transposed slot (index 1, texen2[1][0]) does not light zone 18.
    u.texture_lights_enables[1][1] = 1.0;
    let wrong = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::MultiUv, t.clone(), u.clone()),
    );
    assert_eq!(wrong, off);
    u.texture_lights_enables[1][4] = 1.0;
    let on = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::MultiUv, t.clone(), u.clone()),
    );
    assert!(on[0] > 250 && on[1] > 250 && on[2] < 5, "lit {on:?}");

    // Uninstall (zone 1, texen1[0][0] <- TextureUninstall1[0]): 0.2 grey.
    let head = solid(&mut tm, "mask_head", [255, 0, 0, 255]);
    t[2] = Some(head);
    let mut u = VehicleUniforms::default();
    u.texture_uninstall[0][0] = 1.0;
    let gone = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::MultiUv, t.clone(), u),
    );
    assert!((gone[0] as i32 - 51).abs() <= 2, "uninstalled {gone:?}");

    // Paint: a fully transparent skin texel takes the paint's hue with
    // the texel's saturation and value moved by the paint's.
    let clear = solid(&mut tm, "skin_clear", [128, 128, 128, 0]);
    t[0] = Some(clear);
    let mut u = VehicleUniforms::default();
    u.paint_color = [0.0, 1.0, 0.5, 1.0];
    let painted = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::MultiUv, t.clone(), u.clone()),
    );
    assert!(
        painted[0] > 120
            && (painted[1] as i32 - 64).abs() <= 3
            && (painted[2] as i32 - 64).abs() <= 3,
        "painted {painted:?}"
    );
    // vehicle_norandom_multiuv keeps the texel.
    let kept = render(
        &mut gpu,
        &mut tm,
        draw(VehicleShader::NoRandomMultiUv, t, u),
    );
    assert!((kept[1] as i32 - 128).abs() <= 2, "norandom {kept:?}");
}

#[test]
fn wheel_shader_lit_texture() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(16, 16) else {
        return;
    };
    let mut tm = TextureManager::new();
    let tex = solid(&mut tm, "wheel", [200, 100, 50, 255]);
    let mut t: [Option<Texture>; 10] = Default::default();
    t[0] = Some(tex);
    let mut u = VehicleUniforms::default();
    u.paint_color[3] = 1.0;
    let p = render(&mut gpu, &mut tm, draw(VehicleShader::Wheel, t, u));
    // Tint white desaturated by 0.3 is white; ambient 1.
    assert!(
        (p[0] as i32 - 200).abs() <= 2 && (p[1] as i32 - 100).abs() <= 2,
        "{p:?}"
    );
}
