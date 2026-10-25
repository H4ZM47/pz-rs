//! Replays a fixed `ImprovedFog` scenario and compares it with a dump of
//! the original classes running it: `update()` over ticks of climate, then
//! `renderFog`'s per-level calls (`startRender`, `startFrame`,
//! `renderRowsBehind` for a scattered list of squares, `endRender`) over a
//! synthetic chunk map, checking the drawer's uniform sources and every
//! rectangle bit for bit. Runs only when `PZ_FOG_ORACLE` names the dump;
//! the harness that produces it is kept outside the repository.
//!
//! Dump format, floats as raw IEEE bits in hex: `climate` (fog, r, g, b,
//! wind angle radians, wind intensity, multiplier, quality, ticks) and a
//! `tick` line (base alpha, scaling x, y, row step) per update; `rooms` and
//! `roofs` as `x0 y0 x1 y1 z` runs; per case a `case` line (player x, y, z,
//! case number, player x, y, z, zoom, quality, in room, in vehicle, offX, offY, width, height, base
//! alpha, scaling x, y, colour), `skip z`, a `drawer` line (its uniform
//! fields), `behind z (x y)*`, `rects n` and `r` lines (the 15 values
//! `addRectangle` writes), then `ImprovedFogDrawer.render`'s calls with GL
//! shimmed as `call` lines (`gl ...`, `u location v0 v1 v2 v3` for each
//! uniform upload, `q` with `addQuadDepth`'s 28 values).

use pz_iso_render::fog::{fog_draw, FogClimate, FogFrame, FogPlayer, FogSquare, ImprovedFog};
use pz_iso_render::frame::FrameSize;
use pz_iso_render::Camera;

fn f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).expect("hex"))
}

fn within(r: &[i32], x: i32, y: i32, z: i32) -> bool {
    r.chunks(5)
        .any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3] && z == r[4])
}

#[test]
fn improved_fog_matches_the_original() {
    let Ok(path) = std::env::var("PZ_FOG_ORACLE") else {
        return;
    };
    let text = std::fs::read_to_string(path).expect("dump");
    let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split(' ').collect()).collect();
    let mut fog = ImprovedFog::default();
    let mut climate = None;
    let (mut rooms, mut roofs) = (Vec::new(), Vec::new());
    let mut i = 0;
    let (mut ticks, mut cases, mut rects_checked, mut quads_checked) = (0, 0, 0, 0);
    while i < lines.len() {
        let l = &lines[i];
        match l[0] {
            "climate" => {
                climate = Some(FogClimate {
                    fog_intensity: f(l[1]),
                    color: [f(l[2]), f(l[3]), f(l[4])],
                    wind_angle_radians: f(l[5]),
                    wind_intensity: f(l[6]),
                    multiplier: f(l[7]),
                    fog_quality: l[8].parse().unwrap(),
                });
            }
            "tick" => {
                fog.update(climate.as_ref().unwrap());
                let got = [
                    fog.base_alpha.to_bits(),
                    fog.scaling_x.to_bits(),
                    fog.scaling_y.to_bits(),
                ];
                let want = [f(l[1]).to_bits(), f(l[2]).to_bits(), f(l[3]).to_bits()];
                assert_eq!(got, want, "tick {ticks}");
                assert_eq!(fog.render_every_x_row, l[4].parse::<i32>().unwrap());
                ticks += 1;
            }
            "rooms" => rooms = l[1..].iter().map(|v| v.parse().unwrap()).collect(),
            "roofs" => roofs = l[1..].iter().map(|v| v.parse().unwrap()).collect(),
            "case" => {
                // After the case number.
                let l = &l[1..];
                let (px, py, pz, zoom) = (f(l[1]), f(l[2]), f(l[3]), f(l[4]));
                let quality: i32 = l[5].parse().unwrap();
                let (in_room_flag, in_vehicle) = (l[6] == "1", l[7] == "1");
                let (off_x, off_y): (i32, i32) = (l[8].parse().unwrap(), l[9].parse().unwrap());
                let (w, h): (u32, u32) = (l[10].parse().unwrap(), l[11].parse().unwrap());
                let mut c = climate.unwrap();
                c.fog_quality = quality;
                fog.update(&c);
                assert_eq!(
                    [
                        fog.base_alpha,
                        fog.scaling_x,
                        fog.scaling_y,
                        fog.color[0],
                        fog.color[1],
                        fog.color[2]
                    ]
                    .map(f32::to_bits),
                    [12, 13, 14, 15, 16, 17].map(|k| f(l[k]).to_bits()),
                    "case {cases} state"
                );
                let (sx, sy, sz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
                let player = FogPlayer {
                    x: px,
                    y: py,
                    z: pz,
                    in_room: in_room_flag && within(&rooms, sx, sy, sz),
                    in_vehicle,
                };
                let frame = FogFrame {
                    fog: fog.clone(),
                    fog_quality: quality,
                    player,
                    ..Default::default()
                };
                let cam = Camera {
                    off_x,
                    off_y,
                    zoom,
                    ..Camera::centred(px, py, pz, w as i32, h as i32, zoom)
                };
                let (pwx, pwy) = ((px / 8.0).floor() as i32, (py / 8.0).floor() as i32);
                let (rooms, roofs) = (rooms.clone(), roofs.clone());
                let lookup = move |x: i32, y: i32, z: i32| {
                    let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
                    if (wx - pwx).abs() > 6 || (wy - pwy).abs() > 6 || !(0..=1).contains(&z) {
                        return None;
                    }
                    let (room, roof) = (within(&rooms, x, y, z), within(&roofs, x, y, z));
                    if z == 1 && !room && !roof {
                        return None;
                    }
                    Some(FogSquare {
                        exterior: !room && !roof,
                        in_room: room,
                    })
                };
                // The case's lines up to the next case.
                let mut j = i + 1;
                let mut behind: Vec<(i32, Vec<(i32, i32)>)> = Vec::new();
                let mut drawer = None;
                let mut want_rects = Vec::new();
                let mut calls: Vec<&[&str]> = Vec::new();
                while j < lines.len() && lines[j][0] != "case" {
                    let m = &lines[j];
                    match m[0] {
                        "behind" => {
                            let z = m[1].parse().unwrap();
                            let v: Vec<i32> = m[2..].iter().map(|v| v.parse().unwrap()).collect();
                            behind.push((z, v.chunks(2).map(|p| (p[0], p[1])).collect()));
                        }
                        "drawer" => {
                            drawer =
                                Some(m[1..].iter().map(|v| f(v).to_bits()).collect::<Vec<u32>>())
                        }
                        "r" => want_rects.push(m[1..].to_vec()),
                        "call" => calls.push(&m[1..]),
                        _ => {}
                    }
                    j += 1;
                }
                let behind_of = |z: i32| {
                    behind
                        .iter()
                        .find(|(bz, _)| *bz == z)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                let got = fog_draw(
                    &frame,
                    &cam,
                    FrameSize {
                        width: w,
                        height: h,
                    },
                    &behind_of,
                    &lookup,
                );
                match (got, drawer) {
                    (None, None) => {}
                    (Some(d), Some(want)) => {
                        let got: Vec<u32> = [d.screen[0], d.screen[1], d.screen[2]]
                            .into_iter()
                            .chain(d.texture_info)
                            .chain(d.world_offset)
                            .chain(d.scaling)
                            .chain(d.color_info)
                            .chain(d.param_info)
                            .chain(d.camera)
                            .chain([d.alpha])
                            .map(f32::to_bits)
                            .collect();
                        assert_eq!(got, want, "case {cases} drawer");
                        assert_eq!(d.rects.len(), want_rects.len(), "case {cases} rectangles");
                        for (k, (r, w)) in d.rects.iter().zip(&want_rects).enumerate() {
                            let got: Vec<u32> = r
                                .pos
                                .into_iter()
                                .chain(r.uv)
                                .chain(r.depth)
                                .chain([r.offset, r.layer_alpha])
                                .map(f32::to_bits)
                                .collect();
                            let want: Vec<u32> = w[..14].iter().map(|v| f(v).to_bits()).collect();
                            assert_eq!(got, want, "case {cases} rect {k}");
                            assert_eq!(r.z_layer, w[14].parse::<i32>().unwrap());
                            rects_checked += 1;
                        }
                        // ImprovedFogDrawer.render: the GL state, then per
                        // quad the uniforms in effect (locations 1..8:
                        // screen, texture, rectangle, scaling, colour, world
                        // offset, param, camera) and its vertices.
                        for st in [
                            ["gl", "enable", "3042"].as_slice(),
                            &["gl", "blend", "770", "771"],
                            &["gl", "depthmask", "false"],
                            &["gl", "depthfunc", "513"],
                            &["depthtest", "true"],
                        ] {
                            assert!(calls.contains(&st), "case {cases}: no {st:?}");
                        }
                        let mut uni = [[0u32; 4]; 9];
                        let mut q = 0;
                        for c in &calls {
                            match c[0] {
                                "u" => {
                                    uni[c[1].parse::<usize>().unwrap()] =
                                        [2, 3, 4, 5].map(|k| f(c[k]).to_bits())
                                }
                                "q" => {
                                    let u = d.uniforms(q);
                                    let got = [
                                        u.screen_info,
                                        u.texture_info,
                                        u.rectangle_info,
                                        u.scaling_info,
                                        u.color_info,
                                        u.world_offset,
                                        u.param_info,
                                        u.camera_info,
                                    ]
                                    .map(|v| v.map(f32::to_bits));
                                    assert_eq!(
                                        got.as_slice(),
                                        &uni[1..],
                                        "case {cases} quad {q} uniforms"
                                    );
                                    let r = &d.rects[q];
                                    let [sx, sy, ex, ey] = r.pos;
                                    let [u0, v0, u1, v1] = r.uv;
                                    let dd = r.depth;
                                    let vert = [
                                        sx, sy, 0.0, u0, v0, dd[0], ex, sy, 0.0, u1, v0, dd[1], ex,
                                        ey, 0.0, u1, v1, dd[2], sx, ey, 0.0, u0, v1, dd[3], 1.0,
                                        1.0, 1.0, d.alpha,
                                    ];
                                    let want: Vec<u32> =
                                        c[1..].iter().map(|v| f(v).to_bits()).collect();
                                    assert_eq!(
                                        vert.map(f32::to_bits).as_slice(),
                                        want.as_slice(),
                                        "case {cases} quad {q}"
                                    );
                                    q += 1;
                                }
                                _ => {}
                            }
                        }
                        assert_eq!(q, d.rects.len());
                        quads_checked += q;
                    }
                    (g, w) => panic!("case {cases}: drew {} vs {}", g.is_some(), w.is_some()),
                }
                cases += 1;
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    assert!(ticks > 0 && cases > 0);
    eprintln!("fog oracle: {ticks} ticks, {cases} cases, {rects_checked} rectangles and {quads_checked} drawn quads match");
}
