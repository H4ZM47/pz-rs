//! Replays a fixed weather-effects scenario and compares it with a dump of
//! the original classes running the same scenario (seeded `Rand`, synthetic
//! texture sizes). Runs only when `PZ_WEATHERFX_ORACLE` names the dump; the
//! harness that produces it is kept outside the repository.
//! `PZ_WEATHERFX_SEED` (default 777) and `PZ_WEATHERFX_FRAMES` (default 600)
//! must match the dump's run; `PZ_WEATHERFX_RUST_OUT` writes this side's
//! dump for diffing.
//!
//! Dump format, one record per line, floats as raw IEEE bits in hex:
//! `textures name=WxH ...`, `multiplier ...`, then per checkpoint
//! `fx frame <10 SteppedUpdateFloat values> <7 floats>`,
//! `rect id toRender reqUpdCnt`, `p rect i <particle state>`,
//! `draw frame rect count` and `q <12 floats>` per drawer quad.

use std::collections::HashMap;

use pz_core::random::Rand;
use pz_iso_render::weather_fx::{
    IsoWeatherFx, Layer, ParticleKind, WeatherFxClimate, WeatherFxFrame, WeatherFxSandbox,
    WeatherFxTextures, WeatherFxView,
};
use pz_render::Texture;

fn fb(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

fn db(d: f64) -> String {
    format!("{:x}", d.to_bits())
}

fn tex(name: &str, id: u32, w: u32, h: u32) -> Option<Texture> {
    let mut t = Texture::empty(name);
    t.id = Some(id);
    t.width = w;
    t.height = h;
    t.ready = true;
    Some(t)
}

fn dump(fx: &IsoWeatherFx, frame: i32, out: &mut Vec<String>) {
    let s = [
        fx.wind_precip_intensity,
        fx.wind_intensity,
        fx.wind_angle_intensity,
        fx.precipitation_intensity,
        fx.precipitation_intensity_snow,
        fx.precipitation_intensity_rain,
        fx.cloud_intensity,
        fx.fog_intensity,
        fx.wind_angle_mod,
        fx.indoors_alpha_mod,
    ];
    let mut line = format!("fx {frame}");
    for v in s {
        line += &format!(" {}", fb(v.value()));
    }
    for v in [
        fx.fog_overlay_alpha,
        fx.wind_speed,
        fx.wind_speed_fog,
        fx.wind_angle,
        fx.wind_angle_clouds,
        fx.wind_upd_counter,
        fx.zoom_mod,
    ] {
        line += &format!(" {}", fb(v));
    }
    out.push(line);
    for (r, rect) in fx.rects.iter().enumerate() {
        out.push(format!(
            "rect {r} {} {}",
            rect.particles_to_render, rect.particles_req_upd_cnt
        ));
        for (i, p) in rect.particles.iter().enumerate() {
            let mut s = format!(
                "p {r} {i} {} {} {} {}",
                fb(p.position[0]),
                fb(p.position[1]),
                fb(p.velocity[0]),
                fb(p.velocity[1])
            );
            for b in p.bounds {
                s += &format!(" {b}");
            }
            for v in [
                p.o_width,
                p.o_height,
                p.render_alpha,
                p.alpha,
                p.speed,
                p.alpha_fade_mod.value(),
            ] {
                s += &format!(" {}", fb(v));
            }
            match &p.kind {
                ParticleKind::Snow {
                    life,
                    incarnate_alpha,
                } => s += &format!(" {} {}", fb(*life), fb(*incarnate_alpha)),
                ParticleKind::Rain {
                    life,
                    incarnate_alpha,
                    rp,
                    ..
                } => {
                    s += &format!(" {} {}", fb(*life), fb(*incarnate_alpha));
                    for [x, y] in rp.points {
                        s += &format!(" {} {}", db(x), db(y));
                    }
                    s += &format!(" {} {} {}", fb(p.color[0]), fb(p.color[1]), fb(p.color[2]));
                }
                _ => {}
            }
            out.push(s);
        }
    }
}

fn render(fx: &mut IsoWeatherFx, frame: i32, out: &mut Vec<String>) {
    let v = WeatherFxView::new(3000.25, -1700.75, 1920, 1080, 1.0);
    fx.start_frame();
    for layer in Layer::ALL {
        fx.render_rect(layer, &v);
        // The drawer keeps quads by texture; the dump lists them in the
        // order they were added.
        let n: usize = fx.drawers[layer as usize]
            .by_texture
            .iter()
            .map(Vec::len)
            .sum();
        out.push(format!("draw {frame} {} {n}", layer as usize));
        for q in added_order(fx, layer) {
            let mut s = String::from("q");
            for i in 0..4 {
                s += &format!(" {} {}", fb(q.x[i]), fb(q.y[i]));
            }
            for c in q.rgba {
                s += &format!(" {}", fb(c));
            }
            out.push(s);
        }
    }
}

/// The quads in the order `addParticle` received them: recomputed by
/// replaying the drawer with one bucket.
fn added_order(fx: &IsoWeatherFx, layer: Layer) -> Vec<pz_iso_render::weather_fx::ParticleQuad> {
    let mut one = fx.clone();
    for p in &mut one.rects[layer as usize].particles {
        p.texture = None;
    }
    one.start_frame();
    one.render_rect(
        layer,
        &WeatherFxView::new(3000.25, -1700.75, 1920, 1080, 1.0),
    );
    one.drawers[layer as usize].by_texture.concat()
}

#[test]
fn weather_fx_matches_the_original() {
    let Ok(path) = std::env::var("PZ_WEATHERFX_ORACLE") else {
        return;
    };
    let java = std::fs::read_to_string(&path).expect("oracle dump");
    let mut lines = java.lines();
    let header = lines.next().expect("header");
    let sizes: HashMap<String, (u32, u32)> = header
        .split_whitespace()
        .skip(1)
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap();
            let (w, h) = v.split_once('x').unwrap();
            (k.to_string(), (w.parse().unwrap(), h.parse().unwrap()))
        })
        .collect();
    let mut id = 0;
    let mut t = |name: &str| {
        id += 1;
        let (w, h) = sizes[name];
        tex(name, id, w, h)
    };
    let textures = WeatherFxTextures {
        clouds: std::array::from_fn(|i| t(&format!("clouds_{i}"))),
        fog: std::array::from_fn(|i| t(&format!("fog_{i}"))),
        snow: std::array::from_fn(|i| t(&format!("snow_{}", i + 1))),
        fog_circle: t("fogcircle"),
        fog_white: t("fogwhite"),
    };
    let seed: i64 = std::env::var("PZ_WEATHERFX_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(777);
    let rand = Rand::new(seed);
    let mut fx = IsoWeatherFx::new(&rand, &textures);
    let sandbox = WeatherFxSandbox::default();
    let mut frame = WeatherFxFrame {
        multiplier: 1.0,
        player_indoors: false,
        player_has_mask_to_draw: false,
        zoom: 1.0,
        zoom_enabled: true,
        precipitation_speed: 1.0,
    };
    let set = |fx: &mut IsoWeatherFx, p, w, wp, wa, fog, cloud, snow| {
        let c = WeatherFxClimate {
            precipitation_intensity: p,
            wind_intensity: w,
            wind_precip_intensity: wp,
            wind_angle_intensity: wa,
            fog_intensity: fog,
            cloud_intensity: cloud,
            precipitation_is_snow: snow,
        };
        fx.update_fx(&c, &sandbox);
    };
    let mut out = Vec::new();
    dump(&fx, -1, &mut out);
    let frames: i32 = std::env::var("PZ_WEATHERFX_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    for f in 0..frames {
        match f {
            0 => set(&mut fx, 0.8, 0.6, 0.36, 0.4, 0.5, 0.7, false),
            300 => set(&mut fx, 0.5, 0.9, 0.81, -0.6, 0.2, 0.3, true),
            450 => frame.zoom = 1.5,
            520 => set(&mut fx, 0.0, 0.1, 0.01, 0.0, 0.0, 0.0, true),
            _ => {}
        }
        fx.update(&rand, &frame);
        if f % 50 == 49 || f == frames - 1 {
            dump(&fx, f, &mut out);
            render(&mut fx, f, &mut out);
        }
    }
    if let Ok(p) = std::env::var("PZ_WEATHERFX_RUST_OUT") {
        std::fs::write(p, out.join("\n") + "\n").expect("write");
    }
    let java: Vec<&str> = lines.skip(1).collect();
    let (mut exact, mut near, mut max_rel) = (0usize, 0usize, 0f64);
    let mut bad = Vec::new();
    for (i, (j, r)) in java.iter().zip(&out).enumerate() {
        if *j == r.as_str() {
            exact += 1;
            continue;
        }
        let (jt, rt): (Vec<&str>, Vec<&str>) = (j.split(' ').collect(), r.split(' ').collect());
        let mut ok = jt.len() == rt.len();
        for (a, b) in jt.iter().zip(&rt).filter(|(a, b)| a != b) {
            let v = |s: &str| -> Option<f64> {
                match s.len() {
                    1..=8 => u32::from_str_radix(s, 16)
                        .ok()
                        .map(|b| f64::from(f32::from_bits(b))),
                    9..=16 => u64::from_str_radix(s, 16).ok().map(f64::from_bits),
                    _ => None,
                }
            };
            match (v(a), v(b)) {
                (Some(x), Some(y)) => {
                    let rel = (x - y).abs() / x.abs().max(1.0);
                    max_rel = max_rel.max(rel);
                    if rel > 1e-4 {
                        ok = false;
                    }
                }
                _ => ok = false,
            }
        }
        if ok {
            near += 1;
        } else if bad.len() < 10 {
            bad.push(format!("line {i}\n java {j}\n rust {r}"));
        } else {
            bad.push(String::new());
        }
    }
    eprintln!("weather fx oracle: {exact} lines exact, {near} within rounding (max relative {max_rel:e}), {} differ", bad.len());
    assert_eq!(
        java.len(),
        out.len(),
        "line count; first differences:\n{}",
        bad.iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        bad.is_empty(),
        "{}",
        bad.iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
