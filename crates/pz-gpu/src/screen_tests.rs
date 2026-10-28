//! The `screen` shader's branches on the GPU against the original GLSL's
//! formulas evaluated on the CPU (f32, the offscreen buffer sampled
//! nearest with clamp to edge as `Core.screenFilter` binds it).

use super::*;
use pz_render::{Rgba, ScreenParams, SearchModeShader};

const W: u32 = 64;
const H: u32 = 40;

/// The offscreen buffer's contents as drawn, row 0 at the top: smooth
/// ramps, so a texel picked one off at a sampling boundary differs little.
fn content(x: u32, y: u32) -> [u8; 3] {
    [
        (3 * x + 20) as u8,
        (5 * y + 30) as u8,
        (200 - 2 * x - y) as u8,
    ]
}

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn mix3(a: V3, b: V3, t: f32) -> V3 {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
fn clamp3(a: V3) -> V3 {
    a.map(|v| v.clamp(0.0, 1.0))
}
fn mixf(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
fn desaturate(c: V3, amount: f32) -> V3 {
    let gray = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    mix3(c, [gray; 3], amount)
}
fn contrast(c: V3, amount: f32) -> V3 {
    c.map(|v| (v - 0.4) * amount + 0.4)
}
fn fract(v: f32) -> f32 {
    v - v.floor()
}

/// The CPU side of the shader for one fragment.
struct Ref {
    p: ScreenParams,
    /// Offscreen texture size and GL window height.
    tex: [f32; 2],
    window_h: f32,
}

impl Ref {
    /// Nearest texel at `uv` (v up from the bottom row).
    fn sample(&self, uv: [f32; 2]) -> V3 {
        let tx = ((uv[0] * self.tex[0]).floor() as i64).clamp(0, self.tex[0] as i64 - 1) as u32;
        let row = ((uv[1] * self.tex[1]).floor() as i64).clamp(0, self.tex[1] as i64 - 1) as u32;
        // GL row 0 is the bottom of what was drawn.
        let c = content(tx, self.tex[1] as u32 - 1 - row);
        c.map(|v| f32::from(v) / 255.0)
    }

    fn cubic(v: f32) -> [f32; 4] {
        let n = [1.0 - v, 2.0 - v, 3.0 - v, 4.0 - v];
        let s = n.map(|n| n * n * n);
        let x = s[0];
        let y = s[1] - 4.0 * s[0];
        let z = s[2] - 4.0 * s[1] + 6.0 * s[0];
        let w = 6.0 - x - y - z;
        [x, y, z, w].map(|c| c * (1.0 / 6.0))
    }

    fn bicubic(&self, uv: [f32; 2]) -> V3 {
        let [tw, th] = self.tex;
        let tc = [uv[0] * tw - 0.5, uv[1] * th - 0.5];
        let f = [fract(tc[0]), fract(tc[1])];
        let tc = [tc[0] - f[0], tc[1] - f[1]];
        let xc = Self::cubic(f[0]);
        let yc = Self::cubic(f[1]);
        let c = [tc[0] - 0.5, tc[0] + 1.5, tc[1] - 0.5, tc[1] + 1.5];
        let s = [xc[0] + xc[1], xc[2] + xc[3], yc[0] + yc[1], yc[2] + yc[3]];
        let o = [
            (c[0] + xc[1] / s[0]) / tw,
            (c[1] + xc[3] / s[1]) / tw,
            (c[2] + yc[1] / s[2]) / th,
            (c[3] + yc[3] / s[3]) / th,
        ];
        let s0 = self.sample([o[0], o[2]]);
        let s1 = self.sample([o[1], o[2]]);
        let s2 = self.sample([o[0], o[3]]);
        let s3 = self.sample([o[1], o[3]]);
        let sx = s[0] / (s[0] + s[1]);
        let sy = s[2] / (s[2] + s[3]);
        mix3(mix3(s3, s2, sx), mix3(s1, s0, sx), sy)
    }

    fn rnm(&self, tc: [f32; 2]) -> [f32; 4] {
        let t = self.p.timer;
        // 43758.5453 in f32.
        let n = ((tc[0] + t) * 12.9898 + (tc[1] + t) * 78.233).sin() * 43_758.547;
        [
            fract(n) * 2.0 - 1.0,
            fract(n * 1.2154) * 2.0 - 1.0,
            fract(n * 1.3453) * 2.0 - 1.0,
            fract(n * 1.3647) * 2.0 - 1.0,
        ]
    }

    fn pnoise3d(&self, p: V3) -> f32 {
        const U: f32 = 1.0 / 256.0;
        let pi = p.map(|v| U * v.floor() + 0.5 / 256.0);
        let pf = p.map(fract);
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let grad = |perm: f32, z: f32, off: V3| {
            let r = self.rnm([perm, z]);
            let g = [r[0] * 4.0 - 1.0, r[1] * 4.0 - 1.0, r[2] * 4.0 - 1.0];
            g[0] * (pf[0] - off[0]) + g[1] * (pf[1] - off[1]) + g[2] * (pf[2] - off[2])
        };
        let corner = |dx: f32, dy: f32| {
            let perm = self.rnm([pi[0] + dx * U, pi[1] + dy * U])[3];
            (
                grad(perm, pi[2], [dx, dy, 0.0]),
                grad(perm, pi[2] + U, [dx, dy, 1.0]),
            )
        };
        let (n000, n001) = corner(0.0, 0.0);
        let (n010, n011) = corner(0.0, 1.0);
        let (n100, n101) = corner(1.0, 0.0);
        let (n110, n111) = corner(1.0, 1.0);
        let fx = fade(pf[0]);
        let nx = [
            mixf(n000, n100, fx),
            mixf(n001, n101, fx),
            mixf(n010, n110, fx),
            mixf(n011, n111, fx),
        ];
        let fy = fade(pf[1]);
        let nxy = [mixf(nx[0], nx[2], fy), mixf(nx[1], nx[3], fy)];
        mixf(nxy[0], nxy[1], fade(pf[2]))
    }

    /// `main()`'s grain value at `uv`.
    fn noise(&self, uv: [f32; 2]) -> f32 {
        let [w, h] = [
            self.u().bgl_rendered_texture_width,
            self.u().bgl_rendered_texture_height,
        ];
        let aspect = w / h;
        let a = self.p.timer + 1.425;
        let (tx, ty) = (uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0);
        let rx = ((tx * aspect * a.cos()) - (ty * a.sin())) / aspect * 0.5 + 0.5;
        let ry = ((ty * a.cos()) + (tx * aspect * a.sin())) * 0.5 + 0.5;
        self.pnoise3d([rx * w, ry * h, 0.0])
    }

    fn u(&self) -> pz_render::ScreenUniforms {
        self.p.uniforms(self.tex)
    }

    fn centre(&self) -> [f32; 2] {
        let si = self.u().screen_info;
        [0.5 - si[2] / si[0], 0.5 + si[3] / si[1]]
    }

    fn blur(&self, col: V3, alpha: f32, uv: [f32; 2]) -> V3 {
        let si = self.u().screen_info;
        let rad = [12.0 / si[0], 12.0 / si[1]];
        let mut c = self.sample(uv);
        // blur_pi (6.28318530718) is TAU in f32.
        let pi2 = std::f32::consts::TAU;
        let mut d = 0.0f32;
        let mut dirs = 0;
        while d < pi2 {
            let mut i = 1.0f32 / 3.0;
            while i <= 1.01 {
                c = add(
                    c,
                    self.sample([uv[0] + d.cos() * rad[0] * i, uv[1] + d.sin() * rad[1] * i]),
                );
                i += 1.0 / 3.0;
            }
            d += pi2 / 16.0;
            dirs += 1;
        }
        assert_eq!(dirs, 16);
        let c = clamp3(scale(c, 1.0 / (3.0 * (16.0 + 1.0))));
        add(scale(col, 1.0 - alpha), scale(c, alpha))
    }

    fn view_coord(&self, frag: [f32; 2]) -> [f32; 2] {
        let u = self.u();
        [
            (frag[0] - u.search_mode[2]) * u.param_info[0] / u.screen_info[0],
            (frag[1] - u.search_mode[3]) * u.param_info[0] / u.screen_info[1],
        ]
    }

    fn screen_world(&self, pixel: V3, frag: [f32; 2], uv: [f32; 2]) -> V3 {
        let u = self.u();
        let mut d;
        if u.var_info[0] == 0.0 {
            d = desaturate(pixel, u.desaturation_val);
        } else {
            let zoom = u.param_info[0];
            let coord = [
                (frag[0] - u.search_mode[2]) * zoom / u.screen_info[0],
                (frag[1] - u.search_mode[3] + 56.0 / zoom) * zoom / u.screen_info[1],
            ];
            let dist = distance(coord, self.centre()) * u.screen_info[0];
            let r = u.search_mode[1] * u.param_info[1];
            let alpha = smoothstep((r - u.param_info[2]).max(0.0), r + u.param_info[2], dist);
            d = self.blur(pixel, alpha * u.search_mode[0], uv);
            d = desaturate(d, u.desaturation_val.max(alpha * u.param_info[3]));
            d = scale(d, 1.0 - u.var_info[1] * alpha);
        }
        let d = clamp3(d);
        let intensity = 0.299 * d[0] + 0.587 * d[1] + 0.114 * d[2];
        let inv = (1.0 - intensity) * (1.0 - intensity);
        let grain = self.noise(uv) * 0.0003 * (inv * 5.0);
        contrast(desaturate(d, 0.1), 1.2).map(|v| v + grain)
    }

    fn night_vision(&self, pixel: V3, uv: [f32; 2]) -> V3 {
        let n = self.u().night_value;
        let mut d = pixel;
        let intensity = 0.299 * d[0] + 0.587 * d[1] + 0.114 * d[2];
        d = mix3(d, [intensity; 3], (n * 0.8) * (1.0 - intensity));
        d = d.map(|v| (1.0 - n) * v + (n * v * v) * 1.5);
        let shift = n * 0.25 * (1.0 - intensity);
        d = [d[0] - shift, d[1] - shift, d[2] + shift];
        let mut col = contrast(desaturate(d, 0.1), 1.2);
        col = [col[0] + 0.45, col[1] + 0.45, col[2]];
        let inv = (1.0 - intensity) * (1.0 - intensity);
        let grain = self.noise(uv) * 0.015 * (inv * 5.0);
        col = col.map(|v| v * 1.9 + grain);
        let i2 =
            (0.5 * (0.30 * col[0] + 0.59 * col[1] + 0.11 * col[2] - 0.5) + 0.5).clamp(0.0, 1.0);
        let green = (i2 / 0.59).clamp(0.0, 1.0) * 1.3;
        [0.0, col[1] * green, 0.0]
    }

    fn drunk(&self, pixel: V3, frag: [f32; 2], uv: [f32; 2]) -> V3 {
        let u = self.u();
        let mut col = clamp3(pixel);
        let coord = self.view_coord(frag);
        let centre = self.centre();
        let dist = distance(coord, centre) * u.screen_info[0];
        let circle = smoothstep(0.0, 20.0 + 640.0, dist);
        let dx = ((coord[0] - (0.5 - centre[0])) - centre[0]).abs();
        let dy = ((coord[1] - (0.5 - centre[1])) - centre[1]).abs();
        let da = 1.0 - (dx / 0.5).max(dy / 0.5);
        let dab = da * circle;
        let mag = 0.005 * u.drunk_factor;
        let (t, w, z) = (u.timer, u.timer_wrap, u.param_info[0]);
        let c2 = [
            uv[0] + mag * (t / (w * 7.3)).cos() / z,
            uv[1] + mag * (t / (w * 13.3)).sin() / z,
        ];
        let c3 = [
            uv[0] + mag * (t / (w * 13.3)).cos() / z,
            uv[1] + mag * (t / (w * 7.3)).sin() / z,
        ];
        let p2 = self.bicubic(c2);
        let p3 = self.bicubic(c3);
        let pb = self.blur(col, u.drunk_factor, uv);
        col = mix3(col, p2, dab);
        col = mix3(col, p3, dab);
        col = mix3(col, pb, 1.0 - da);
        clamp3(col)
    }

    fn glasses(&self, pixel: V3, frag: [f32; 2], uv: [f32; 2]) -> V3 {
        let u = self.u();
        let col = clamp3(pixel);
        let coord = self.view_coord(frag);
        let zoom = u.param_info[0];
        let dist = distance(coord, self.centre()) * u.screen_info[0];
        let rad = 3.0f32;
        let rpw = (rad * 64.0) / 2.0;
        let gpw = (rpw * 3.0) * zoom;
        let gw = gpw * (rad / 32.0 / zoom);
        let gwa = (gw * 64.0) / 2.0;
        let alpha = smoothstep((gw - gwa).max(0.0), gw + gwa, dist);
        self.blur(col, u.blur_factor * alpha, uv)
    }

    /// `main()` for the screen pixel `(x, y)` (row 0 at the top) of a
    /// pass over the whole `W` x `H` target.
    /// `nudge` moves the texture coordinate by that many texels: at a
    /// pixel centre the bicubic lookup sits exactly on a texel centre,
    /// where its nearest samples jump between neighbours with the last
    /// bit of the interpolated coordinate.
    fn pixel(&self, x: u32, y: u32, nudge: [f32; 2]) -> V3 {
        let uv = [
            (x as f32 + 0.5 + nudge[0]) / W as f32,
            1.0 - (y as f32 + 0.5 + nudge[1]) / H as f32,
        ];
        let frag = [x as f32 + 0.5, self.window_h - (y as f32 + 0.5)];
        let u = self.u();
        let pixel = self.bicubic(uv);
        let mut col = pixel;
        if u.drunk_factor > 0.0 {
            col = self.drunk(pixel, frag, uv);
        }
        if u.blur_factor > 0.0 {
            col = self.glasses(col, frag, uv);
        }
        if u.night_vision_goggles < 0.5 {
            self.screen_world(col, frag, uv)
        } else {
            self.night_vision(col, uv)
        }
    }
}

/// Renders the ramp through the screen pass with `p` and compares every
/// pixel with the CPU formulas: at most `tol` apart per channel, except
/// for at most `outliers` pixels (texels picked one off where a sample
/// lands on a texel edge).
fn check(p: ScreenParams, tol: i32, outliers: usize) -> Vec<u8> {
    let Some(mut g) = Gpu::new_headless(W, H)
        .map_err(|e| eprintln!("skipping: {e}"))
        .ok()
    else {
        return Vec::new();
    };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin {
        width: W,
        height: H,
    });
    for y in 0..H {
        for x in 0..W {
            let c = content(x, y).map(|v| (f32::from(v) + 0.5) / 255.0);
            dl.render_rect(x as i32, y as i32, 1, 1, Rgba(c[0], c[1], c[2], 1.0));
        }
    }
    dl.cmds.push(DrawCmd::OffscreenEnd);
    dl.cmds.push(DrawCmd::ScreenPass {
        x: 0.0,
        y: 0.0,
        w: W as f32,
        h: H as f32,
        tex: [0.0, 0.0, W as f32, H as f32],
        params: p,
    });
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
    let got = g.read_pixels();
    let mut p = p;
    if p.offscreen_width == 0.0 {
        (p.offscreen_width, p.offscreen_height) = (W as f32, H as f32);
    }
    let r = Ref {
        p,
        tex: [W as f32, H as f32],
        window_h: H as f32,
    };
    let mut bad = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let have = [got[i], got[i + 1], got[i + 2]].map(i32::from);
            let e = 1e-3;
            let nudges = [
                [0.0, 0.0],
                [-e, 0.0],
                [e, 0.0],
                [0.0, -e],
                [0.0, e],
                [-e, -e],
                [-e, e],
                [e, -e],
                [e, e],
            ];
            let wants = nudges.map(|n| {
                r.pixel(x, y, n)
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32)
            });
            if !wants
                .iter()
                .any(|want| (0..3).all(|c| (have[c] - want[c]).abs() <= tol))
            {
                bad.push((x, y, have, wants[0]));
            }
        }
    }
    assert!(
        bad.len() <= outliers,
        "{} pixels off, first {:?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
    got
}

fn spread(p: &[u8], channel: usize) -> i32 {
    let v: Vec<i32> = p.chunks(4).map(|c| i32::from(c[channel])).collect();
    v.iter().max().unwrap() - v.iter().min().unwrap()
}

#[test]
fn world_grade_desaturates() {
    check(
        ScreenParams {
            desaturation: 0.35,
            timer: 17.0,
            ..Default::default()
        },
        2,
        0,
    );
}

/// Search mode: blurred, desaturated and darkened outside the circle
/// round the (right-click moved) view centre, untouched inside it.
#[test]
fn search_mode_circle() {
    let p = ScreenParams {
        desaturation: 0.1,
        search: SearchModeShader {
            enabled: true,
            blur: 1.0,
            radius: 0.5,
            gradient_width: 0.25,
            desat: 0.85,
            darkness: 0.4,
        },
        offscreen_width: W as f32,
        offscreen_height: H as f32,
        right_click: [4.0, 56.0],
        zoom: 1.0,
        tile_width: 32.0,
        ..Default::default()
    };
    let got = check(p, 2, 0);
    if got.is_empty() {
        return;
    }
    // Inside (near x 36, gl y 20) the colour stays; far out it is dark.
    let at = |x: u32, y: u32| got[((y * W + x) * 4) as usize];
    assert!(at(36, 20) > at(2, 2) + 20, "{} {}", at(36, 20), at(2, 2));
}

/// The search circle follows `zoom` and the offscreen left/top of a
/// player.
#[test]
fn search_mode_offscreen_origin_and_zoom() {
    let p = ScreenParams {
        search: SearchModeShader {
            enabled: true,
            blur: 0.5,
            radius: 0.25,
            gradient_width: 0.5,
            desat: 0.5,
            darkness: 0.8,
        },
        offscreen_left: 16.0,
        offscreen_top: 4.0,
        offscreen_width: 48.0,
        offscreen_height: 30.0,
        right_click: [-6.0, 70.0],
        zoom: 2.0,
        tile_width: 64.0,
        ..Default::default()
    };
    check(p, 2, 0);
}

#[test]
fn drunk_deform_jiggles_and_blurs_the_edges() {
    let p = ScreenParams {
        drunk_factor: 0.8,
        timer: 123.0,
        timer_wrap: 0.75,
        zoom: 0.5,
        offscreen_width: W as f32,
        offscreen_height: H as f32,
        right_click: [3.0, -2.0],
        ..Default::default()
    };
    check(p, 2, 0);
}

#[test]
fn short_sighted_blur_away_from_the_centre() {
    let p = ScreenParams {
        blur_factor: 0.9,
        zoom: 16.0,
        offscreen_width: W as f32,
        offscreen_height: H as f32,
        ..Default::default()
    };
    let got = check(p, 2, 0);
    if got.is_empty() {
        return;
    }
    // The blur strength grows away from the centre.
    let base = check(
        ScreenParams {
            offscreen_width: W as f32,
            offscreen_height: H as f32,
            zoom: 16.0,
            ..Default::default()
        },
        2,
        0,
    );
    let diff = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        (0..3)
            .map(|c| (i32::from(got[i + c]) - i32::from(base[i + c])).abs())
            .sum::<i32>()
    };
    assert!(diff(1, 1) > diff(32, 20), "{} {}", diff(1, 1), diff(32, 20));
}

/// Night-vision goggles: green only. A white buffer has no grain term.
#[test]
fn night_vision_goggles_turn_green() {
    let Some(mut g) = Gpu::new_headless(8, 8)
        .map_err(|e| eprintln!("skipping: {e}"))
        .ok()
    else {
        return;
    };
    let mut tm = TextureManager::new();
    for night in [0.0f32, 0.4, 1.0] {
        let p = ScreenParams {
            night_vision_goggles: true,
            night_value: night,
            offscreen_width: 8.0,
            offscreen_height: 8.0,
            ..Default::default()
        };
        let mut dl = DrawList::default();
        dl.cmds.push(DrawCmd::OffscreenBegin {
            width: 8,
            height: 8,
        });
        dl.render_rect(0, 0, 8, 8, Rgba(1.0, 1.0, 1.0, 1.0));
        dl.cmds.push(DrawCmd::OffscreenEnd);
        dl.cmds.push(DrawCmd::ScreenPass {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 8.0,
            tex: [0.0, 0.0, 8.0, 8.0],
            params: p,
        });
        g.render_frame(
            &[(
                FrameParams {
                    clear: Some([0.0, 0.0, 0.0, 1.0]),
                    ..FrameParams::ui(8, 8)
                },
                &dl,
            )],
            &mut tm,
        );
        let got = g.read_pixels();
        let r = Ref {
            p,
            tex: [8.0, 8.0],
            window_h: 8.0,
        };
        let want = r
            .night_vision([1.0; 3], [0.5, 0.5])
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32);
        let have = [got[0], got[1], got[2]].map(i32::from);
        assert!(
            (0..3).all(|c| (have[c] - want[c]).abs() <= 1),
            "night {night}: {have:?} {want:?}"
        );
        assert_eq!((have[0], have[2]), (0, 0));
    }
}

/// Night vision over the ramp, grain included. The goggles' grain is 50
/// times the world's (`0.075 * noise * (1 - intensity)^2`, times the
/// green gain), and the noise hash (`fract(sin(x) * 43758.5453)`) turns
/// the GPU's sin rounding into a different value now and then: up to 5%
/// of the pixels may differ.
#[test]
fn night_vision_over_a_ramp() {
    let p = ScreenParams {
        night_vision_goggles: true,
        night_value: 0.5,
        timer: 9.0,
        ..Default::default()
    };
    let got = check(p, 2, (W * H / 20) as usize);
    if !got.is_empty() {
        assert_eq!((spread(&got, 0), spread(&got, 2)), (0, 0), "green only");
    }
}

/// Drunk, glasses blur and search mode together, in the original's order.
#[test]
fn all_branches_combined() {
    let p = ScreenParams {
        desaturation: 0.2,
        drunk_factor: 0.5,
        blur_factor: 0.7,
        search: SearchModeShader {
            enabled: true,
            blur: 0.8,
            radius: 0.4,
            gradient_width: 0.3,
            desat: 0.6,
            darkness: 0.5,
        },
        offscreen_width: W as f32,
        offscreen_height: H as f32,
        right_click: [0.0, 40.0],
        zoom: 1.5,
        tile_width: 32.0,
        timer: 77.0,
        timer_wrap: 1.0,
        ..Default::default()
    };
    check(p, 2, 0);
}
