use super::*;
use pz_render::Rgba;

fn gpu(w: u32, h: u32) -> Option<Gpu> {
    match Gpu::new_headless(w, h) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn px(p: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    p[i..i + 4].try_into().unwrap()
}

fn params() -> ScreenParams {
    ScreenParams {
        desaturation: 0.0,
        timer: 0.0,
        ..Default::default()
    }
}

/// A synthetic world frame: an asymmetric layout in the offscreen buffer,
/// then the screen pass that shows it.
fn world_frame(w: u32, h: u32) -> DrawList {
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin {
        width: w,
        height: h,
    });
    dl.render_rect(0, 0, w as i32, h as i32, Rgba(0.3, 0.3, 0.3, 1.0));
    dl.render_rect(140, 80, 60, 40, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.render_rect(300, 200, 100, 90, Rgba(0.0, 1.0, 0.0, 1.0));
    dl.render_rect(200, 150, 30, 150, Rgba(0.0, 0.0, 1.0, 1.0));
    dl.render_rect(0, 0, w as i32, 10, Rgba(1.0, 1.0, 0.0, 1.0));
    // Ramps, which the bicubic filter keeps exactly: red down rows
    // 100..150 at x 330..370, green along columns 140..190 at y 250..280.
    for i in 0..50 {
        dl.render_rect(
            330,
            100 + i,
            40,
            1,
            Rgba(5.0 * i as f32 / 255.0, 0.5, 0.5, 1.0),
        );
        dl.render_rect(
            140 + i,
            250,
            1,
            30,
            Rgba(0.5, 5.0 * i as f32 / 255.0, 0.5, 1.0),
        );
    }
    dl.cmds.push(DrawCmd::OffscreenEnd);
    dl.cmds.push(DrawCmd::ScreenPass {
        x: 0.0,
        y: 0.0,
        w: w as f32,
        h: h as f32,
        tex: [0.0, 0.0, w as f32, h as f32],
        params: params(),
    });
    dl
}

/// The thumbnail is 256x256, bottom-up, and (for a full-screen view) the
/// graded middle square the frame's screen pass put on screen.
#[test]
fn thumbnail_is_the_graded_middle_of_the_view() {
    let (w, h) = (512u32, 384u32);
    let Some(mut g) = gpu(w, h) else { return };
    let mut tm = TextureManager::new();
    let dl = world_frame(w, h);
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..FrameParams::ui(w, h)
            },
            &dl,
        )],
        &mut tm,
    );
    let screen = g.read_pixels();
    // `sx = left + width / 2 - 128`, `sy = top + height / 2 - 128`.
    let (sx, sy) = (w as i32 / 2 - 128, h as i32 / 2 - 128);
    let thumb = g
        .read_offscreen_thumbnail(sx, sy, params())
        .unwrap()
        .expect("offscreen buffer");
    assert_eq!(thumb.len(), (THUMBNAIL_SIZE * THUMBNAIL_SIZE * 4) as usize);
    // The `screen` shader's bicubic filter samples at texel centres, where
    // its `fract` is unstable: right at a colour edge the two draws may
    // blend toward different sides. Compare every pixel whose screen
    // neighbourhood (two pixels, beyond the filter's reach) is flat; a
    // one-pixel misplacement would still show at the edges' second row.
    let flat = |x: u32, y: u32| {
        let p = px(&screen, w, x, y);
        (x - 2..=x + 2).all(|xx| {
            (y - 2..=y + 2).all(|yy| {
                px(&screen, w, xx, yy)
                    .iter()
                    .zip(p)
                    .all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= 1)
            })
        })
    };
    let (mut worst, mut compared) = (0, 0);
    for r in 0..THUMBNAIL_SIZE {
        // Thumbnail row r (bottom-up) is screen row h - 1 - sy - r.
        let sr = h - 1 - sy as u32 - r;
        for c in 0..THUMBNAIL_SIZE {
            if !flat(sx as u32 + c, sr) {
                continue;
            }
            compared += 1;
            let a = px(&thumb, THUMBNAIL_SIZE, c, r);
            let b = px(&screen, w, sx as u32 + c, sr);
            for k in 0..4 {
                worst = worst.max((i32::from(a[k]) - i32::from(b[k])).abs());
            }
        }
    }
    assert!(compared > 50_000, "{compared}");
    assert!(worst <= 1, "thumbnail differs from the screen by {worst}");
    // The ramps pin the rows and columns to the pixel (one off is 5 * 1.2).
    let ramp = |x: u32, y: u32| {
        let r = h - 1 - sy as u32 - y;
        let (a, b) = (
            px(&thumb, THUMBNAIL_SIZE, x - sx as u32, r),
            px(&screen, w, x, y),
        );
        a.iter()
            .zip(b)
            .all(|(p, q)| (i32::from(*p) - i32::from(q)).abs() <= 2)
    };
    for y in 103..147 {
        assert!(ramp(350, y), "red ramp row {y}");
    }
    for x in 143..187 {
        assert!(ramp(x, 265), "green ramp column {x}");
    }
    // Spot checks: the red rect (screen 140..200 x 80..120) is near the
    // top left, so in the bottom-up rows it is near the end.
    let red = px(
        &thumb,
        THUMBNAIL_SIZE,
        (170 - sx) as u32,
        h - 1 - sy as u32 - 100,
    );
    assert!(red[0] > 200 && red[1] < 60, "{red:?}");
    assert_eq!(red[3], 255);
}

/// Rows come back bottom-up: the view's top is the last row.
#[test]
fn thumbnail_rows_are_bottom_up() {
    let Some(mut g) = gpu(256, 256) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin {
        width: 256,
        height: 256,
    });
    dl.render_rect(0, 0, 256, 128, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.render_rect(0, 128, 256, 128, Rgba(0.0, 0.0, 1.0, 1.0));
    dl.cmds.push(DrawCmd::OffscreenEnd);
    g.render_frame(&[(FrameParams::ui(256, 256), &dl)], &mut tm);
    let t = g.read_offscreen_thumbnail(0, 0, params()).unwrap().unwrap();
    let (bottom, top) = (px(&t, 256, 128, 10), px(&t, 256, 128, 245));
    assert!(top[0] > 200 && top[2] < 50, "{top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 50, "{bottom:?}");
}

/// No offscreen buffer: no thumbnail from it; the back-buffer read
/// (`TakeScreenshot(.., GL_BACK)`) gives the GL rectangle bottom-up.
#[test]
fn without_offscreen_buffer_the_back_buffer_is_read() {
    let Some(mut g) = gpu(64, 32) else { return };
    let mut tm = TextureManager::new();
    assert_eq!(g.read_offscreen_thumbnail(0, 0, params()).unwrap(), None);
    let mut dl = DrawList::default();
    dl.render_rect(0, 0, 64, 16, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.render_rect(0, 16, 32, 16, Rgba(0.0, 1.0, 0.0, 1.0));
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 1.0, 1.0]),
                ..FrameParams::ui(64, 32)
            },
            &dl,
        )],
        &mut tm,
    );
    // GL rect x 16..80 (past the right edge), rows 8..24.
    let p = g.read_main_rect(16, 8, 64, 16).unwrap().unwrap();
    assert_eq!(p.len(), 64 * 16 * 4);
    // Row 0 is GL row 8: screen row 23 (green left half, blue right).
    assert_eq!(px(&p, 64, 0, 0), [0, 255, 0, 255]);
    assert_eq!(px(&p, 64, 20, 0), [0, 0, 255, 255]);
    // Row 15 is GL row 23: screen row 8, red.
    assert_eq!(px(&p, 64, 0, 15), [255, 0, 0, 255]);
    // Outside the target: zero.
    assert_eq!(px(&p, 64, 60, 15), [0, 0, 0, 0]);
}

// Dense, asymmetric colour boundaries make the blur/deformation branches
// observable; flat background alone would let a missing uniform pass.
fn weather_frame(params: ScreenParams) -> DrawList {
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin {
        width: 256,
        height: 256,
    });
    for y in 0..16 {
        for x in 0..16 {
            let r = if (x + y) % 2 == 0 { 0.85 } else { 0.12 };
            let g = 0.15 + y as f32 * 0.035;
            let b = 0.2 + x as f32 * 0.03;
            dl.render_rect(x * 16, y * 16, 16, 16, Rgba(r, g, b, 1.0));
        }
    }
    dl.cmds.push(DrawCmd::OffscreenEnd);
    dl.cmds.push(DrawCmd::ScreenPass {
        x: 0.0,
        y: 0.0,
        w: 256.0,
        h: 256.0,
        tex: [0.0, 0.0, 256.0, 256.0],
        params,
    });
    dl
}

fn assert_weather_thumbnail(mut input: ScreenParams, effect: &str) {
    let Some(mut g) = gpu(256, 256) else { return };
    let mut tm = TextureManager::new();
    input.offscreen_width = 256.0;
    input.offscreen_height = 256.0;
    input.tile_width = 64.0;
    input.timer = 37.0;
    input.timer_wrap = -0.75;
    // The thumbnail must replace the camera's ordinary zoom and offset,
    // while preserving this player's actual effects and frame timer.
    input.zoom = 2.25;
    input.right_click = [37.0, -21.0];
    let reference = weather_frame(input.thumbnail(256, 256, 1.0));
    g.render_frame(&[(FrameParams::ui(256, 256), &reference)], &mut tm);
    let screen = g.read_pixels();
    let thumb = g.read_offscreen_thumbnail(0, 0, input).unwrap().unwrap();
    let plain = ScreenParams {
        night_vision_goggles: false,
        drunk_factor: 0.0,
        blur_factor: 0.0,
        search: Default::default(),
        ..input
    };
    let no_effect = g.read_offscreen_thumbnail(0, 0, plain).unwrap().unwrap();
    let (mut worst, mut changed) = (0, 0);
    for row in 0..256 {
        for col in 0..256 {
            let a = px(&thumb, 256, col, row);
            let b = px(&screen, 256, col, 255 - row);
            for k in 0..4 {
                worst = worst.max((i32::from(a[k]) - i32::from(b[k])).abs());
            }
            let old = px(&no_effect, 256, col, row);
            if a[..3]
                .iter()
                .zip(old)
                .any(|(v, p)| (i32::from(*v) - i32::from(p)).abs() > 4)
            {
                changed += 1;
            }
        }
    }
    assert!(
        worst <= 2,
        "{effect}: thumbnail differs from screen by {worst}"
    );
    assert!(
        changed > 512,
        "{effect}: effect not observable ({changed} pixels)"
    );
}

#[test]
fn thumbnail_preserves_night_vision_and_night_value() {
    assert_weather_thumbnail(
        ScreenParams {
            night_vision_goggles: true,
            night_value: 0.72,
            ..params()
        },
        "night vision",
    );
}

#[test]
fn thumbnail_preserves_drunk_deformation_and_timer_wrap() {
    assert_weather_thumbnail(
        ScreenParams {
            drunk_factor: 0.9,
            ..params()
        },
        "drunk deformation",
    );
}

#[test]
fn thumbnail_preserves_short_sighted_blur() {
    assert_weather_thumbnail(
        ScreenParams {
            blur_factor: 0.95,
            ..params()
        },
        "short-sighted blur",
    );
}

#[test]
fn thumbnail_preserves_search_mode_at_thumbnail_zoom() {
    assert_weather_thumbnail(
        ScreenParams {
            search: pz_render::SearchModeShader {
                enabled: true,
                blur: 0.65,
                radius: 1.0,
                gradient_width: 1.25,
                desat: 0.7,
                darkness: 0.6,
            },
            ..params()
        },
        "search mode",
    );
}
