use super::*;
use pz_gpu::FrameParams;
use pz_render::{DrawCmd, DrawList, Rgba, TextureManager};

fn gpu(w: u32, h: u32) -> Option<Gpu> {
    match Gpu::new_headless(w, h) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn params() -> ScreenParams {
    ScreenParams {
        desaturation: 0.0,
        timer: 0.0,
        ..Default::default()
    }
}

fn px(t: &Thumbnail, x: i32, y: i32) -> [u8; 4] {
    let i = ((y * t.width + x) * 4) as usize;
    t.rgba[i..i + 4].try_into().unwrap()
}

fn full(w: i32, h: i32) -> PlayerScreen {
    PlayerScreen {
        left: 0,
        top: 0,
        width: w,
        height: h,
    }
}

/// A world render at 640x480: red above row 240, blue below, a green
/// block left of the middle. The thumbnail is 256x256 from the middle,
/// bottom-up: its first rows are blue, its last red, the green on the
/// left; `thumb.png` then has red on top.
#[test]
fn thumbnail_of_a_world_frame() {
    let Some(mut g) = gpu(640, 480) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin {
        width: 640,
        height: 480,
    });
    dl.render_rect(0, 0, 640, 240, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.render_rect(0, 240, 640, 240, Rgba(0.0, 0.0, 1.0, 1.0));
    dl.render_rect(200, 200, 60, 80, Rgba(0.0, 1.0, 0.0, 1.0));
    dl.cmds.push(DrawCmd::OffscreenEnd);
    g.render_frame(&[(FrameParams::ui(640, 480), &dl)], &mut tm);
    let t = capture_thumbnail(&mut g, full(640, 480), full(640, 480), params()).expect("thumbnail");
    assert_eq!((t.width, t.height, t.rgba.len()), (256, 256, 256 * 256 * 4));
    // Square x 192..448, y 112..368 of the view.
    let (bottom, top) = (px(&t, 200, 5), px(&t, 200, 250));
    assert!(bottom[2] > 200 && bottom[0] < 50, "{bottom:?}");
    assert!(top[0] > 200 && top[2] < 50, "{top:?}");
    // Green at view x 200..260: thumbnail columns 8..68.
    let green = px(&t, 30, 128);
    assert!(green[1] > 200 && green[0] < 50, "{green:?}");
    let right = px(&t, 120, 128);
    assert!(right[1] < 50, "{right:?}");

    let dir = std::env::temp_dir().join(format!(
        "pz-thumb-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    thumbnail::save_thumbnail(&dir, &t).unwrap();
    let png = std::fs::read(dir.join(thumbnail::THUMB_FILE)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 256);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 256);
}

/// No offscreen buffer: player 0's back buffer, at most its size.
#[test]
fn without_offscreen_buffer_reads_the_screen() {
    let Some(mut g) = gpu(200, 100) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.render_rect(0, 0, 200, 50, Rgba(1.0, 0.0, 0.0, 1.0));
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 1.0, 1.0]),
                ..FrameParams::ui(200, 100)
            },
            &dl,
        )],
        &mut tm,
    );
    let t = capture_thumbnail(&mut g, full(200, 100), full(200, 100), params()).expect("thumbnail");
    assert_eq!((t.width, t.height), (200, 100));
    assert_eq!(px(&t, 10, 0), [0, 0, 255, 255]);
    assert_eq!(px(&t, 10, 99), [255, 0, 0, 255]);
}
