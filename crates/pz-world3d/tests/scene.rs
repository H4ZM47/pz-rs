//! The whole 3D path on the GPU: a first-person camera over a floor
//! facing a wall, through `scene_frame` (offscreen buffer and screen pass).

use pz_gpu::{FrameParams, Gpu};
use pz_iso_render::frame::FrameSize;
use pz_render::{DrawList, ScreenParams, TextureManager};
use pz_world3d::camera::Camera3D;
use pz_world3d::frame::{scene_frame, Scene};
use pz_world3d::mesh::Face;
use pz_world3d::to_model;

const W: u32 = 64;
const H: u32 = 48;

fn face(tex: &pz_render::Texture, corners: [[f32; 3]; 4], origin: [f32; 3], color: u32) -> Face {
    Face {
        texture: tex.clone(),
        pos: corners.map(|c| to_model(c, origin)),
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        color: [color; 4],
        translucent: false,
    }
}

fn px(p: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    p[i..i + 4].try_into().unwrap()
}

#[test]
fn floor_below_wall_ahead_sky_above() {
    let mut g = match Gpu::new_headless(W, H) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let o = [100.5, 100.5, 0.0];
    const RED: u32 = 0xFF00_00FF;
    const GREEN: u32 = 0xFF00_FF00;
    let faces = [
        // The floor around the player.
        face(
            &white,
            [
                [90.0, 90.0, 0.0],
                [111.0, 90.0, 0.0],
                [111.0, 111.0, 0.0],
                [90.0, 111.0, 0.0],
            ],
            o,
            RED,
        ),
        // A wall three squares north, one level high: green on the west
        // half, white on the east half.
        face(
            &white,
            [
                [90.0, 97.0, 0.0],
                [100.5, 97.0, 0.0],
                [100.5, 97.0, 1.0],
                [90.0, 97.0, 1.0],
            ],
            o,
            GREEN,
        ),
        face(
            &white,
            [
                [100.5, 97.0, 0.0],
                [111.0, 97.0, 0.0],
                [111.0, 97.0, 1.0],
                [100.5, 97.0, 1.0],
            ],
            o,
            0xFFFF_FFFF,
        ),
    ];
    // Facing north.
    let cam = Camera3D::first_person(o, o, -std::f32::consts::FRAC_PI_2, 0.0);
    let scene = Scene {
        size: FrameSize {
            width: W,
            height: H,
        },
        view_proj: cam.view_proj(W as f32 / H as f32).0,
        sky: [0.0, 0.0, 1.0, 1.0],
        eye: cam.eye,
        faces: &faces,
        models: &[],
        screen: ScreenParams::default(),
        crosshair: false,
    };
    let dl = DrawList {
        cmds: scene_frame(&scene),
        ..Default::default()
    };
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..FrameParams::ui(W, H)
            },
            &dl,
        )],
        &mut tm,
    );
    let p = g.read_pixels();
    let (west, east) = (px(&p, W / 4, H / 2), px(&p, 3 * W / 4, H / 2));
    let (bottom, top) = (px(&p, W / 2, H - 2), px(&p, W / 2, 1));
    // Facing north, west is on the left and east on the right (not mirrored).
    assert!(
        west[1] > 200 && west[0] < 60,
        "west wall on the left: {west:?}"
    );
    assert!(
        east[0] > 200 && east[1] > 200 && east[2] > 200,
        "east wall on the right: {east:?}"
    );
    assert!(bottom[0] > 200 && bottom[1] < 60, "floor below: {bottom:?}");
    assert!(
        top[2] > 200 && top[0] < 60 && top[1] < 60,
        "sky above: {top:?}"
    );
}
